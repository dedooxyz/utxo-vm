#!/usr/bin/env node
/**
 * JKC Testnet — Wait for confirmation + chain test continuation
 * Polls electrs for block confirmation, then runs chained state transitions
 */

const ELECTRS = "https://jkc-testnet-api.s3na.xyz";
const INDEXER = "http://localhost:9773";

const NEW_TXS = [
  { name: "Deploy WASM", txid: "a883ad7cb99ba50c6c6e343f3eb2e24cd5d4a94b139d5d02efffdf4ac9be5193" },
  { name: "Batch Deploy", txid: "21f3863350ecda7378a4998c896aa3447bb69c8dbd8f032a197756012a2aeb68" },
  { name: "Tiny Payload", txid: "6b47a211424a2454b2ceffad00b4774a38b3bab24cdea885258b5e76d91f8e43" },
  { name: "Large Payload", txid: "d6cd723e1291652f5ecae5587d133837e15dfd774c178c68a18c16a967bb1a46" },
  { name: "Medium Payload", txid: "0319dcec1610615061f6a7bfe46a42b6fe832a2aca12976c9a70b5efa87d6713" },
];

async function isConfirmed(txid) {
  try {
    const r = await fetch(`${ELECTRS}/tx/${txid}`);
    const d = await r.json();
    return d.status?.confirmed || false;
  } catch { return false; }
}

async function syncAll() {
  const tip = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  const tipHeight = parseInt(tip.trim(), 10);
  const r = await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tipHeight - 20 }),
  });
  return r.json();
}

