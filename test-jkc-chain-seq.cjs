#!/usr/bin/env node
/**
 * JKC Testnet — Sequential chain test
 * Broadcast one tx, wait for confirmation, get new UTXO, broadcast next
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const ECPair = ECPairFactory(ecc);

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
  else parts.push(Buffer.from([0x4e, len & 0xff, (len >> 8) & 0xff, (len >> 16) & 0xff, (len >> 24) & 0xff]));
  parts.push(payloadBuf, Buffer.from([0x68]));
  return Buffer.concat(parts);
}

async function getUtxo() {
  const utxos = await (await fetch(`${ELECTRS}/address/${ADDR}/utxo`)).json();
  if (!utxos.length) throw new Error("No UTXOs available");
  return utxos[0];
}

async function broadcastAndConfirm(envelopes, name, fee = 1000n) {
  const utxo = await getUtxo();
  console.log(`  UTXO: ${utxo.txid.slice(0, 16)}:${utxo.vout} (${utxo.value} sats)`);

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
  console.log(`  TXID: ${txid}`);

  // Wait for confirmation
  let tries = 0;
  while (true) {
    await new Promise(r => setTimeout(r, 10000));
    tries++;
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      if (info.status?.confirmed) {
        console.log(`  Confirmed in block ${info.status.block_height} (${tries} tries)`);
        return txid;
      }
    } catch {}
    if (tries % 6 === 0) console.log(`  Still waiting... (${tries * 10}s)`);
  }
}

async function syncAndCheck() {
  const tip = parseInt(await (await fetch(`${ELECTRS}/blocks/tip/height`)).text(), 10);
  await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tip - 20 }),
  });
}

const results = [];
let currentSeal = null;

async function runTest(name, fn) {
  console.log(`\n${"─".repeat(50)}`);
  console.log(`TEST: ${name}`);
  console.log("─".repeat(50));
  try {
    const out = await fn();
    console.log(`  ✅ PASS`);
    results.push({ name, ok: true, ...out });
    return out;
  } catch (err) {
    console.error(`  ❌ FAIL: ${err.message}`);
    results.push({ name, ok: false, error: err.message });
    return null;
  }
}

async function main() {
  console.log("=== JKC Testnet — Sequential Chain Test ===");
  console.log(`Address: ${ADDR}\n`);

  const tip = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tip.trim()}`);

  // ─── 1. DEPLOY WASM ───
  await runTest("1. Deploy WASM", async () => {
    const wasm = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
      0x03, 0x02, 0x01, 0x00,
      0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
      0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ]);
    const env = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const txid = await broadcastAndConfirm([env], "Deploy WASM");
    await syncAndCheck();
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const deploy = objs.find(o => o.seal.startsWith(txid));
    if (deploy) currentSeal = deploy.seal;
    return { txid: txid.slice(0, 16), seal: currentSeal };
  });

  // ─── 2. TRANSFER CALL ───
  await runTest("2. Transfer call", async () => {
    if (!currentSeal) throw new Error("No deployed contract");
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "transfer",
      args: { to: "02cafebabe00000000000000000000000000000000000000000000000000000000", amount: 250 },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastAndConfirm([env], "Transfer");
    await syncAndCheck();
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const obj = objs.find(o => o.seal.includes(txid.slice(0, 16)));
    if (obj) currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal };
  });

  // ─── 3. STEALTH SETTLEMENT ───
  await runTest("3. Stealth settlement", async () => {
    if (!currentSeal) throw new Error("No active seal");
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "stealth_settle",
      args: { stealthAddress: "tjc1qstealth_e2e", scanKey: "02" + "aa".repeat(32), amount: 10000 },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastAndConfirm([env], "Stealth");
    await syncAndCheck();
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const obj = objs.find(o => o.seal.includes(txid.slice(0, 16)));
    if (obj) currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal };
  });

  // ─── 4. STATE TRANSITION ───
  await runTest("4. State transition (update_state)", async () => {
    if (!currentSeal) throw new Error("No active seal");
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "update_state",
      args: { field: "totalSupply", value: "5000000", caller: ADDR },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastAndConfirm([env], "StateUpdate");
    await syncAndCheck();
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const obj = objs.find(o => o.seal.includes(txid.slice(0, 16)));
    if (obj) currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal };
  });

  // ─── 5. BATCH (2 envelopes) ───
  let batchSeal = null;
  await runTest("5. Batch deploy (2 envelopes)", async () => {
    const wasm = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    const env1 = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const env2 = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const txid = await broadcastAndConfirm([env1, env2], "Batch", 2000n);
    await syncAndCheck();
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const batchObjs = objs.filter(o => o.seal.startsWith(txid));
    if (batchObjs.length >= 2) batchSeal = batchObjs[0].seal;
    return { txid: txid.slice(0, 16), count: batchObjs.length };
  });

  // ─── 6. TRANSFER ON BATCH OBJECT ───
  await runTest("6. Transfer on batch object", async () => {
    if (!batchSeal) throw new Error("No batch object");
    const payload = JSON.stringify({
      targetSeal: batchSeal,
      method: "transfer",
      args: { to: "03deadbeef00000000000000000000000000000000000000000000000000000000" },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastAndConfirm([env], "BatchTransfer");
    await syncAndCheck();
    return { txid: txid.slice(0, 16) };
  });

  // ─── 7. TINY PAYLOAD ───
  await runTest("7. Tiny payload (1-byte)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", Buffer.from([0xff]));
    const txid = await broadcastAndConfirm([env], "Tiny");
    await syncAndCheck();
    return { txid: txid.slice(0, 16) };
  });

  // ─── 8. MEDIUM PAYLOAD ───
  await runTest("8. Medium payload (100 bytes)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", Buffer.alloc(100, 0xcc));
    const txid = await broadcastAndConfirm([env], "Medium");
    await syncAndCheck();
    return { txid: txid.slice(0, 16) };
  });

  // ─── 9. LARGE PAYLOAD ───
  await runTest("9. Large payload (400 bytes)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/wasm", Buffer.alloc(400, 0xab));
    const txid = await broadcastAndConfirm([env], "Large");
    await syncAndCheck();
    return { txid: txid.slice(0, 16) };
  });

  // ─── 10. CHAIN: 3 HOPS ───
  let chainSeal = currentSeal;
  for (let hop = 1; hop <= 3; hop++) {
    await runTest(`10.${hop}. Chain hop ${hop}`, async () => {
      if (!chainSeal) throw new Error("No chain seal");
      const payload = JSON.stringify({
        targetSeal: chainSeal,
        method: "transfer",
        args: { to: `02hop${hop}0000000000000000000000000000000000000000000000000000000000` },
      });
      const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
      const txid = await broadcastAndConfirm([env], `Hop${hop}`);
      await syncAndCheck();
      const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
      // Find object that was updated (owner changed)
      const obj = objs.find(o => o.seal.includes(txid.slice(0, 16)));
      if (obj) chainSeal = obj.seal;
      return { txid: txid.slice(0, 16), hop, newSeal: chainSeal };
    });
  }

  // ─── 11. PARSE ENVELOPE ───
  await runTest("11. Parse envelope endpoint", async () => {
    const testScript = "6a4c780063067574786f766d0101106170706c69636174696f6e2f6a736f6e4c597b227461726765745365616c223a2274785f67656e657369733a30222c226d6574686f64223a227472616e73666572222c2261726773223a7b22746f223a2230326361666562616265222c22616d6f756e74223a3530307d7d68";
    const res = await fetch(`${INDEXER}/api/v1/parse-envelope`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ scriptPubKey: testScript }),
    });
    const parsed = await res.json();
    return { protocol: parsed.protocol, contentType: parsed.contentType };
  });

  // ─── 12. HISTORY ───
  await runTest("12. Full history", async () => {
    const res = await fetch(`${INDEXER}/api/v1/object/obj_a883ad7cb99ba50c/history`);
    const history = await res.json();
    return { transitions: history.length, methods: history.map(h => h.method) };
  });

  // ─── 13. ALL OBJECTS ───
  await runTest("13. List all objects", async () => {
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    return { total: objs.length };
  });

  // =====================================================
  // SUMMARY
  // =====================================================
  console.log(`\n\n${"═".repeat(60)}`);
  console.log("FINAL RESULTS");
  console.log("═".repeat(60));
  console.log(`${"#".padEnd(3)} ${"Test".padEnd(45)} Status`);
  console.log("─".repeat(60));
  results.forEach((r, i) => {
    const s = r.ok ? "✅" : "❌";
    console.log(`${(i + 1).toString().padEnd(3)} ${r.name.padEnd(45)} ${s}`);
  });
  console.log("─".repeat(60));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  // Final electrs verification
  console.log("--- Electrs Verification ---");
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tipFinal}`);
  const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  console.log(`Indexed objects: ${objs.length}`);
  for (const o of objs) {
    const txid = o.seal.split(":")[0];
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      const hasOpReturn = info.vout.some(v => v.scriptpubkey_type === "op_return");
      const envs = info.vout.filter(v => v.scriptpubkey_type === "op_return");
      console.log(`  ${o.objectId.slice(0, 22)} — block ${info.status?.block_height} — OP_RETURN: ${envs.length} — envs: ${hasOpReturn}`);
    } catch { console.log(`  ${o.objectId.slice(0, 22)} — not found`); }
  }
}

main().catch(console.error);
