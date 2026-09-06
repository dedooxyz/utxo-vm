const test = require("node:test");
const assert = require("node:assert");
const { MemoryStateDB } = require("../dist/state_db.js");
const { ChainBlockScanner } = require("../dist/scanner.js");

test("MemoryStateDB: Save, Query by ID and Seal, History", async () => {
  const db = new MemoryStateDB();

  const record = {
    objectId: "obj_test_100",
    codeHash: "hash_test_100",
    seal: "tx_init_100:0",
    satoshis: "25000",
    owner: "02pubkey_alice",
    stateData: { count: 1 },
    updatedAtBlock: 100,
  };

  await db.saveObject(record);

  const byId = await db.getObject("obj_test_100");
  assert.deepStrictEqual(byId, record);

  const bySeal = await db.getObjectBySeal("tx_init_100:0");
  assert.deepStrictEqual(bySeal, record);

  const byOwner = await db.getObjectsByOwner("02pubkey_alice");
  assert.strictEqual(byOwner.length, 1);

  // Record transition
  const transition = {
    txid: "tx_transition_101",
    consumedSeal: "tx_init_100:0",
    newSeal: "tx_transition_101:0",
    method: "increment",
    events: [{ topic: "Incremented", data: "count is now 2" }],
    blockHeight: 101,
    timestamp: Date.now(),
  };

  await db.recordTransition("obj_test_100", transition);

  const oldSealQuery = await db.getObjectBySeal("tx_init_100:0");
  assert.strictEqual(oldSealQuery, undefined);

  const newSealQuery = await db.getObjectBySeal("tx_transition_101:0");
  assert(newSealQuery !== undefined);

  const history = await db.getHistory("obj_test_100");
  assert.strictEqual(history.length, 1);
  assert.strictEqual(history[0].method, "increment");
});

test("ChainBlockScanner: Process Inscription Envelopes", async () => {
  const db = new MemoryStateDB();
  const scanner = new ChainBlockScanner(db, "LTC");

  // 1. Deploy Transaction
  const deployTx = {
    txid: "tx_deploy_ltc_001",
    inputs: [{ txid: "prev", vout: 0 }],
    outputs: [{ vout: 0, satoshis: BigInt(50000), scriptPubKeyHex: "5120..." }],
    envelope: {
      protocol: "utxovm",
      version: 1,
      contentType: "application/wasm",
      payload: [0, 97, 115, 109],
      metadata: { owner: "03ltc_owner" },
    },
  };

  await scanner.processTransaction(deployTx, 500);

  const deployedSeal = "tx_deploy_ltc_001:0";
  const deployedObj = await db.getObjectBySeal(deployedSeal);
  assert(deployedObj !== undefined);
  assert.strictEqual(deployedObj.satoshis, "50000");
  assert.strictEqual(deployedObj.owner, "03ltc_owner");

  // 2. Call Transaction
  const callTx = {
    txid: "tx_call_ltc_002",
    inputs: [{ txid: "tx_deploy_ltc_001", vout: 0 }],
    outputs: [{ vout: 0, satoshis: BigInt(50000), scriptPubKeyHex: "5120..." }],
    envelope: {
      protocol: "utxovm",
      version: 1,
      contentType: "application/json",
      payload: [],
      metadata: {
        targetSeal: deployedSeal,
        method: "transfer",
        args: { to: "03ltc_bob" },
        caller: "03ltc_owner",
      },
    },
  };

  await scanner.processTransaction(callTx, 501);

  const updatedObj = await db.getObject(deployedObj.objectId);
  assert.strictEqual(updatedObj.seal, "tx_call_ltc_002:0");
  assert.strictEqual(updatedObj.owner, "03ltc_bob");
});
