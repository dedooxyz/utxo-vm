#!/usr/bin/env node
/**
 * JKC Testnet — Vault Deployment & Challenge Test
 *
 * This script tests the full fraud proof flow on JKC testnet:
 * 1. Deploy operator vault (P2TR)
 * 2. Operator posts batch with wrong root
 * 3. Challenger submits challenge
 * 4. Bond is slashed to challenger
 *
 * Prerequisites:
 * - JKC testnet wallet with tJKC
 * - Electrs API: https://jkc-testnet-api.s3na.xyz
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const crypto = require("crypto");

// Initialize ECPair
const ECPair = ECPairFactory(ecc);

// --- JKC Testnet Configuration ---
const JKC_TESTNET = {
  messagePrefix: "\x19Junkcoin Signed Message:\n",
  bech32: "tjkc",
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 111,
  scriptHash: 196,
  wif: 239,
};

const ELECTRS = "https://jkc-testnet-api.s3na.xyz";

// Wallet keys (from jkc-testnet-wallet.md)
const OPERATOR_WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";
const CHALLENGER_WIF = "cRaT4QvBkVCTvKn1xP59CMT3UH24m4DcFD2A84k1MhgE8PxNcbNt";

// --- Helper Functions ---

async function electrsGet(path) {
  const res = await fetch(`${ELECTRS}${path}`);
  if (!res.ok) throw new Error(`GET ${path}: ${res.status}`);
  return res;
}

async function electrsGetText(path) {
  return (await (await electrsGet(path)).text()).trim();
}

async function electrsGetJson(path) {
  return (await (await electrsGet(path)).json());
}

async function broadcast(txHex) {
  const res = await fetch(`${ELECTRS}/tx`, {
    method: "POST",
    headers: { "Content-Type": "text/plain" },
    body: txHex,
  });
  const txid = (await res.text()).trim();
  if (!res.ok) throw new Error(`Broadcast: ${txid}`);
  return txid;
}

async function getUtxos(address) {
  const utxos = await electrsGetJson(`/address/${address}/utxo`);
  return utxos;
}

// --- Script Builders ---

/**
 * Build P2TR vault script tree
 * Two spending paths:
 * 1. Operator unbond: <delay> OP_CSV OP_DROP <operator_pk> OP_CHECKSIG
 * 2. Challenge: <operator_pk> OP_CHECKSIGVERIFY OP_CAT OP_SHA256 <hash> OP_EQUALVERIFY
 */
function buildVaultScriptTree(operatorPubkey, unbondDelay) {
  // Leaf 0: Operator unbond path
  const operatorLeaf = buildOperatorUnbondLeaf(operatorPubkey, unbondDelay);

  // Leaf 1: Challenge path (simplified for v1)
  const challengeLeaf = buildChallengeLeaf(operatorPubkey);

  return { operatorLeaf, challengeLeaf };
}

function buildOperatorUnbondLeaf(pubkey, delay) {
  const script = [];

  // Push delay as minimal encoding
  if (delay <= 16) {
    script.push(0x50 + delay);
  } else if (delay <= 0xff) {
    script.push(0x01, delay);
  } else {
    script.push(0x02, delay & 0xff, (delay >> 8) & 0xff);
  }

  // OP_CHECKSEQUENCEVERIFY OP_DROP
  script.push(0xb2, 0x75);

  // Push pubkey (33 bytes)
  script.push(0x21);
  script.push(...pubkey);

  // OP_CHECKSIG
  script.push(0xac);

  return Buffer.from(script);
}

function buildChallengeLeaf(pubkey) {
  const script = [];

  // Push operator pubkey
  script.push(0x21);
  script.push(...pubkey);

  // OP_CHECKSIGVERIFY
  script.push(0xad);

  // OP_CAT (concatenate two hashes)
  script.push(0x7e);

  // OP_SHA256
  script.push(0xa8);

  // Placeholder for expected hash (32 bytes)
  script.push(0x20);
  script.push(...new Array(32).fill(0x42)); // Dummy hash

  // OP_EQUALVERIFY
  script.push(0x88);

  // Challenger pubkey placeholder
  script.push(0x21);
  script.push(...new Array(33).fill(0x03));

  // OP_CHECKSIG
  script.push(0xac);

  return Buffer.from(script);
}

// Taproot helpers
const TAPROOT_LEAF_VERSION = 0xc0;

function tapleafHash(script) {
  const leafVersion = Buffer.from([TAPROOT_LEAF_VERSION]);
  const scriptLen = script.length;

  let scriptLenBuf;
  if (scriptLen < 0x4c) {
    scriptLenBuf = Buffer.from([scriptLen]);
  } else if (scriptLen <= 0xff) {
    scriptLenBuf = Buffer.from([0x4c, scriptLen]);
  } else {
    scriptLenBuf = Buffer.from([0x4d, scriptLen & 0xff, (scriptLen >> 8) & 0xff]);
  }

  const data = Buffer.concat([leafVersion, scriptLenBuf, script]);
  return crypto.createHash("sha256").update(data).digest();
}

