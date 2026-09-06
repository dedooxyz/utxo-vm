#!/usr/bin/env node
/**
 * Cross-Chain State Anchor Test
 * 
 * Simulates child chain anchoring state to Junkcoin
 * Tests: state anchor creation, seal verification, merkle proof
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const ECPair = ECPairFactory(ecc);
const crypto = require("crypto");

const ELECTRS = "https://jkc-testnet-api.s3na.xyz";
const INDEXER = "http://localhost:9773";
const JKC_TESTNET = {
  messagePrefix: "\x19Junkcoin Signed Message:\n",
  bech32: "tjc",
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 111, scriptHash: 196, wif: 239,
};
const WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";
const keyPair = ECPair.fromWIF(WIF, JKC_TESTNET);
const ADDR = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network: JKC_TESTNET }).address;

// =====================================================
// Merkle Tree
// =====================================================
class MerkleTree {
  constructor(leaves) {
    this.leaves = leaves.map(l => Buffer.from(l, "hex"));
    this.tree = this.buildTree(this.leaves);
  }

  buildTree(leaves) {
    if (leaves.length === 0) return [];
    let level = leaves;
    const tree = [level];
    while (level.length > 1) {
      const nextLevel = [];
      for (let i = 0; i < level.length; i += 2) {
        const left = level[i];
        const right = i + 1 < level.length ? level[i + 1] : left;
        const combined = Buffer.concat([left, right]);
        const hash = crypto.createHash("sha256").update(combined).digest();
        nextLevel.push(hash);
      }
      level = nextLevel;
      tree.push(level);
    }
    return tree;
  }

  getRoot() {
    if (this.tree.length === 0) return Buffer.alloc(0);
    return this.tree[this.tree.length - 1][0];
  }

  getProof(index) {
    const proof = [];
    let idx = index;
    for (let i = 0; i < this.tree.length - 1; i++) {
      const level = this.tree[i];
      const isRight = idx % 2 === 1;
      const siblingIdx = isRight ? idx - 1 : idx + 1;
      if (siblingIdx < level.length) {
        proof.push({
          hash: level[siblingIdx].toString("hex"),
          position: isRight ? "left" : "right"
        });
      }
      idx = Math.floor(idx / 2);
    }
    return proof;
  }

  static verify(leaf, proof, root) {
    let current = Buffer.from(leaf, "hex");
    for (const step of proof) {
      const sibling = Buffer.from(step.hash, "hex");
      const combined = step.position === "left" 
        ? Buffer.concat([sibling, current])
        : Buffer.concat([current, sibling]);
      current = crypto.createHash("sha256").update(combined).digest();
    }
    return current.equals(Buffer.from(root, "hex"));
  }
}

// =====================================================
// Build Envelope
// =====================================================
function buildEnvelope(protocol, version, contentType, payload) {
  const protoBuf = Buffer.from(protocol, "utf8");
  const typeBuf = Buffer.from(contentType, "utf8");
  const payloadBuf = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const parts = [Buffer.from([0x00, 0x63])];
  parts.push(Buffer.from([protoBuf.length]), protoBuf);
  parts.push(Buffer.from([0x01, version]));
  parts.push(Buffer.from([typeBuf.length]), typeBuf);
  const len = payloadBuf.length;
  if (len < 0x4c) parts.push(Buffer.from([len]));
  else if (len <= 0xff) parts.push(Buffer.from([0x4c, len]));
  else if (len <= 0xffff) parts.push(Buffer.from([0x4d, len & 0xff, (len >> 8) & 0xff]));
  parts.push(payloadBuf, Buffer.from([0x68]));
  return Buffer.concat(parts);
}

// =====================================================
// Broadcast and Confirm
// =====================================================
async function broadcastAndConfirm(envelopes, fee = 1000n) {
  const utxos = await (await fetch(`${ELECTRS}/address/${ADDR}/utxo`)).json();
  if (!utxos.length) throw new Error("No UTXOs");
  const utxo = utxos[0];
  const prevTxHex = await (await fetch(`${ELECTRS}/tx/${utxo.txid}/hex`)).text();
  const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });
  psbt.addInput({ hash: utxo.txid, index: utxo.vout, nonWitnessUtxo: Buffer.from(prevTxHex, "hex") });
  for (const env of envelopes) {
    const script = bitcoin.script.compile([bitcoin.opcodes.OP_RETURN, env]);
    psbt.addOutput({ script, value: 0n });
  }
  psbt.addOutput({ address: ADDR, value: BigInt(utxo.value) - fee });
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  const txHex = psbt.extractTransaction().toHex();
  const res = await fetch(`${ELECTRS}/tx`, { method: "POST", headers: { "Content-Type": "text/plain" }, body: txHex });
  const txid = (await res.text()).trim();
  if (!res.ok) throw new Error(`Broadcast: ${txid}`);
  
  let tries = 0;
  while (true) {
    await new Promise(r => setTimeout(r, 10000));
    tries++;
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      if (info.status?.confirmed) return { txid, block: info.status.block_height };
    } catch {}
    if (tries % 6 === 0) console.log(`    Waiting... (${tries * 10}s)`);
  }
}

// =====================================================
// Main Test
// =====================================================
async function main() {
  console.log("╔══════════════════════════════════════════════════════╗");
  console.log("║  Cross-Chain State Anchor Test                     ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");
  console.log(`Address: ${ADDR}`);
  console.log(`Indexer: ${INDEXER}\n`);

  const results = [];

  // ─── 1. CREATE CHILD CHAIN STATE ───
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 1: Create Child Chain State");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  // Simulate child chain block with transactions
  const childChainTxs = [
    crypto.randomBytes(32).toString("hex"),
    crypto.randomBytes(32).toString("hex"),
    crypto.randomBytes(32).toString("hex"),
    crypto.randomBytes(32).toString("hex"),
    crypto.randomBytes(32).toString("hex"),
  ];

  console.log(`  Child chain transactions: ${childChainTxs.length}`);
  for (const tx of childChainTxs) {
    console.log(`    ${tx.slice(0, 16)}...`);
  }

  // Build merkle tree
  const tree = new MerkleTree(childChainTxs);
  const merkleRoot = tree.getRoot().toString("hex");
  console.log(`\n  Merkle Root: ${merkleRoot}`);

  // Generate state root (hash of all state changes)
  const stateData = JSON.stringify({
    blockHeight: 12345,
    blockHash: crypto.randomBytes(32).toString("hex"),
    merkleRoot,
    timestamp: Date.now(),
  });
  const stateRoot = crypto.createHash("sha256").update(stateData).digest("hex");
  console.log(`  State Root: ${stateRoot}`);

  results.push({ step: "Create Child Chain State", ok: true });

  // ─── 2. CREATE STATE ANCHOR ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 2: Create State Anchor for Junkcoin");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const anchor = {
    type: "state_anchor",
    chainId: "child_chain_a_testnet",
    blockHeight: 12345,
    blockHash: "0x" + crypto.randomBytes(32).toString("hex"),
    stateRoot: "0x" + stateRoot,
    merkleRoot: "0x" + merkleRoot,
    prevSeal: null,
    timestamp: Date.now(),
    txCount: childChainTxs.length,
  };

  console.log(`  Chain ID: ${anchor.chainId}`);
  console.log(`  Block Height: ${anchor.blockHeight}`);
  console.log(`  Merkle Root: ${anchor.merkleRoot}`);
  console.log(`  State Root: ${anchor.stateRoot}`);

  results.push({ step: "Create State Anchor", ok: true });

  // ─── 3. BROADCAST ANCHOR TO JUNKCOIN ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 3: Broadcast Anchor to Junkcoin");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const anchorEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify(anchor)));
  console.log(`  Envelope size: ${anchorEnv.length} bytes`);

  const anchorResult = await broadcastAndConfirm([anchorEnv]);
  console.log(`  TXID: ${anchorResult.txid}`);
  console.log(`  Block: ${anchorResult.block}`);

  const anchorSeal = `${anchorResult.txid}:0`;
  console.log(`  Seal: ${anchorSeal}`);

  results.push({ step: "Broadcast Anchor", ok: true, txid: anchorResult.txid });

  // ─── 4. VERIFY SEAL ON JUNKCOIN ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 4: Verify Seal on Junkcoin");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  // Sync indexer
  const tip = parseInt(await (await fetch(`${ELECTRS}/blocks/tip/height`)).text(), 10);
  await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tip - 10 }),
  });

  // Query seal
  const objId = `obj_${anchorResult.txid.slice(0, 16)}`;
  const obj = await (await fetch(`${INDEXER}/api/v1/object/${objId}`)).json();
  
  console.log(`  Object ID: ${objId}`);
  console.log(`  Seal: ${obj.seal}`);
  console.log(`  Seal matches: ${obj.seal === anchorSeal ? "✅ YES" : "❌ NO"}`);
  console.log(`  State data: ${JSON.stringify(obj.stateData).slice(0, 100)}...`);

  results.push({ step: "Verify Seal", ok: obj.seal === anchorSeal });

  // ─── 5. VERIFY MERKLE PROOF ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 5: Verify Merkle Proof");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  // Get proof for first transaction
  const proof = tree.getProof(0);
  console.log(`  Transaction: ${childChainTxs[0].slice(0, 16)}...`);
  console.log(`  Proof steps: ${proof.length}`);

  // Verify proof
  const proofValid = MerkleTree.verify(childChainTxs[0], proof, merkleRoot);
  console.log(`  Proof valid: ${proofValid ? "✅ YES" : "❌ NO"}`);

  // Verify against stored merkle root
  const storedRoot = obj.stateData?.merkleRoot?.replace("0x", "");
  const rootMatch = storedRoot === merkleRoot;
  console.log(`  Root matches stored: ${rootMatch ? "✅ YES" : "❌ NO"}`);

  results.push({ step: "Verify Merkle Proof", ok: proofValid && rootMatch });

  // ─── 6. CREATE STATE CLAIM ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 6: Create State Claim");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const claim = {
    type: "state_claim",
    chainId: "child_chain_a_testnet",
    junkcoinSeal: anchorSeal,
    merkleRoot: "0x" + merkleRoot,
    proof: proof,
    signatures: ["0x" + crypto.randomBytes(64).toString("hex")],
    timestamp: Date.now(),
  };

  console.log(`  Chain ID: ${claim.chainId}`);
  console.log(`  Junkcoin Seal: ${claim.junkcoinSeal}`);
  console.log(`  Merkle Root: ${claim.merkleRoot}`);
  console.log(`  Proof steps: ${claim.proof.length}`);
  console.log(`  Signatures: ${claim.signatures.length}`);

  results.push({ step: "Create State Claim", ok: true });

  // ─── 7. SIMULATE CHILD CHAIN VERIFICATION ───
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 7: Simulate Child Chain Verification");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  // Child chain queries Junkcoin for seal
  const verifySeal = await (await fetch(`${INDEXER}/api/v1/object/${objId}`)).json();
  console.log(`  1. Query seal from Junkcoin: ${verifySeal.seal ? "✅ FOUND" : "❌ NOT FOUND"}`);

  // Child chain verifies merkle root
  const verifyRoot = verifySeal.stateData?.merkleRoot?.replace("0x", "") === merkleRoot;
  console.log(`  2. Verify merkle root: ${verifyRoot ? "✅ MATCH" : "❌ MISMATCH"}`);

  // Child chain verifies proof
  const verifyProof = MerkleTree.verify(childChainTxs[0], proof, merkleRoot);
  console.log(`  3. Verify merkle proof: ${verifyProof ? "✅ VALID" : "❌ INVALID"}`);

  // Child chain accepts state
  const stateAccepted = verifySeal.seal && verifyRoot && verifyProof;
  console.log(`  4. Accept state: ${stateAccepted ? "✅ ACCEPTED" : "❌ REJECTED"}`);

  results.push({ step: "Child Chain Verification", ok: stateAccepted });

  // ─── FINAL SUMMARY ───
  console.log("\n╔══════════════════════════════════════════════════════╗");
  console.log("║  FINAL RESULTS                                      ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");

  console.log(`${"Step".padEnd(35)} Status`);
  console.log("─".repeat(55));
  for (const r of results) {
    console.log(`${r.step.padEnd(35)} ${r.ok ? "✅" : "❌"}`);
  }
  console.log("─".repeat(55));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  // Verify via electrs
  console.log("--- Electrs Verification ---");
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tipFinal}`);

  const txInfo = await (await fetch(`${ELECTRS}/tx/${anchorResult.txid}`)).json();
  const hasOpReturn = txInfo.vout.some(v => v.scriptpubkey_type === "op_return");
  console.log(`Anchor TX: ${anchorResult.txid}`);
  console.log(`Block: ${txInfo.status?.block_height}`);
  console.log(`OP_RETURN: ${hasOpReturn ? "✅ YES" : "❌ NO"}`);

  console.log("\n=== Cross-Chain State Anchor Test Complete ===");
}

main().catch(console.error);
