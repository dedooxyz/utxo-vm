#!/usr/bin/env node
/**
 * JKC Testnet — Full Opcode Test Suite
 *
 * Tests all utxovm inscription envelope variants on Junkcoin testnet via electrs:
 *   1. Deploy WASM        (application/wasm)
 *   2. Transfer call      (application/json, method=transfer)
 *   3. Stealth settlement (application/json, method=stealth_settle)
 *   4. State transition   (application/json, method=update_state)
 *   5. Batch multi-envelope (2 envelopes in 1 tx)
 *   6. Tiny payload       (minimal 1-byte)
 *   7. Large payload      (>255 bytes, OP_PUSHDATA2)
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const ECPair = ECPairFactory(ecc);
const crypto = require("crypto");

// --- Config ---
const ELECTRS = "https://jkc-testnet-api.s3na.xyz";
const JKC_TESTNET = {
  messagePrefix: "\x19Junkcoin Signed Message:\n",
  bech32: "tjc",
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 111,
  scriptHash: 196,
  wif: 239,
};

const WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";
const keyPair = ECPair.fromWIF(WIF, JKC_TESTNET);
const PUBKEY = keyPair.publicKey.toString("hex");

// --- Envelope builder ---
function buildEnvelope(protocol, version, contentType, payload) {
  const protoBuf = Buffer.from(protocol, "utf8");
  const typeBuf = Buffer.from(contentType, "utf8");
  const payloadBuf = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const parts = [Buffer.from([0x00, 0x63])]; // OP_FALSE OP_IF
  parts.push(Buffer.from([protoBuf.length]), protoBuf);
  parts.push(Buffer.from([0x01, version]));
  parts.push(Buffer.from([typeBuf.length]), typeBuf);
  const len = payloadBuf.length;
  if (len < 0x4c) parts.push(Buffer.from([len]));
  else if (len <= 0xff) parts.push(Buffer.from([0x4c, len]));
  else if (len <= 0xffff) parts.push(Buffer.from([0x4d, len & 0xff, (len >> 8) & 0xff]));
  else parts.push(Buffer.from([0x4e, len & 0xff, (len >> 8) & 0xff, (len >> 16) & 0xff, (len >> 24) & 0xff]));
  parts.push(payloadBuf, Buffer.from([0x68]));
  return Buffer.concat(parts);
}

// --- Fetch helpers ---
async function electrsGet(path) {
  const res = await fetch(`${ELECTRS}${path}`);
  if (!res.ok) throw new Error(`GET ${path}: ${res.status}`);
  return res;
}
async function electrsGetText(path) { return (await (await electrsGet(path)).text()).trim(); }
async function electrsGetJson(path) { return (await electrsGet(path)).json(); }
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

// --- Build & sign a single-envelope tx ---
async function buildSignBroadcast(utxo, envelopes, feeSats = 1000n) {
  const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });
  const prevTxHex = await electrsGetText(`/tx/${utxo.txid}/hex`);
  psbt.addInput({ hash: utxo.txid, index: utxo.vout, nonWitnessUtxo: Buffer.from(prevTxHex, "hex") });

  for (const env of envelopes) {
    const script = bitcoin.script.compile([bitcoin.opcodes.OP_RETURN, env]);
    psbt.addOutput({ script, value: 0n });
  }

  const changeValue = BigInt(utxo.value) - feeSats;
  psbt.addOutput({ address: keyPair.address || P2PKH_ADDR, value: changeValue });
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  return broadcast(psbt.extractTransaction().toHex());
}

// Derive address
const P2PKH_ADDR = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network: JKC_TESTNET }).address;

// --- Test registry ---
const results = [];

async function runTest(name, fn) {
  console.log(`\n${"=".repeat(60)}`);
  console.log(`TEST: ${name}`);
  console.log("=".repeat(60));
  try {
    const txid = await fn();
    console.log(`  TXID: ${txid}`);
    console.log(`  Explorer: ${ELECTRS.replace("api", "dedoo.xyz").replace("jkc-testnet-api", "jkc-testnet")}/tx/${txid}`);
    results.push({ name, txid, ok: true });
    return txid;
  } catch (err) {
    console.error(`  FAILED: ${err.message}`);
    results.push({ name, txid: null, ok: false, error: err.message });
    return null;
  }
}

async function getUtxo() {
  const utxos = await electrsGetJson(`/address/${P2PKH_ADDR}/utxo`);
  if (!utxos.length) throw new Error("No UTXOs available");
  return utxos[0];
}

// =====================================================
// MAIN
// =====================================================
async function main() {
  console.log("=== JKC Testnet — Full Opcode Test Suite ===");
  console.log(`Address: ${P2PKH_ADDR}`);
  console.log(`Electrs: ${ELECTRS}`);

  const tip = await electrsGetText("/blocks/tip/height");
  console.log(`Tip height: ${tip}\n`);

  // ---- Test 1: Deploy WASM ----
  await runTest("1. Deploy WASM (application/wasm)", async () => {
    const wasmBytes = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
      0x03, 0x02, 0x01, 0x00,
      0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
      0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ]);
    const env = buildEnvelope("utxovm", 1, "application/wasm", wasmBytes);
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // ---- Test 2: Transfer call ----
  await runTest("2. Transfer Call (method=transfer)", async () => {
    const payload = JSON.stringify({
      targetSeal: "tx_genesis:0",
      method: "transfer",
      args: { to: "02cafebabe", amount: 500 },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // ---- Test 3: Stealth settlement ----
  await runTest("3. Stealth Settlement (method=stealth_settle)", async () => {
    const payload = JSON.stringify({
      targetSeal: "tx_vault:1",
      method: "stealth_settle",
      args: {
        stealthAddress: "tjc1qstealth_abc123def456",
        scanPubKey: "02" + "aa".repeat(32),
        spendPubKey: "03" + "bb".repeat(32),
        amount: 25000,
      },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // ---- Test 4: State transition ----
  await runTest("4. State Transition (method=update_state)", async () => {
    const payload = JSON.stringify({
      targetSeal: "tx_token:0",
      method: "update_state",
      args: {
        field: "totalSupply",
        value: "2000000",
        caller: PUBKEY,
      },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // ---- Test 5: Batch multi-envelope (2 in 1 tx) ----
  await runTest("5. Batch: 2 Envelopes in 1 tx", async () => {
    const env1 = buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify({
      method: "mint", args: { tokenId: "tok_001", amount: 1000 },
    })));
    const env2 = buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify({
      method: "transfer", args: { to: "recipient_pubkey", amount: 100 },
    })));
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env1, env2], 2000n);
  });

  // ---- Test 6: Tiny payload (1 byte) ----
  await runTest("6. Tiny Payload (1-byte)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", Buffer.from([0xff]));
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // ---- Test 7: Large payload (>255 bytes, triggers OP_PUSHDATA2) ----
  await runTest("7. Large Payload (>255 bytes)", async () => {
    const largePayload = Buffer.alloc(400, 0xab); // 400 bytes of 0xab
    const env = buildEnvelope("utxovm", 1, "application/wasm", largePayload);
    console.log(`  Envelope size: ${env.length} bytes`);
    console.log(`  Payload push: ${env.length > 260 ? "OP_PUSHDATA2" : "OP_PUSHDATA1"}`);
    const utxo = await getUtxo();
    return buildSignBroadcast(utxo, [env]);
  });

  // =====================================================
  // SUMMARY
  // =====================================================
  console.log("\n\n" + "=".repeat(60));
  console.log("RESULTS SUMMARY");
  console.log("=".repeat(60));
  console.log(`${"Test".padEnd(45)} ${"Status".padEnd(10)} TXID`);
  console.log("-".repeat(85));
  for (const r of results) {
    const status = r.ok ? "✅ OK" : "❌ FAIL";
    const txid = r.txid ? r.txid.slice(0, 16) + "..." : r.error?.slice(0, 30);
    console.log(`${r.name.padEnd(45)} ${status.padEnd(10)} ${txid}`);
  }
  console.log(`\nTotal: ${results.filter(r => r.ok).length}/${results.length} passed`);

  // Verify all successful txs
  console.log("\n--- Verifying all txids on Electrs ---");
  for (const r of results.filter(r => r.ok)) {
    try {
      const info = await electrsGetJson(`/tx/${r.txid}`);
      const confirmed = info.status?.confirmed ? "confirmed" : "mempool";
      const hasOpReturn = info.vout[0]?.scriptpubkey_type === "op_return";
      const hasUtxovm = (info.vout[0]?.scriptpubkey_asm || "").includes("7574786f766d");
      console.log(`  ${r.txid.slice(0, 16)}... — ${confirmed} — OP_RETURN: ${hasOpReturn} — utxovm: ${hasUtxovm}`);
    } catch (e) {
      console.log(`  ${r.txid?.slice(0, 16)}... — verify error: ${e.message}`);
    }
  }
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