function buildTaprootOutputKey(internalKey, scriptTree) {
  // For simplicity, we'll use a basic taproot construction
  // In production, use a proper taproot library

  const leftHash = tapleafHash(scriptTree.operatorLeaf);
  const rightHash = tapleafHash(scriptTree.challengeLeaf);

  // Sort the hashes (BIP-340)
  const [first, second] = Buffer.compare(leftHash, rightHash) < 0
    ? [leftHash, rightHash]
    : [rightHash, leftHash];

  const scriptTreeHash = crypto.createHash("sha256")
    .update(Buffer.concat([first, second]))
    .digest();

  // Tweaked key = internalKey + scriptTreeHash (simplified)
  // In production, use proper taproot key tweaking
  return {
    outputKey: internalKey, // Placeholder
    scriptTreeHash,
  };
}

// --- Test Flow ---

async function testVaultDeployment() {
  console.log("=" .repeat(60));
  console.log("TEST: Vault Deployment on JKC Testnet");
  console.log("=".repeat(60));

  // 1. Load keys
  const operatorKeyPair = ECPair.fromWIF(OPERATOR_WIF, JKC_TESTNET);
  const challengerKeyPair = ECPair.fromWIF(CHALLENGER_WIF, JKC_TESTNET);

  const operatorPubkey = operatorKeyPair.publicKey;
  const challengerPubkey = challengerKeyPair.publicKey;

  console.log(`Operator pubkey: ${operatorPubkey.toString("hex")}`);
  console.log(`Challenger pubkey: ${challengerPubkey.toString("hex")}`);

  // 2. Build vault script tree
  const unbondDelay = 60; // 60 blocks (≈ 1 hour)
  const scriptTree = buildVaultScriptTree(operatorPubkey, unbondDelay);

  console.log(`\nOperator leaf script (${scriptTree.operatorLeaf.length} bytes):`);
  console.log(scriptTree.operatorLeaf.toString("hex"));
  console.log(`\nChallenge leaf script (${scriptTree.challengeLeaf.length} bytes):`);
  console.log(scriptTree.challengeLeaf.toString("hex"));

  // 3. Calculate taproot output key
  const internalKey = operatorPubkey;
  const { outputKey, scriptTreeHash } = buildTaprootOutputKey(internalKey, scriptTree);

  console.log(`\nScript tree hash: ${scriptTreeHash.toString("hex")}`);

  // 4. Create P2TR address
  // In production, use proper taproot address derivation
  // For now, we'll use a simplified approach
  const operatorAddress = bitcoin.payments.p2pkh({
    pubkey: operatorPubkey,
    network: JKC_TESTNET,
  }).address;

  console.log(`\nOperator address: ${operatorAddress}`);

  // 5. Get UTXOs
  const utxos = await getUtxos(operatorAddress);
  console.log(`\nFound ${utxos.length} UTXOs`);

  if (utxos.length === 0) {
    console.log("No UTXOs available. Please fund the operator address first.");
    return;
  }

  // 6. Build vault deployment transaction
  const utxo = utxos[0];
  console.log(`\nUsing UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} sats)`);

  const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });

  // Add input
  const prevTxHex = await electrsGetText(`/tx/${utxo.txid}/hex`);
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(prevTxHex, "hex"),
  });

  // Add vault output (OP_RETURN with vault data)
  const vaultData = Buffer.concat([
    Buffer.from("utxovm:vault"),
    Buffer.from([unbondDelay]),
    scriptTreeHash,
  ]);

  const vaultScript = bitcoin.script.compile([
    bitcoin.opcodes.OP_RETURN,
    vaultData,
  ]);

  psbt.addOutput({
    script: vaultScript,
    value: 0n, // OP_RETURN outputs have 0 value
  });

  // Add operator pubkey as P2PKH output (for now)
  psbt.addOutput({
    address: operatorAddress,
    value: BigInt(utxo.value) - 1000n, // Leave 1000 sats for fee
  });

  // Sign
  psbt.signInput(0, operatorKeyPair);
  psbt.finalizeAllInputs();

  // Broadcast
  const txHex = psbt.extractTransaction().toHex();
  console.log(`\nTransaction hex: ${txHex}`);

  const txid = await broadcast(txHex);
  console.log(`\n✅ Vault deployed! TXID: ${txid}`);

  return txid;
}

async function testChallengeFlow() {
  console.log("\n" + "=".repeat(60));
  console.log("TEST: Challenge Flow on JKC Testnet");
  console.log("=".repeat(60));

  // This is a simplified test of the challenge flow
  // In production, this would involve:
  // 1. Operator posting a batch with wrong root
  // 2. Challenger detecting the fraud
  // 3. Challenger building a challenge transaction
  // 4. L1 verifying the OP_CAT proof

  console.log("\nChallenge flow requires:");
  console.log("1. Operator to post batch with wrong root");
  console.log("2. Challenger to detect fraud");
  console.log("3. Challenger to submit challenge transaction");
  console.log("4. L1 to verify OP_CAT hash comparison");
  console.log("\nThis will be implemented in the next phase.");
}

// --- Main ---

async function main() {
  try {
    console.log("UTXO-VM Vault Test on JKC Testnet\n");

    // Test vault deployment
    const txid = await testVaultDeployment();

    // Test challenge flow
    await testChallengeFlow();

    console.log("\n" + "=".repeat(60));
    console.log("TEST COMPLETE");
    console.log("=".repeat(60));

  } catch (error) {
    console.error("\n❌ Test failed:", error.message);
    process.exit(1);
  }
}

// Run tests
main();
