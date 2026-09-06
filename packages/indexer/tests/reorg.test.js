const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { MemoryStateDB } = require("../dist/state_db.js");
const { SqliteStateDB } = require("../dist/sqlite_db.js");
const { ChainBlockScanner } = require("../dist/scanner.js");

const TEST_DB_PATH = path.join(__dirname, "test_reorg.db");

function cleanupDb() {
  if (fs.existsSync(TEST_DB_PATH)) {
    try { fs.unlinkSync(TEST_DB_PATH); } catch {}
  }
}

test("SqliteStateDB: Block Headers, Undo Log, and Reorg Rollback", async () => {
  cleanupDb();
  const db = new SqliteStateDB(TEST_DB_PATH);

  try {
    // 1. Block 100: Deploy Token Object
    await db.recordBlockHeader("JKC", 100, "hash_block_100", "hash_block_99");
    const tokenObj = {
      objectId: "obj_token_test",
      codeHash: "hash_code_token",
      seal: "tx_deploy_token:0",
      satoshis: "10000",
      owner: "02owner_alice",
      stateData: { symbol: "TKN", supply: 1000, balances: { alice: 1000 } },
      updatedAtBlock: 100,
    };
    await db.saveObject(tokenObj, "JKC");
    const stateRoot100 = await db.computeAndSaveStateRoot("JKC", 100);
    assert(stateRoot100 && stateRoot100.length === 64, "State root 100 should be valid SHA256 hex");

    // 2. Block 101: Deploy NFT Object & Transfer Token (State Transition)
    await db.recordBlockHeader("JKC", 101, "hash_block_101_forkA", "hash_block_100");
    const nftObj = {
      objectId: "obj_nft_test",
      codeHash: "hash_code_nft",
      seal: "tx_deploy_nft:0",
      satoshis: "5000",
      owner: "02owner_bob",
      stateData: { tokenId: 1, name: "CryptoJunk #1" },
      updatedAtBlock: 101,
    };
    await db.saveObject(nftObj, "JKC");

    // Update Token: Alice sends 200 to Bob
    const updatedToken = {
      ...tokenObj,
      seal: "tx_transfer_token:0",
      stateData: { symbol: "TKN", supply: 1000, balances: { alice: 800, bob: 200 } },
      updatedAtBlock: 101,
    };
    await db.saveObject(updatedToken, "JKC");
    await db.recordTransition("obj_token_test", {
      txid: "tx_transfer_token",
      consumedSeal: "tx_deploy_token:0",
      newSeal: "tx_transfer_token:0",
      method: "transfer",
      events: [{ topic: "Transfer", data: "200 to bob" }],
      blockHeight: 101,
      timestamp: Date.now(),
    });

    const stateRoot101A = await db.computeAndSaveStateRoot("JKC", 101);
    assert.notStrictEqual(stateRoot101A, stateRoot100, "State root must change after block 101 mutations");

    // Verify state before reorg
    const beforeReorgNft = await db.getObject("obj_nft_test");
    assert(beforeReorgNft !== undefined, "NFT should exist before reorg");
    const beforeReorgToken = await db.getObject("obj_token_test");
    assert.strictEqual(beforeReorgToken.seal, "tx_transfer_token:0");
    assert.strictEqual(beforeReorgToken.stateData.balances.alice, 800);

    // 3. TRIGGER REORG: Rollback to Block 100
    const rollbackResult = await db.rollbackToBlock("JKC", 100);
    assert.strictEqual(rollbackResult.rolledBackObjects, 2, "Should rollback 2 object mutations");
    assert.strictEqual(rollbackResult.rolledBackTransitions, 1, "Should rollback 1 transition");

    // 4. Verify post-rollback state
    // NFT deployed at block 101 must be deleted
    const postReorgNft = await db.getObject("obj_nft_test");
    assert.strictEqual(postReorgNft, undefined, "NFT deployed in block 101 must be deleted");

    // Token must be restored to block 100 state and original seal
    const postReorgToken = await db.getObject("obj_token_test");
    assert(postReorgToken !== undefined);
    assert.strictEqual(postReorgToken.seal, "tx_deploy_token:0", "Seal must revert to original");
    assert.strictEqual(postReorgToken.stateData.balances.alice, 1000, "Balance must revert to 1000");

    // Transitions from block 101 must be wiped
    const postTransitions = await db.getTransitions("obj_token_test");
    assert.strictEqual(postTransitions.length, 0, "Transitions from block 101 must be removed");

    // State root computed now must match stateRoot100
    const stateRootRestored = await db.computeAndSaveStateRoot("JKC", 100);
    assert.strictEqual(stateRootRestored, stateRoot100, "State root must match original block 100 root");

    // 5. Apply Fork B at block 101 (Different action: Alice burns 500)
    await db.recordBlockHeader("JKC", 101, "hash_block_101_forkB", "hash_block_100");
    const forkBToken = {
      ...tokenObj,
      seal: "tx_burn_token:0",
      stateData: { symbol: "TKN", supply: 500, balances: { alice: 500 } },
      updatedAtBlock: 101,
    };
    await db.saveObject(forkBToken, "JKC");
    await db.recordTransition("obj_token_test", {
      txid: "tx_burn_token",
      consumedSeal: "tx_deploy_token:0",
      newSeal: "tx_burn_token:0",
      method: "burn",
      events: [{ topic: "Burn", data: "500 burned" }],
      blockHeight: 101,
      timestamp: Date.now(),
    });

    const stateRoot101B = await db.computeAndSaveStateRoot("JKC", 101);
    assert.notStrictEqual(stateRoot101B, stateRoot101A, "Fork B state root must differ from Fork A");
    assert.notStrictEqual(stateRoot101B, stateRoot100, "Fork B state root must differ from Block 100");

    const finalToken = await db.getObject("obj_token_test");
    assert.strictEqual(finalToken.seal, "tx_burn_token:0");
    assert.strictEqual(finalToken.stateData.balances.alice, 500);

  } finally {
    db.close();
    cleanupDb();
  }
});

