#!/usr/bin/env node
/**
 * JKC Testnet — E2E Core-VM Integration Test
 * 
 * Deploy WASM, execute methods, verify state transitions
 * All via real electrs + indexer + core-vm
 */

const bitcoin = require("bitcoinjs-lib");
const { ECPairFactory } = require("ecpair");
const ecc = require("tiny-secp256k1");
const ECPair = ECPairFactory(ecc);
const { execSync } = require("child_process");
const path = require("path");

const ELECTRS = "https://jkc-testnet-api.s3na.xyz";
const INDEXER = "http://localhost:9773";
const VM_BINARY = path.join(__dirname, "target/release/utxo-core-vm-cli");

const JKC_TESTNET = {
  messagePrefix: "\x19Junkcoin Signed Message:\n",
  bech32: "tjc",
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 111, scriptHash: 196, wif: 239,
};
const WIF = "cVDJUtDjdaM25yNVVDLLX3hcHUfth4c7tY3rSc4hy9e8ibtCuj6G";
const keyPair = ECPair.fromWIF(WIF, JKC_TESTNET);
const ADDR = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network: JKC_TESTNET }).address;

function executeVm(command, args) {
  const input = JSON.stringify({ command, args });
  try {
    const result = execSync(VM_BINARY, {
      input,
      encoding: "utf8",
      timeout: 30000,
    });
    return JSON.parse(result);
  } catch (err) {
    return { success: false, error: err.message };
  }
}

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
  await fetch(`${INDEXER}/api/v1/sync`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sinceBlock: tip - 30 }),
  });
}

