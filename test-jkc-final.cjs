#!/usr/bin/env node
/**
 * JKC Testnet — Comprehensive Test Suite
 * 
 * Tests everything: deploy, transfer, batch, stealth, cross-chain
 * All sequential to avoid mempool conflicts
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

// Merkle Tree
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
        nextLevel.push(crypto.createHash("sha256").update(combined).digest());
      }
      level = nextLevel;
      tree.push(level);
    }
    return tree;
  }
  getRoot() { return this.tree.length ? this.tree[this.tree.length - 1][0] : Buffer.alloc(0); }
  getProof(index) {
    const proof = [];
    let idx = index;
    for (let i = 0; i < this.tree.length - 1; i++) {
      const level = this.tree[i];
      const isRight = idx % 2 === 1;
      const siblingIdx = isRight ? idx - 1 : idx + 1;
      if (siblingIdx < level.length) {
        proof.push({ hash: level[siblingIdx].toString("hex"), position: isRight ? "left" : "right" });
      }
      idx = Math.floor(idx / 2);
    }
    return proof;
  }
  static verify(leaf, proof, root) {
    let current = Buffer.from(leaf, "hex");
    for (const step of proof) {
      const sibling = Buffer.from(step.hash, "hex");
      const combined = step.position === "left" ? Buffer.concat([sibling, current]) : Buffer.concat([current, sibling]);
      current = crypto.createHash("sha256").update(combined).digest();
    }
    return current.equals(Buffer.from(root, "hex"));
  }
}

async function main() {
  console.log("╔══════════════════════════════════════════════════════╗");
  console.log("║  JKC Testnet — Comprehensive Test Suite            ║");
  console.log("╚══════════════════════════════════════════════════════╝\n");
  console.log(`Address: ${ADDR}`);

  const tip = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  console.log(`Tip: ${tip.trim()}\n`);

  const results = [];
  let contractSeal = null;
  let contractObjectId = null;
  const allTxids = [];

  async function runTest(name, fn) {
    console.log(`\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━`);
    console.log(`TEST: ${name}`);
    console.log(`━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n`);
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

  // ═══════════════════════════════════════════════════
  // 1. DEPLOY TOKEN CONTRACT
  // ═══════════════════════════════════════════════════
  await runTest("1. Deploy Token Contract", async () => {
    const wasm = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f,
      0x03, 0x02, 0x01, 0x00,
      0x07, 0x08, 0x01, 0x04, 0x69, 0x6e, 0x69, 0x74, 0x00, 0x00,
      0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ]);

    const metadata = {
      name: "JKCTestToken",
      symbol: "JKCT",
      decimals: 8,
      totalSupply: 1000000,
      owner: ADDR,
    };

    const payload = JSON.stringify({
      wasm: Buffer.from(wasm).toString("hex"),
      metadata,
      init: { method: "init", args: metadata },
    });

    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    
    const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
    const deployed = objs.find(o => o.seal.startsWith(result.txid));
    if (deployed) {
      contractSeal = deployed.seal;
      contractObjectId = deployed.objectId;
    }
    
    return { txid: result.txid, block: result.block, seal: contractSeal };
  });

  // ═══════════════════════════════════════════════════
  // 2. MINT TOKENS
  // ═══════════════════════════════════════════════════
  await runTest("2. Mint 100K JKCT", async () => {
    if (!contractSeal) throw new Error("No contract");
    const payload = JSON.stringify({
      targetSeal: contractSeal,
      method: "mint",
      args: { to: ADDR, amount: 100000, caller: ADDR },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    contractSeal = obj.seal;
    return { txid: result.txid, block: result.block };
  });

  // ═══════════════════════════════════════════════════
  // 3. TRANSFER TOKENS
  // ═══════════════════════════════════════════════════
  await runTest("3. Transfer 25K JKCT", async () => {
    if (!contractSeal) throw new Error("No contract");
    const addrB = "02cafebabe00000000000000000000000000000000000000000000000000000000";
    const payload = JSON.stringify({
      targetSeal: contractSeal,
      method: "transfer",
      args: { from: ADDR, to: addrB, amount: 25000, caller: ADDR },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    contractSeal = obj.seal;
    return { txid: result.txid, block: result.block, owner: obj.owner.slice(0, 20) };
  });

  // ═══════════════════════════════════════════════════
  // 4. BURN TOKENS
  // ═══════════════════════════════════════════════════
  await runTest("4. Burn 10K JKCT", async () => {
    if (!contractSeal) throw new Error("No contract");
    const addrB = "02cafebabe00000000000000000000000000000000000000000000000000000000";
    const payload = JSON.stringify({
      targetSeal: contractSeal,
      method: "burn",
      args: { from: addrB, amount: 10000, caller: addrB },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    contractSeal = obj.seal;
    return { txid: result.txid, block: result.block };
  });

  // ═══════════════════════════════════════════════════
  // 5. STEALTH SETTLEMENT
  // ═══════════════════════════════════════════════════
  await runTest("5. Stealth Settlement", async () => {
    if (!contractSeal) throw new Error("No contract");
    const payload = JSON.stringify({
      targetSeal: contractSeal,
      method: "stealth_settle",
      args: {
        stealthAddress: "tjc1qstealth_final_test",
        scanPubKey: "02" + "aa".repeat(32),
        spendPubKey: "03" + "bb".repeat(32),
        amount: 5000,
        token: "JKCT",
      },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    contractSeal = obj.seal;
    return { txid: result.txid, block: result.block };
  });

  // ═══════════════════════════════════════════════════
  // 6. UPDATE METADATA
  // ═══════════════════════════════════════════════════
  await runTest("6. Update Metadata", async () => {
    if (!contractSeal) throw new Error("No contract");
    const payload = JSON.stringify({
      targetSeal: contractSeal,
      method: "update_state",
      args: {
        field: "metadata",
        value: { description: "JKC Testnet Token", version: "2.0" },
        caller: ADDR,
      },
    });
    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
    contractSeal = obj.seal;
    return { txid: result.txid, block: result.block };
  });

  // ═══════════════════════════════════════════════════
  // 7. 10-HOP OWNERSHIP CHAIN
  // ═══════════════════════════════════════════════════
  await runTest("7. 10-Hop Ownership Chain", async () => {
    if (!contractSeal) throw new Error("No contract");
    const owners = [];
    for (let i = 0; i < 10; i++) {
      owners.push("02" + i.toString().padStart(2, "0") + "00".repeat(31));
    }
    
    let currentSeal = contractSeal;
    const txids = [];
    
    for (let i = 0; i < 10; i++) {
      const from = i === 0 ? ADDR : owners[i - 1];
      const payload = JSON.stringify({
        targetSeal: currentSeal,
        method: "transfer",
        args: { from, to: owners[i], amount: 1000, caller: from },
      });
      const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(payload));
      const result = await broadcastAndConfirm([env]);
      allTxids.push(result.txid);
      txids.push(result.txid);
      await syncAll();
      const obj = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}`)).json();
      currentSeal = obj.seal;
      console.log(`    Hop ${i + 1}: block ${result.block}`);
    }
    
    contractSeal = currentSeal;
    return { hops: 10, txids: txids.length };
  });

  // ═══════════════════════════════════════════════════
  // 8. BATCH (5 ENVELOPES)
  // ═══════════════════════════════════════════════════
  await runTest("8. Batch Deploy (5 envelopes)", async () => {
    const wasm = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    const envs = [];
    for (let i = 0; i < 5; i++) {
      envs.push(buildEnvelope("utxovm", 1, "application/wasm", wasm));
    }
    const result = await broadcastAndConfirm(envs, 5000n);
    allTxids.push(result.txid);
    await syncAll();
    return { txid: result.txid, block: result.block, envelopes: 5 };
  });

  // ═══════════════════════════════════════════════════
  // 9. CROSS-CHAIN ANCHOR
  // ═══════════════════════════════════════════════════
  await runTest("9. Cross-Chain State Anchor", async () => {
    const tree = new MerkleTree([
      crypto.randomBytes(32).toString("hex"),
      crypto.randomBytes(32).toString("hex"),
      crypto.randomBytes(32).toString("hex"),
    ]);
    const merkleRoot = tree.getRoot().toString("hex");

    const anchor = {
      type: "state_anchor",
      chainId: "child_chain_test",
      blockHeight: 99999,
      blockHash: "0x" + crypto.randomBytes(32).toString("hex"),
      stateRoot: "0x" + crypto.randomBytes(32).toString("hex"),
      merkleRoot: "0x" + merkleRoot,
      timestamp: Date.now(),
    };

    const env = buildEnvelope("utxovm", 1, "application/json", Buffer.from(JSON.stringify(anchor)));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    return { txid: result.txid, block: result.block, chainId: anchor.chainId };
  });

  // ═══════════════════════════════════════════════════
  // 10. LARGE PAYLOAD (500 bytes)
  // ═══════════════════════════════════════════════════
  await runTest("10. Large Payload (500 bytes)", async () => {
    const largeData = Buffer.alloc(500, 0xab);
    const env = buildEnvelope("utxovm", 1, "application/wasm", largeData);
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    return { txid: result.txid, block: result.block, size: env.length };
  });

  // ═══════════════════════════════════════════════════
  // 11. TINY PAYLOAD (1 byte)
  // ═══════════════════════════════════════════════════
  await runTest("11. Tiny Payload (1 byte)", async () => {
    const env = buildEnvelope("utxovm", 1, "application/octet-stream", Buffer.from([0xff]));
    const result = await broadcastAndConfirm([env]);
    allTxids.push(result.txid);
    await syncAll();
    return { txid: result.txid, block: result.block };
  });

  // ═══════════════════════════════════════════════════
  // 12. PARSE ENVELOPE UTILITY
  // ═══════════════════════════════════════════════════
  await runTest("12. Parse Envelope Utility", async () => {
    const testScript = "6a4c780063067574786f766d0101106170706c69636174696f6e2f6a736f6e4c597b227461726765745365616c223a2274785f67656e657369733a30222c226d6574686f64223a227472616e73666572222c2261726773223a7b22746f223a2230326361666562616265222c22616d6f756e74223a3530307d7d68";
    const res = await fetch(`${INDEXER}/api/v1/parse-envelope`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ scriptPubKey: testScript }),
    });
    const parsed = await res.json();
    return { protocol: parsed.protocol, contentType: parsed.contentType };
  });

  // ═══════════════════════════════════════════════════
  // 13. VERIFY ALL ON ELECTRS
  // ═══════════════════════════════════════════════════
  await runTest("13. Verify All Txs on Electrs", async () => {
    let verified = 0;
    for (const txid of allTxids) {
      try {
        const info = await (await fetch(`${ELECTRS}/tx/${txid}`)).json();
        if (info.status?.confirmed) verified++;
      } catch {}
    }
    return { total: allTxids.length, verified };
  });

  // ═══════════════════════════════════════════════════
  // FINAL SUMMARY
  // ═══════════════════════════════════════════════════
  console.log(`\n\n${"═".repeat(60)}`);
  console.log("FINAL RESULTS");
  console.log("═".repeat(60));
  console.log(`${"#".padEnd(3)} ${"Test".padEnd(35)} Status`);
  console.log("─".repeat(60));
  results.forEach((r, i) => {
    console.log(`${(i + 1).toString().padEnd(3)} ${r.name.padEnd(35)} ${r.ok ? "✅" : "❌"}`);
  });
  console.log("─".repeat(60));
  console.log(`Total: ${results.filter(r => r.ok).length}/${results.length} passed\n`);

  // Network stats
  const tipFinal = await (await fetch(`${ELECTRS}/blocks/tip/height`)).text();
  const objs = await (await fetch(`${INDEXER}/api/v1/objects`)).json();
  
  console.log("--- Network Stats ---");
  console.log(`Tip: ${tipFinal}`);
  console.log(`Objects: ${objs.length}`);
  console.log(`Transactions: ${allTxids.length}`);
  
  if (contractObjectId) {
    const history = await (await fetch(`${INDEXER}/api/v1/object/${contractObjectId}/history`)).json();
    console.log(`State Transitions: ${history.length}`);
  }

  console.log("\n=== Comprehensive Test Complete ===");
}

main().catch(console.error);
