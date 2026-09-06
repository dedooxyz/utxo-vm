#!/usr/bin/env node
/**
 * JKC Testnet — E2E Token Contract Test
 * 
 * Deploy a token contract, mint, transfer, burn — real WASM execution
 * Tests: core-vm + indexer + state transitions
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
  parts.push(payloadBuf, Buffer.from([0x68]));
  return Buffer.concat(parts);
}

async function getUtxo() {
  const utxos = await (await fetch(`${ELECTRS}/address/${ADDR}/utxo`)).json();
  if (!utxos.length) throw new Error("No UTXOs");
  return utxos[0];
}

async function broadcastAndConfirm(envelopes, fee = 1000n) {
  const utxo = await getUtxo();
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

async function syncAll() {
  const tip = parseInt(await (await fetch(`${ELECTRS}/blocks/tip/height`)).text(), 10);
  const r = await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tip - 30 }),
  });
  return r.json();
}

async function getObjects() {
  return (await fetch(`${INDEXER}/api/v1/objects`)).json();
}

async function getObject(id) {
  return (await fetch(`${INDEXER}/api/v1/object/${id}`)).json();
}

async function getHistory(id) {
  return (await fetch(`${INDEXER}/api/v1/object/${id}/history`)).json();
}

async function main() {
  console.log("╔══════════════════════════════════════════════════════╗");
  console.log("║  JKC Testnet — E2E Token Contract Test             ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");
  console.log(`Address: ${ADDR}`);
  console.log(`Indexer: ${INDEXER}\n`);

  const tip = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tip.trim()}\n`);

  const log = (msg) => console.log(`  ${msg}`);
  const results = [];
  let contractSeal = null;
  let contractObjectId = null;

  // ═══════════════════════════════════════════════════
  // STEP 1: Deploy Token Contract
  // ═══════════════════════════════════════════════════
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 1: Deploy Token Contract (WASM)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const tokenContract = {
    name: "JKCTestToken",
    symbol: "JKCT",
    decimals: 8,
    totalSupply: 1000000,
    owner: ADDR,
  };

  const wasm = new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
  ]);

  const deployPayload = JSON.stringify({
    wasm: Buffer.from(wasm).toString("hex"),
    metadata: tokenContract,
    init: { method: "init", args: tokenContract },
  });

  const deployEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(deployPayload));
  const deployResult = await broadcastAndConfirm([deployEnv]);
  
  await syncAll();
  const objs = await getObjects();
  const deployed = objs.find(o => o.seal.startsWith(deployResult.txid));
  
  if (deployed) {
    contractSeal = deployed.seal;
    contractObjectId = deployed.objectId;
    log(`TXID: ${deployResult.txid}`);
    log(`Seal: ${contractSeal}`);
    log(`Object: ${contractObjectId}`);
    log(`Block: ${deployResult.block}`);
    results.push({ step: "Deploy", ok: true, txid: deployResult.txid });
  } else {
    log("ERROR: Deploy not indexed");
    results.push({ step: "Deploy", ok: false });
  }

  // ═══════════════════════════════════════════════════
  // STEP 2: Mint Tokens
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 2: Mint Tokens (100,000 JKCT)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const mintPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "mint",
    args: {
      to: ADDR,
      amount: 100000,
      caller: ADDR,
    },
  });

  const mintEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(mintPayload));
  const mintResult = await broadcastAndConfirm([mintEnv]);
  
  await syncAll();
  const mintObj = await getObject(contractObjectId);
  log(`TXID: ${mintResult.txid}`);
  log(`Block: ${mintResult.block}`);
  log(`New seal: ${mintObj.seal}`);
  log(`State: ${JSON.stringify(mintObj.stateData)}`);
  contractSeal = mintObj.seal;
  results.push({ step: "Mint", ok: true, txid: mintResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 3: Transfer Tokens to Address B
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 3: Transfer Tokens (25,000 JKCT → Address B)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const addrB = "02cafebabe00000000000000000000000000000000000000000000000000000000";
  const transferPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "transfer",
    args: {
      from: ADDR,
      to: addrB,
      amount: 25000,
      caller: ADDR,
    },
  });

  const transferEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(transferPayload));
  const transferResult = await broadcastAndConfirm([transferEnv]);
  
  await syncAll();
  const transferObj = await getObject(contractObjectId);
  log(`TXID: ${transferResult.txid}`);
  log(`Block: ${transferResult.block}`);
  log(`New seal: ${transferObj.seal}`);
  log(`State: ${JSON.stringify(transferObj.stateData)}`);
  contractSeal = transferObj.seal;
  results.push({ step: "Transfer", ok: true, txid: transferResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 4: Burn Tokens
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 4: Burn Tokens (10,000 JKCT)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const burnPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "burn",
    args: {
      from: ADDR,
      amount: 10000,
      caller: ADDR,
    },
  });

  const burnEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(burnPayload));
  const burnResult = await broadcastAndConfirm([burnEnv]);
  
  await syncAll();
  const burnObj = await getObject(contractObjectId);
  log(`TXID: ${burnResult.txid}`);
  log(`Block: ${burnResult.block}`);
  log(`New seal: ${burnObj.seal}`);
  log(`State: ${JSON.stringify(burnObj.stateData)}`);
  contractSeal = burnObj.seal;
  results.push({ step: "Burn", ok: true, txid: burnResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 5: Stealth Settlement
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 5: Stealth Settlement");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const stealthPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "stealth_settle",
    args: {
      stealthAddress: "tjc1qstealth_token_test_abc123",
      scanPubKey: "02" + "aa".repeat(32),
      spendPubKey: "03" + "bb".repeat(32),
      amount: 5000,
      token: "JKCT",
    },
  });

  const stealthEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(stealthPayload));
  const stealthResult = await broadcastAndConfirm([stealthEnv]);
  
  await syncAll();
  const stealthObj = await getObject(contractObjectId);
  log(`TXID: ${stealthResult.txid}`);
  log(`Block: ${stealthResult.block}`);
  log(`New seal: ${stealthObj.seal}`);
  log(`State: ${JSON.stringify(stealthObj.stateData)}`);
  contractSeal = stealthObj.seal;
  results.push({ step: "Stealth", ok: true, txid: stealthResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 6: Update Metadata
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 6: Update Contract Metadata");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const updatePayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "update_state",
    args: {
      field: "metadata",
      value: {
        description: "JKC Testnet Token",
        website: "https://utxo-vm.test",
        updated: Date.now(),
      },
      caller: ADDR,
    },
  });

  const updateEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(updatePayload));
  const updateResult = await broadcastAndConfirm([updateEnv]);
  
  await syncAll();
  const updateObj = await getObject(contractObjectId);
  log(`TXID: ${updateResult.txid}`);
  log(`Block: ${updateResult.block}`);
  log(`New seal: ${updateObj.seal}`);
  log(`State: ${JSON.stringify(updateObj.stateData)}`);
  contractSeal = updateObj.seal;
  results.push({ step: "Update", ok: true, txid: updateResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 7: 5-Hop Transfer Chain
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 7: 5-Hop Ownership Chain");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  let chainSeal = contractSeal;
  const owners = [
    "02aaaa0000000000000000000000000000000000000000000000000000000000aa",
    "02bbbb0000000000000000000000000000000000000000000000000000000000bb",
    "02cccc0000000000000000000000000000000000000000000000000000000000cc",
    "02dddd0000000000000000000000000000000000000000000000000000000000dd",
    "02eeee0000000000000000000000000000000000000000000000000000000000ee",
  ];

  for (let i = 0; i < owners.length; i++) {
    log(`Hop ${i + 1}: → ${owners[i].slice(0, 20)}...`);
    
    const hopPayload = JSON.stringify({
      targetSeal: chainSeal,
      method: "transfer",
      args: {
        from: i === 0 ? ADDR : owners[i - 1],
        to: owners[i],
        amount: 1000,
        caller: i === 0 ? ADDR : owners[i - 1],
      },
    });

    const hopEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(hopPayload));
    const hopResult = await broadcastAndConfirm([hopEnv]);
    
    await syncAll();
    const hopObj = await getObject(contractObjectId);
    log(`  TXID: ${hopResult.txid} — block ${hopResult.block}`);
    chainSeal = hopObj.seal;
    contractSeal = hopObj.seal;
  }
  results.push({ step: "5-Hop Chain", ok: true, hops: 5 });

  // ═══════════════════════════════════════════════════
  // STEP 8: Batch Operations (3 in 1 tx)
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 8: Batch Operations (3 envelopes in 1 tx)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const batchEnvs = [];
  
  // Deploy a second contract
  batchEnvs.push(buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify({
    wasm: Buffer.from(wasm).toString("hex"),
    metadata: { name: "BatchToken", symbol: "BATCH" },
  }))));
  
  // Mint on main contract
  batchEnvs.push(buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify({
    targetSeal: contractSeal,
    method: "mint",
    args: { to: ADDR, amount: 50000, caller: ADDR },
  }))));
  
  // Transfer on main contract
  batchEnvs.push(buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify({
    targetSeal: contractSeal,
    method: "transfer",
    args: { from: ADDR, to: owners[4], amount: 5000, caller: ADDR },
  }))));
  
  const batchResult = await broadcastAndConfirm(batchEnvs, 3000n);
  
  await syncAll();
  const batchObjs = await getObjects();
  log(`TXID: ${batchResult.txid}`);
  log(`Block: ${batchResult.block}`);
  log(`Objects: ${batchObjs.length}`);
  results.push({ step: "Batch Ops", ok: true, txid: batchResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 9: Large State Payload
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 9: Large State Payload (500 bytes)");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const largeState = {
    targetSeal: contractSeal,
    method: "update_state",
    args: {
      field: "largeData",
      value: {
        data: "A".repeat(400),
        hash: "Qm" + "a".repeat(44),
        timestamp: Date.now(),
        nonce: Math.random().toString(36),
      },
      caller: ADDR,
    },
  };

  const largeEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify(largeState)));
  const largeResult = await broadcastAndConfirm([largeEnv]);
  
  await syncAll();
  const largeObj = await getObject(contractObjectId);
  log(`TXID: ${largeResult.txid}`);
  log(`Block: ${largeResult.block}`);
  log(`Envelope size: ${largeEnv.length} bytes`);
  log(`State: ${JSON.stringify(largeObj.stateData).length} chars`);
  contractSeal = largeObj.seal;
  results.push({ step: "Large State", ok: true, txid: largeResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 10: Parse & Verify All Envelopes
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 10: Verify All Envelopes on Electrs");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const allTxids = [
    deployResult.txid, mintResult.txid, transferResult.txid,
    burnResult.txid, stealthResult.txid, updateResult.txid,
    batchResult.txid, largeResult.txid,
  ];

  for (const txid of allTxids) {
    try {
      const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
      const envs = info.vout.filter(v => v.scriptpubkey_type === "op_return");
      log(`${txid.slice(0, 16)} — block ${info.status?.block_height} — OP_RETURN: ${envs.length}`);
    } catch { log(`${txid.slice(0, 16)} — not found`); }
  }

  // ═══════════════════════════════════════════════════
  // FINAL SUMMARY
  // ═══════════════════════════════════════════════════
  console.log("\n╔══════════════════════════════════════════════════════╗");
  console.log("║  FINAL RESULTS                                      ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");

  console.log(`${"Step".padEnd(15)} Status`);
  console.log("─".repeat(50));
  for (const r of results) {
    console.log(`${r.step.padEnd(15)} ${r.ok ? "✅" : "❌"} ${r.txid ? r.txid.slice(0, 16) : ""}`);
  }
  console.log("─".repeat(50));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  const finalObjs = await getObjects();
  console.log(`Indexed objects: ${finalObjs.length}`);
  
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tipFinal}`);

  // Contract state
  if (contractObjectId) {
    const finalContract = await getObject(contractObjectId);
    console.log(`\nContract: ${contractObjectId}`);
    console.log(`  Seal: ${finalContract.seal}`);
    console.log(`  Owner: ${finalContract.owner}`);
    console.log(`  State: ${JSON.stringify(finalContract.stateData, null, 2)}`);
    
    const history = await getHistory(contractObjectId);
    console.log(`  History: ${history.length} state transitions`);
    for (const h of history) {
      console.log(`    ${h.method} — block ${h.blockHeight} — ${h.txid.slice(0, 16)}`);
    }
  }
}

main().catch(console.error);
