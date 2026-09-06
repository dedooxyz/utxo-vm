#!/usr/bin/env node
/**
 * JKC Testnet — Full E2E Test Suite
 *
 * Deploy → Transfer → Stealth → State Transition → Batch → Tiny/Large
 * All via real electrs + indexer API
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
  if (!utxos.length) throw new Error("No UTXOs");
  return utxos[0];
}

async function broadcastEnvelope(envelopes, feeSats = 1000n) {
  const utxo = await getUtxo();
  const prevTxHex = await (await fetch(`${ELECTRS}/tx/${utxo.txid}/hex`)).text();
  const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });
  psbt.addInput({ hash: utxo.txid, index: utxo.vout, nonWitnessUtxo: Buffer.from(prevTxHex, "hex") });
  for (const env of envelopes) {
    const script = bitcoin.script.compile([bitcoin.opcodes.OP_RETURN, env]);
    psbt.addOutput({ script, value: 0n });
  }
  psbt.addOutput({ address: ADDR, value: BigInt(utxo.value) - feeSats });
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  const txHex = psbt.extractTransaction().toHex();
  const res = await fetch(`${ELECTRS}/tx`, { method: "POST", headers: { "Content-Type": "text/plain" }, body: txHex });
  const txid = (await res.text()).trim();
  if (!res.ok) throw new Error(`Broadcast: ${txid}`);
  return txid;
}

async function reSync(sinceBlock) {
  const res = await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock }),
  });
  return res.json();
}

async function getObject(id) {
  const res = await fetch(`${INDEXER}/api/v1/object/${id}`);
  return res.json();
}

async function getHistory(id) {
  const res = await fetch(`${INDEXER}/api/v1/object/${id}/history`);
  return res.json();
}

const results = [];
let currentSeal = null;
let currentObjectId = null;
let lastBlock = 0;

async function runTest(name, fn) {
  console.log(`\n${"─".repeat(60)}`);
  console.log(`TEST: ${name}`);
  console.log("─".repeat(60));
  try {
    const out = await fn();
    console.log(`  ✅ ${JSON.stringify(out).slice(0, 120)}`);
    results.push({ name, ok: true, ...out });
    return out;
  } catch (err) {
    console.error(`  ❌ ${err.message}`);
    results.push({ name, ok: false, error: err.message });
    return null;
  }
}

// =====================================================
async function main() {
  console.log("=== JKC Testnet — Full E2E Test Suite ===");
  console.log(`Address: ${ADDR}`);
  console.log(`Indexer: ${INDEXER}\n`);

  const tip = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  lastBlock = parseInt(tip.trim(), 10);
  console.log(`Tip: ${lastBlock}`);

  // ─── 1. DEPLOY WASM ───
  await runTest("1. Deploy WASM contract", async () => {
    const wasm = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
      0x03, 0x02, 0x01, 0x00,
      0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
      0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ]);
    const env = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

    // Find the deployed object
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const deployed = objs.find(o => o.seal.startsWith(txid));
    if (deployed) {
      currentSeal = deployed.seal;
      currentObjectId = deployed.objectId;
    }
    return { txid: txid.slice(0, 16), seal: currentSeal, synced: sync.indexed };
  });

  // ─── 2. TRANSFER CALL ───
  await runTest("2. Transfer call (state transition)", async () => {
    if (!currentSeal) throw new Error("No deployed contract");
    const recipient = "02cafebabe00000000000000000000000000000000000000000000000000000000";
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "transfer",
      args: { to: recipient, amount: 250 },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

    // Update current seal
    const obj = await getObject(currentObjectId);
    currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal, owner: obj.owner.slice(0, 20), synced: sync.indexed };
  });

  // ─── 3. STEALTH SETTLEMENT ───
  await runTest("3. Stealth settlement", async () => {
    if (!currentSeal) throw new Error("No active seal");
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "stealth_settle",
      args: {
        stealthAddress: "tjc1qstealth_test_abc123",
        scanPubKey: "02" + "aa".repeat(32),
        spendPubKey: "03" + "bb".repeat(32),
        amount: 10000,
      },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

    const obj = await getObject(currentObjectId);
    currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal, synced: sync.indexed };
  });

  // ─── 4. STATE TRANSITION (update_state) ───
  await runTest("4. State transition (update_state)", async () => {
    if (!currentSeal) throw new Error("No active seal");
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "update_state",
      args: { field: "totalSupply", value: "5000000", caller: ADDR },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

    const obj = await getObject(currentObjectId);
    currentSeal = obj.seal;
    return { txid: txid.slice(0, 16), newSeal: currentSeal, synced: sync.indexed };
  });

  // ─── 5. BATCH DEPLOY (2 envelopes in 1 tx) ───
  let batchSeal1 = null, batchSeal2 = null;
  await runTest("5. Batch: 2 deploy envelopes in 1 tx", async () => {
    const wasm = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    const env1 = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const env2 = buildEnvelope("utxovm", 1, "application/wasm", wasm);
    const txid = await broadcastEnvelope([env1, env2], 2000n);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

    // Find both objects from same tx
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const batchObjs = objs.filter(o => o.seal.startsWith(txid));
    if (batchObjs.length >= 2) {
      batchSeal1 = batchObjs[0].seal;
      batchSeal2 = batchObjs[1].seal;
    }
    return { txid: txid.slice(0, 16), objects: batchObjs.length, synced: sync.indexed };
  });

  // ─── 6. TRANSFER ON BATCH OBJECT ───
  await runTest("6. Transfer on batch-deployed object", async () => {
    if (!batchSeal1) throw new Error("No batch object");
    const payload = JSON.stringify({
      targetSeal: batchSeal1,
      method: "transfer",
      args: { to: "03deadbeef00000000000000000000000000000000000000000000000000000000" },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);
    return { txid: txid.slice(0, 16), synced: sync.indexed };
  });

  // ─── 7. TINY PAYLOAD (1 byte) ───
  await runTest("7. Tiny payload (1-byte inscription)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", Buffer.from([0xff]));
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);
    return { txid: txid.slice(0, 16), synced: sync.indexed };
  });

  // ─── 8. LARGE PAYLOAD (>255 bytes) ───
  await runTest("8. Large payload (400 bytes, OP_PUSHDATA2)", async () => {
    const largeData = Buffer.alloc(400, 0xab);
    const env = buildEnvelope("utxovm", 1, "application/wasm", largeData);
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);
    return { txid: txid.slice(0, 16), envelopeSize: env.length, synced: sync.indexed };
  });

  // ─── 9. MEDIUM PAYLOAD (triggers OP_PUSHDATA1) ───
  await runTest("9. Medium payload (100 bytes)", async () => {
    const medData = Buffer.alloc(100, 0xcc);
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", medData);
    const txid = await broadcastEnvelope([env]);
    const sync = await reSync(lastBlock);
    lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);
    return { txid: txid.slice(0, 16), synced: sync.indexed };
  });

  // ─── 10. MULTI-CHAINED STATE TRANSITIONS (3 hops) ───
  let chainSeal = currentSeal;
  for (let hop = 1; hop <= 3; hop++) {
    await runTest(`10.${hop}. Chain transition hop ${hop}`, async () => {
      if (!chainSeal) throw new Error("No chain seal");
      const payload = JSON.stringify({
        targetSeal: chainSeal,
        method: "transfer",
        args: { to: `02hop${hop}0000000000000000000000000000000000000000000000000000000000` },
      });
      const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
      const txid = await broadcastEnvelope([env]);
      const sync = await reSync(lastBlock);
      lastBlock = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text().then(Number);

      const obj = await getObject(currentObjectId);
      chainSeal = obj.seal;
      return { txid: txid.slice(0, 16), hop, newSeal: chainSeal, synced: sync.indexed };
    });
  }

  // ─── 11. PARSE ENVELOPE UTILITY ───
  await runTest("11. Parse envelope utility endpoint", async () => {
    const testScript = "6a4c780063067574786f766d0101106170706c69636174696f6e2f6a736f6e4c597b227461726765745365616c223a2274785f67656e657369733a30222c226d6574686f64223a227472616e73666572222c2261726773223a7b22746f223a2230326361666562616265222c22616d6f756e74223a3530307d7d68";
    const res = await fetch(`${INDEXER}/api/v1/parse-envelope`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ scriptPubKey: testScript }),
    });
    const parsed = await res.json();
    return { protocol: parsed.protocol, contentType: parsed.contentType, hasPayload: !!parsed.payload };
  });

  // ─── 12. GET FULL HISTORY ───
  await runTest("12. Full state transition history", async () => {
    const history = await getHistory(currentObjectId);
    return { totalTransitions: history.length, methods: history.map(h => h.method) };
  });

  // ─── 13. GET ALL OBJECTS ───
  await runTest("13. List all indexed objects", async () => {
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    return { total: objs.length };
  });

  // =====================================================
  // SUMMARY
  // =====================================================
  console.log(`\n\n${"═".repeat(60)}`);
  console.log("FULL E2E TEST RESULTS");
  console.log("═".repeat(60));
  console.log(`${"#".padEnd(3)} ${"Test".padEnd(48)} Status`);
  console.log("─".repeat(60));
  results.forEach((r, i) => {
    const s = r.ok ? "✅" : "❌";
    console.log(`${(i + 1).toString().padEnd(3)} ${r.name.padEnd(48)} ${s}`);
  });
  console.log("─".repeat(60));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  // Verify all via electrs
  console.log("--- Final verification on Electrs ---");
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tipFinal}`);

  const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  console.log(`Indexed objects: ${objs.length}`);

  for (const o of objs) {
    const txid = o.seal.split(":")[0];
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      const confirmed = info.status?.confirmed ? "confirmed" : "mempool";
      const hasOpReturn = info.vout.some(v => v.scriptpubkey_type === "op_return");
      console.log(`  ${o.objectId.slice(0, 22)}... — ${confirmed} — OP_RETURN: ${hasOpReturn}`);
    } catch {}
  }
}

main().catch(console.error);