async function main() {
  console.log("=== Waiting for tx confirmation ===\n");

  let waitCount = 0;
  while (true) {
    const results = await Promise.all(NEW_TXS.map(async t => ({
      ...t,
      confirmed: await isConfirmed(t.txid),
    })));

    const confirmed = results.filter(r => r.confirmed).length;
    console.log(`  [${++waitCount}] ${confirmed}/${NEW_TXS.length} confirmed`);

    if (confirmed === NEW_TXS.length) break;
    await new Promise(r => setTimeout(r, 15000));
  }

  console.log("\n=== All confirmed! Syncing indexer ===\n");
  const sync = await syncAll();
  console.log(`  Synced: ${JSON.stringify(sync)}`);

  // List all objects
  const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  console.log(`  Objects: ${objs.length}`);
  for (const o of objs) {
    console.log(`    ${o.objectId} — seal: ${o.seal.slice(0, 30)}... — block: ${o.updatedAtBlock}`);
  }

  // Find the new deploy's seal
  const deploy = objs.find(o => o.seal.startsWith(NEW_TXS[0].txid));
  if (!deploy) {
    console.error("Deploy object not found after sync!");
    return;
  }

  const WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";
  const { buildEnvelope, broadcastEnvelope, getUtxo } = require("./test-jkc-opcodes-full.cjs").__exports || {};

  // Use inline helpers
  const bitcoin = require("bitcoinjs-lib");
  const { ECPairFactory } = require("ecpair");
  const ecc = require("tiny-secp256k1");
  const ECPair = ECPairFactory(ecc);
  const JKC_TESTNET = {
    messagePrefix: "\x19Junkcoin Signed Message:\n",
    bech32: "tjc",
    bip32: { public: 0x043587cf, private: 0x04358394 },
    pubKeyHash: 111, scriptHash: 196, wif: 239,
  };
  const keyPair = ECPair.fromWIF(WIF, JKC_TESTNET);
  const ADDR = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network: JKC_TESTNET }).address;

  function buildEnv(protocol, version, contentType, payload) {
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

  async function broadcast(envs, fee = 1000n) {
    const utxos = await (await fetch(`${ELECTRS}/address/${ADDR}/utxo`)).json();
    if (!utxos.length) throw new Error("No UTXOs");
    const utxo = utxos[0];
    const prevHex = await (await fetch(`${ELECTRS}/tx/${utxo.txid}/hex`)).text();
    const psbt = new bitcoin.Psbt({ network: JKC_TESTNET });
    psbt.addInput({ hash: utxo.txid, index: utxo.vout, nonWitnessUtxo: Buffer.from(prevHex, "hex") });
    for (const env of envs) {
      const script = bitcoin.script.compile([bitcoin.opcodes.OP_RETURN, env]);
      psbt.addOutput({ script, value: 0n });
    }
    psbt.addOutput({ address: ADDR, value: BigInt(utxo.value) - fee });
    psbt.signInput(0, keyPair);
    psbt.finalizeAllInputs();
    const txHex = psbt.extractTransaction().toHex();
    const res = await fetch(`${ELECTRS}/tx`, { method: "POST", headers: { "Content-Type": "text/plain" }, body: txHex });
    return (await res.text()).trim();
  }

  // ─── CHAIN TESTS ───
  console.log("\n=== CHAIN TESTS ===\n");

  const chainResults = [];
  let currentSeal = deploy.seal;

  // Test 1: Transfer call on deployed WASM
  console.log("TEST 1: Transfer call on deployed WASM");
  {
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "transfer",
      args: { to: "02cafebabe00000000000000000000000000000000000000000000000000000000", amount: 500 },
    });
    const env = buildEnv("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcast([env]);
    chainResults.push({ name: "Transfer", txid: txid.slice(0, 16), status: "broadcast" });
    console.log(`  TXID: ${txid.slice(0, 16)}...`);
  }

  // Test 2: Stealth settlement
  console.log("TEST 2: Stealth settlement");
  {
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "stealth_settle",
      args: { stealthAddress: "tjc1qstealth_e2e_test", scanKey: "02" + "aa".repeat(32), amount: 10000 },
    });
    const env = buildEnv("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcast([env]);
    chainResults.push({ name: "Stealth", txid: txid.slice(0, 16), status: "broadcast" });
    console.log(`  TXID: ${txid.slice(0, 16)}...`);
  }

  // Test 3: State update
  console.log("TEST 3: State update (update_state)");
  {
    const payload = JSON.stringify({
      targetSeal: currentSeal,
      method: "update_state",
      args: { field: "totalSupply", value: "1000000", caller: ADDR },
    });
    const env = buildEnv("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcast([env]);
    chainResults.push({ name: "StateUpdate", txid: txid.slice(0, 16), status: "broadcast" });
    console.log(`  TXID: ${txid.slice(0, 16)}...`);
  }

  // Test 4: 3-hop chain
  let chainSeal = currentSeal;
  for (let hop = 1; hop <= 3; hop++) {
    console.log(`TEST 4.${hop}: Chain hop ${hop}`);
    const payload = JSON.stringify({
      targetSeal: chainSeal,
      method: "transfer",
      args: { to: `02hop${hop}0000000000000000000000000000000000000000000000000000000000` },
    });
    const env = buildEnv("utxovm", 1, "application/json", Buffer.from(payload));
    const txid = await broadcast([env]);
    chainResults.push({ name: `Hop${hop}`, txid: txid.slice(0, 16), status: "broadcast" });
    console.log(`  TXID: ${txid.slice(0, 16)}...`);
  }

  // Wait for all chain txs to confirm
  console.log("\n=== Waiting for chain txs to confirm ===\n");
  let w = 0;
  while (true) {
    const allConfirmed = await Promise.all(chainResults.map(async r => {
      try {
        const d = await (await fetch(`${ELECTRS}/tx/${r.txid}...`)).json();
        return d.status?.confirmed;
      } catch { return false; }
    }));
    const conf = allConfirmed.filter(Boolean).length;
    console.log(`  [${++w}] ${conf}/${chainResults.length} confirmed`);
    if (conf === chainResults.length) break;
    await new Promise(r => setTimeout(r, 15000));
  }

  // Final sync
  console.log("\n=== Final sync ===\n");
  const tipH = parseInt(await (await fetch(`${ELECTRS}/blocks/tip/height`)).text(), 10);
  const sync2 = await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tipH - 30 }),
  }).then(r => r.json());
  console.log(`  ${JSON.stringify(sync2)}`);

  // Verify objects
  const finalObjs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  console.log(`\n  Objects: ${finalObjs.length}`);
  for (const o of finalObjs) {
    console.log(`    ${o.objectId.slice(0, 22)} — owner: ${o.owner.slice(0, 20)}... — block: ${o.updatedAtBlock}`);
  }

  // Verify via electrs
  console.log("\n--- Electrs verification ---");
  for (const t of NEW_TXS) {
    const txid = t.txid;
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      const hasOpReturn = info.vout.some(v => v.scriptpubkey_type === "op_return");
      console.log(`  ${t.name}: confirmed=${info.status?.confirmed} OP_RETURN=${hasOpReturn}`);
    } catch { console.log(`  ${t.name}: not found`); }
  }

  console.log("\n=== DONE ===");
}

main().catch(console.error);