test("MemoryStateDB: In-Memory Rollback and State Root Verification", async () => {
  const db = new MemoryStateDB();

  await db.recordBlockHeader("JKC", 50, "hash_50");
  await db.saveObject({
    objectId: "obj_mem_1",
    codeHash: "code_1",
    seal: "tx1:0",
    satoshis: "100",
    owner: "alice",
    stateData: { val: 1 },
    updatedAtBlock: 50,
  }, "JKC");

  const root50 = await db.computeAndSaveStateRoot("JKC", 50);

  // Block 51 mutation
  await db.recordBlockHeader("JKC", 51, "hash_51");
  await db.saveObject({
    objectId: "obj_mem_1",
    codeHash: "code_1",
    seal: "tx2:0",
    satoshis: "100",
    owner: "alice",
    stateData: { val: 2 },
    updatedAtBlock: 51,
  }, "JKC");

  const root51 = await db.computeAndSaveStateRoot("JKC", 51);
  assert.notStrictEqual(root50, root51);

  // Rollback to 50
  await db.rollbackToBlock("JKC", 50);

  const restored = await db.getObject("obj_mem_1");
  assert.strictEqual(restored.seal, "tx1:0");
  assert.strictEqual(restored.stateData.val, 1);

  const restoredRoot = await db.computeAndSaveStateRoot("JKC", 50);
  assert.strictEqual(restoredRoot, root50);
});

test("ChainBlockScanner: Mempool Isolation (0-Conf does not overwrite canonical state)", async () => {
  const db = new MemoryStateDB();
  const scanner = new ChainBlockScanner(db, "JKC");

  // Canonical confirmed block 10
  const deployTx = {
    txid: "tx_deploy_01",
    inputs: [{ txid: "prev", vout: 0 }],
    outputs: [{ vout: 0, satoshis: BigInt(1000), scriptPubKeyHex: "5120..." }],
    envelope: {
      protocol: "utxovm",
      version: 1,
      contentType: "application/json",
      payload: [],
      metadata: { initial: true, owner: "alice" },
    },
  };
  await scanner.processBlock(10, [deployTx], "hash_block_10");

  const confirmedObj = await db.getObject("obj_tx_deploy_01");
  assert(confirmedObj !== undefined);
  assert.strictEqual(confirmedObj.seal, "tx_deploy_01:0");

  // Record unconfirmed mempool transaction
  await db.recordMempoolTransition({
    txid: "tx_mempool_call",
    chain: "JKC",
    objectId: confirmedObj.objectId,
    consumedSeal: "tx_deploy_01:0",
    newSeal: "tx_mempool_call:0",
    method: "transfer",
    events: [],
    receivedAt: Date.now(),
  });

  // Ensure canonical state is NOT prematurely overwritten by mempool
  const canonicalAfterMempool = await db.getObject("obj_tx_deploy_01");
  assert.strictEqual(canonicalAfterMempool.seal, "tx_deploy_01:0", "Canonical seal must remain unspent until block confirmation");

  // Verify mempool endpoint records it
  const pending = await db.getMempoolTransitions("JKC");
  assert.strictEqual(pending.length, 1);
  assert.strictEqual(pending[0].txid, "tx_mempool_call");

  // Confirm in block 11
  const confirmCallTx = {
    txid: "tx_mempool_call",
    inputs: [{ txid: "tx_deploy_01", vout: 0 }],
    outputs: [{ vout: 0, satoshis: BigInt(1000), scriptPubKeyHex: "5120..." }],
    envelope: {
      protocol: "utxovm",
      version: 1,
      contentType: "application/json",
      payload: [],
      metadata: { targetSeal: "tx_deploy_01:0", method: "transfer", args: { to: "bob" }, caller: "alice" },
    },
  };
  await scanner.processBlock(11, [confirmCallTx], "hash_block_11");

  // Now canonical seal is updated
  const finalizedObj = await db.getObject("obj_tx_deploy_01");
  assert.strictEqual(finalizedObj.seal, "tx_mempool_call:0");
  assert.strictEqual(finalizedObj.owner, "bob");

  // Mempool is cleared
  const pendingAfterConfirm = await db.getMempoolTransitions("JKC");
  assert.strictEqual(pendingAfterConfirm.length, 0, "Mempool transaction should be cleared on confirmation");
});
