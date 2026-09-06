#!/usr/bin/env node
/**
 * JKC Testnet Opcode Broadcast Test
 *
 * Tests the utxovm inscription envelope on Junkcoin testnet via electrs.
 * Uses bitcoinjs-lib v7 + ECPair for P2PKH transaction building and signing.
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const ECPair = ECPairFactory(ecc);

// --- Config ---
const ELECTRS_URL = "https://jkc-testnet-api.s3na.xyz";
const WALLET_ADDR = "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi";
const WALLET_WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";

// Junkcoin testnet network params
const JKC_TESTNET = {
  messagePrefix: "\x19Junkcoin Signed Message:\n",
  bech32: "tjc",
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 111,
  scriptHash: 196,
  wif: 239,
};

// --- Inscription Envelope Builder ---
function buildInscriptionEnvelope(protocol, version, contentType, payload) {
  const protoBuf = Buffer.from(protocol, "utf8");
  const typeBuf = Buffer.from(contentType, "utf8");
  const payloadBuf = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);

  const parts = [Buffer.from([0x00, 0x63])]; // OP_FALSE OP_IF

  // Protocol tag
  parts.push(Buffer.from([protoBuf.length]));
  parts.push(protoBuf);

  // Version
  parts.push(Buffer.from([0x01, version]));

  // Content type
  parts.push(Buffer.from([typeBuf.length]));
  parts.push(typeBuf);

  // Payload with length
  const len = payloadBuf.length;
  if (len < 0x4c) {
    parts.push(Buffer.from([len]));
  } else if (len <= 0xff) {
    parts.push(Buffer.from([0x4c, len]));
  } else if (len <= 0xffff) {
    parts.push(Buffer.from([0x4d, len & 0xff, (len >> 8) & 0xff]));
  } else {
    parts.push(Buffer.from([0x4e, len & 0xff, (len >> 8) & 0xff, (len >> 16) & 0xff, (len >> 24) & 0xff]));
  }

  parts.push(payloadBuf);
  parts.push(Buffer.from([0x68])); // OP_ENDIF

  return Buffer.concat(parts);
}

// --- Fetch helpers ---
async function electrsGetText(path) {
  const res = await fetch(`${ELECTRS_URL}${path}`);
  if (!res.ok) throw new Error(`electrs GET ${path} failed: ${res.status}`);
  return (await res.text()).trim();
}

async function electrsGetJson(path) {
  const res = await fetch(`${ELECTRS_URL}${path}`);
  if (!res.ok) throw new Error(`electrs GET ${path} failed: ${res.status}`);
  return res.json();
}

// --- Main ---
async function main() {
  console.log("=== JKC Testnet Opcode Broadcast Test ===\n");

  // 1. Get tip height
  const tipHeight = await electrsGetText("/blocks/tip/height");
  console.log(`Tip height: ${tipHeight}`);

  // 2. Get UTXOs
  const utxos = await electrsGetJson(`/address/${WALLET_ADDR}/utxo`);
  console.log(`UTXOs for ${WALLET_ADDR}:`);
  for (const u of utxos) {
    console.log(`  ${u.txid}:${u.vout} — ${u.value} sats (confirmed: ${u.status?.confirmed})`);
  }

  if (utxos.length === 0) {
    console.error("No UTXOs found!");
    process.exit(1);
  }

  const utxo = utxos[0];
  console.log(`\nUsing UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} sats)`);

  // 3. Build inscription payload
  const callPayload = {
    targetSeal: `${utxo.txid}:${utxo.vout}`,
    method: "init",
    args: { name: "TestJKC", symbol: "tJKC", chain: "junkcoin-testnet" },
  };

  const envelope = buildInscriptionEnvelope(
    "utxovm",
    1,
    "application/json",
    Buffer.from(JSON.stringify(callPayload))
  );

  console.log(`\nInscription envelope (${envelope.length} bytes):`);
  console.log(`  hex: ${envelope.toString("hex")}`);
  console.log(`  OP_FALSE OP_IF: ${envelope[0] === 0x00 && envelope[1] === 0x63}`);
  console.log(`  OP_ENDIF: ${envelope[envelope.length - 1] === 0x68}`);

  // 4. Create ECPair from WIF
  const keyPair = ECPair.fromWIF(WALLET_WIF, JKC_TESTNET);
  console.log(`\nPublic key: ${keyPair.publicKey.toString("hex")}`);

  // 5. Get previous transaction hex
  const prevTxHex = await electrsGetText(`/tx/${utxo.txid}/hex`);
  const prevTx = bitcoin.Transaction.fromHex(prevTxHex);
  const prevOutput = prevTx.outs[utxo.vout];
  console.log(`\nPrev output scriptPubKey: ${prevOutput.script.toString("hex")}`);

  // 6. Build transaction with PSBT
  const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });

  // Input with non-witness UTXO
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(prevTxHex, "hex"),
  });

  // Output 0: OP_RETURN + inscription envelope
  // bitcoinjs-lib needs OP_RETURN built via script.compile
  const opReturnScript = bitcoin.script.compile([
    bitcoin.opcodes.OP_RETURN,
    envelope,
  ]);
  psbt.addOutput({
    script: opReturnScript,
    value: 0n,
  });

  // Output 1: change
  const fee = 1000n;
  const changeValue = BigInt(utxo.value) - fee;
  psbt.addOutput({
    address: WALLET_ADDR,
    value: changeValue,
  });

  console.log(`\nTransaction plan:`);
  console.log(`  Input:  ${utxo.txid}:${utxo.vout} (${utxo.value} sats)`);
  console.log(`  Out[0]: OP_RETURN inscription (0 sats)`);
  console.log(`  Out[1]: Change → ${WALLET_ADDR} (${changeValue} sats)`);
  console.log(`  Fee:    ${fee} sats`);

  // 7. Sign
  psbt.signInput(0, keyPair);
  console.log(`\nSigned.`);

  // 8. Finalize & extract
  psbt.finalizeAllInputs();
  const finalTx = psbt.extractTransaction();
  const txHex = finalTx.toHex();
  console.log(`\nFinal tx (${finalTx.byteLength()} bytes):`);
  console.log(txHex);

  // 9. Broadcast
  console.log("\n--- Broadcasting ---");
  const broadcastRes = await fetch(`${ELECTRS_URL}/tx`, {
    method: "POST",
    headers: { "Content-Type": "text/plain" },
    body: txHex,
  });
  const txid = (await broadcastRes.text()).trim();
  if (!broadcastRes.ok) {
    console.error(`Broadcast FAILED: ${txid}`);
    process.exit(1);
  }
  console.log(`SUCCESS! TXID: ${txid}`);

  // 10. Verify
  console.log("\n--- Verifying ---");
  const txInfo = await electrsGetJson(`/tx/${txid}`);
  console.log(`Status: ${JSON.stringify(txInfo.status, null, 2)}`);

  const vout0 = txInfo.vout[0];
  console.log(`\nOutput[0]:`);
  console.log(`  type:  ${vout0.scriptpubkey_type}`);
  console.log(`  value: ${vout0.value} sats`);
  console.log(`  asm:   ${vout0.scriptpubkey_asm}`);

  // Check envelope content in hex
  const txHexConfirm = await electrsGetText(`/tx/${txid}/hex`);
  console.log(`\nContains "utxovm": ${txHexConfirm.includes("7574786f766d")}`);
  console.log(`Contains OP_FALSE OP_IF: ${txHexConfirm.startsWith("0063") || txHexConfirm.includes("0063")}`);
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