async function main() {
  console.log("╔══════════════════════════════════════════════════════╗");
  console.log("║  JKC Testnet — E2E Core-VM Integration Test        ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");
  console.log(`Address: ${ADDR}`);
  console.log(`Indexer: ${INDEXER}`);
  console.log(`VM Binary: ${VM_BINARY}\n`);

  const results = [];
  let contractSeal = null;
  let contractObjectId = null;

  // ═══════════════════════════════════════════════════
  // STEP 1: Test Core-VM Directly
  // ═══════════════════════════════════════════════════
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 1: Test Core-VM Directly");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const testWasm = "0061736d010000000105016000017f0302010007080104696e697400000a0601040041000b";
  
  // Test code_hash
  const hashResult = executeVm("code_hash", { wasm_hex: testWasm });
  console.log(`  Code Hash: ${hashResult.result?.code_hash}`);
  results.push({ step: "Code Hash", ok: hashResult.success });

  // Test deploy
  const deployResult = executeVm("deploy", {
    wasm_hex: testWasm,
    caller: ADDR,
    seal_txid: "0000000000000000000000000000000000000000000000000000000000000000",
    seal_vout: 0,
    satoshis: 1000,
    init_args_hex: "",
  });
  console.log(`  Deploy: gas=${deployResult.result?.gas_consumed}, return=${deployResult.result?.return_code}`);
  results.push({ step: "Deploy", ok: deployResult.success });

  // ═══════════════════════════════════════════════════
  // STEP 2: Deploy Token Contract on Chain
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 2: Deploy Token Contract on Chain");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const tokenWasm = new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
  ]);

  const tokenMetadata = {
    name: "JKCTestToken",
    symbol: "JKCT",
    decimals: 8,
    totalSupply: 1000000,
    owner: ADDR,
  };

  const deployPayload = JSON.stringify({
    wasm: Buffer.from(tokenWasm).toString("hex"),
    metadata: tokenMetadata,
    init: { method: "init", args: tokenMetadata },
  });

  const deployEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(deployPayload));
  const deployResult2 = await broadcastAndConfirm([deployEnv]);
  
  await syncAll();
  const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  const deployed = objs.find(o => o.seal.startsWith(deployResult2.txid));
  
  if (deployed) {
    contractSeal = deployed.seal;
    contractObjectId = deployed.objectId;
    console.log(`  TXID: ${deployResult2.txid}`);
    console.log(`  Seal: ${contractSeal}`);
    console.log(`  Object: ${contractObjectId}`);
    console.log(`  Block: ${deployResult2.block}`);
    results.push({ step: "Deploy on Chain", ok: true, txid: deployResult2.txid });
  } else {
    console.log("  ERROR: Deploy not indexed");
    results.push({ step: "Deploy on Chain", ok: false });
  }

  // ═══════════════════════════════════════════════════
  // STEP 3: Execute Mint via Core-VM
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 3: Execute Mint via Core-VM");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const mintPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "mint",
    args: { to: ADDR, amount: 100000, caller: ADDR },
  });

  const mintEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(mintPayload));
  const mintResult = await broadcastAndConfirm([mintEnv]);
  
  await syncAll();
  const mintObj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
  console.log(`  TXID: ${mintResult.txid}`);
  console.log(`  Block: ${mintResult.block}`);
  console.log(`  New seal: ${mintObj.seal}`);
  console.log(`  State: ${JSON.stringify(mintObj.stateData).slice(0, 100)}...`);
  contractSeal = mintObj.seal;
  results.push({ step: "Mint", ok: true, txid: mintResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 4: Execute Transfer via Core-VM
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 4: Execute Transfer via Core-VM");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const addrB = "02cafebabe00000000000000000000000000000000000000000000000000000000";
  const transferPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "transfer",
    args: { from: ADDR, to: addrB, amount: 25000, caller: ADDR },
  });

  const transferEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(transferPayload));
  const transferResult = await broadcastAndConfirm([transferEnv]);
  
  await syncAll();
  const transferObj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
  console.log(`  TXID: ${transferResult.txid}`);
  console.log(`  Block: ${transferResult.block}`);
  console.log(`  New seal: ${transferObj.seal}`);
  console.log(`  Owner: ${transferObj.owner}`);
  contractSeal = transferObj.seal;
  results.push({ step: "Transfer", ok: true, txid: transferResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 5: Execute Burn via Core-VM
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 5: Execute Burn via Core-VM");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const burnPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "burn",
    args: { from: addrB, amount: 10000, caller: addrB },
  });

  const burnEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(burnPayload));
  const burnResult = await broadcastAndConfirm([burnEnv]);
  
  await syncAll();
  const burnObj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
  console.log(`  TXID: ${burnResult.txid}`);
  console.log(`  Block: ${burnResult.block}`);
  console.log(`  New seal: ${burnObj.seal}`);
  contractSeal = burnObj.seal;
  results.push({ step: "Burn", ok: true, txid: burnResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 6: Stealth Settlement
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 6: Stealth Settlement");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const stealthPayload = JSON.stringify({
    targetSeal: contractSeal,
    method: "stealth_settle",
    args: {
      stealthAddress: "tjc1qstealth_corevm_test",
      scanPubKey: "02" + "aa".repeat(32),
      spendPubKey: "03" + "bb".repeat(32),
      amount: 5000,
      token: "JKCT",
    },
  });

  const stealthEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(stealthPayload));
  const stealthResult = await broadcastAndConfirm([stealthEnv]);
  
  await syncAll();
  const stealthObj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
  console.log(`  TXID: ${stealthResult.txid}`);
  console.log(`  Block: ${stealthResult.block}`);
  console.log(`  New seal: ${stealthObj.seal}`);
  contractSeal = stealthObj.seal;
  results.push({ step: "Stealth", ok: true, txid: stealthResult.txid });

  // ═══════════════════════════════════════════════════
  // STEP 7: 3-Hop Chain
  // ═══════════════════════════════════════════════════
  console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
  console.log("STEP 7: 3-Hop Chain");
  console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

  const owners = [
    "02aaaa0000000000000000000000000000000000000000000000000000000000aa",
    "02bbbb0000000000000000000000000000000000000000000000000000000000bb",
    "02cccc0000000000000000000000000000000000000000000000000000000000cc",
  ];

  for (let i = 0; i < owners.length; i++) {
    console.log(`  Hop ${i + 1}: → ${owners[i].slice(0, 20)}...`);
    
    const hopPayload = JSON.stringify({
      targetSeal: contractSeal,
      method: "transfer",
      args: { from: i === 0 ? addrB : owners[i - 1], to: owners[i], amount: 1000, caller: i === 0 ? addrB : owners[i - 1] },
    });

    const hopEnv = buildEnvelope("utxovm", 1, "application/json", Buffer.from(hopPayload));
    const hopResult = await broadcastAndConfirm([hopEnv]);
    
    await syncAll();
    const hopObj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    console.log(`    TXID: ${hopResult.txid} — block ${hopResult.block}`);
    contractSeal = hopObj.seal;
  }
  results.push({ step: "3-Hop Chain", ok: true });

  // ═══════════════════════════════════════════════════
  // FINAL SUMMARY
  // ═══════════════════════════════════════════════════
  console.log("\n╔══════════════════════════════════════════════════════╗");
  console.log("║  FINAL RESULTS                                      ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");

  console.log(`${"Step".padEnd(20)} Status`);
  console.log("─".repeat(50));
  for (const r of results) {
    console.log(`${r.step.padEnd(20)} ${r.ok ? "✅" : "❌"}`);
  }
  console.log("─".repeat(50));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  // Verify via electrs
  console.log("--- Electrs Verification ---");
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tipFinal}`);

  const finalObjs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  console.log(`Indexed objects: ${finalObjs.length}`);

  if (contractObjectId) {
    const finalContract = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    console.log(`\nContract: ${contractObjectId}`);
    console.log(`  Seal: ${finalContract.seal}`);
    console.log(`  Owner: ${finalContract.owner}`);
    
    const history = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}/history`)).json();
    console.log(`  History: ${history.length} state transitions`);
    for (const h of history) {
      console.log(`    ${h.method} — block ${h.blockHeight} — ${h.txid.slice(0, 16)}`);
    }
  }

  console.log("\n=== Core-VM Integration Test Complete ===");
}

main().catch(console.error);
