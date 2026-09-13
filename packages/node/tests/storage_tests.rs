use utxo_vmd::storage::StateStore;
use utxo_vmd::types::{BlockRecord, SmartObjectRecord, UndoLogRecord};

#[test]
fn test_storage_crud_and_proof() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_store.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");

    let obj = SmartObjectRecord {
        object_id: "obj_token_test".to_string(),
        code_hash: "wasm_code_hash_123".to_string(),
        seal: "tx_abc123:0".to_string(),
        satoshis: 50_000,
        owner: "address_owner_1".to_string(),
        state_data: serde_json::json!({
            "ticker": "UTX20",
            "supply": 1000000
        }),
        created_at_block: 100,
        updated_at_block: 100,
    };

    store.save_object(&obj).expect("Save object failed");

    // Query by ID
    let fetched = store.get_object("obj_token_test").expect("Query failed");
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().satoshis, 50_000);

    // Query by Seal
    let by_seal = store.get_object_by_seal("tx_abc123:0").expect("Query by seal failed");
    assert!(by_seal.is_some());
    assert_eq!(by_seal.unwrap().object_id, "obj_token_test");

    // Merkle Proof
    let proof = store.get_state_proof("obj_token_test").expect("Proof failed");
    assert!(proof.verified);
    assert_eq!(proof.root_hex, store.current_state_root());
}

#[test]
fn test_storage_reorg_rollback() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_reorg.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");

    // Object created at block 10
    let mut obj = SmartObjectRecord {
        object_id: "obj_reorg".to_string(),
        code_hash: "wasm_code".to_string(),
        seal: "tx_first:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({ "balance": 100 }),
        created_at_block: 10,
        updated_at_block: 10,
    };
    store.save_object(&obj).unwrap();

    // Undo log for update at block 11
    let undo = UndoLogRecord {
        id: 1,
        chain: "JKC".to_string(),
        block_height: 11,
        object_id: "obj_reorg".to_string(),
        action: "UPDATE".to_string(),
        prev_code_hash: Some("wasm_code".to_string()),
        prev_seal: Some("tx_first:0".to_string()),
        prev_satoshis: Some(1000),
        prev_owner: Some("alice".to_string()),
        prev_state_data: Some(serde_json::json!({ "balance": 100 })),
        prev_updated_at_block: Some(10),
        prev_created_at_block: Some(5),
    };
    store.save_undo_log(&undo).unwrap();

    // Mutate object at block 11
    obj.seal = "tx_second:0".to_string();
    obj.satoshis = 2000;
    obj.updated_at_block = 11;
    obj.state_data = serde_json::json!({ "balance": 200 });
    store.save_object(&obj).unwrap();

    assert_eq!(store.get_object("obj_reorg").unwrap().unwrap().satoshis, 2000);

    // Save block records at height 10 and 12 (12 has NO undo log / contract mutations)
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 10,
        block_hash: "hash_10".to_string(),
        prev_hash: None,
        state_root: "root_10".to_string(),
        timestamp: 1000,
    }).unwrap();
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 12,
        block_hash: "hash_12".to_string(),
        prev_hash: None,
        state_root: "root_12".to_string(),
        timestamp: 1200,
    }).unwrap();

    // Rollback to block 10
    let rolled = store.rollback_to_block("JKC", 10).unwrap();
    assert_eq!(rolled, 1);

    let restored = store.get_object("obj_reorg").unwrap().unwrap();
    assert_eq!(restored.satoshis, 1000);
    assert_eq!(restored.seal, "tx_first:0");

    // Block 10 must still exist; block 12 (no undo log) must be cleaned up
    assert!(store.get_block("JKC", 10).unwrap().is_some());
    assert!(store.get_block("JKC", 12).unwrap().is_none());
}

#[test]
fn test_multi_block_reorg_rollback() {
    // Follow-up to Issues 12-16: dedicated multi-block reorg test.
    // Simulates a 3-block reorg (blocks 11, 12, 13 all rolled back to 10)
    // with multiple object mutations across those blocks, including a block
    // with no contract mutations (no undo log) to verify the block cleanup
    // fix in rollback_to_block.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_multi_reorg.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");

    // --- Block 10: create object ---
    let mut obj = SmartObjectRecord {
        object_id: "obj_multi".to_string(),
        code_hash: "wasm_code".to_string(),
        seal: "tx_10:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({ "balance": 100 }),
        created_at_block: 10,
        updated_at_block: 10,
    };
    store.save_object(&obj).unwrap();
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 10,
        block_hash: "hash_10".to_string(),
        prev_hash: None,
        state_root: "root_10".to_string(),
        timestamp: 1000,
    }).unwrap();
    store.set_last_sync_block("JKC", 10).unwrap();

    // --- Block 11: update object (balance 100 -> 200) ---
    store.save_undo_log(&UndoLogRecord {
        id: 1,
        chain: "JKC".to_string(),
        block_height: 11,
        object_id: "obj_multi".to_string(),
        action: "UPDATE".to_string(),
        prev_code_hash: Some("wasm_code".to_string()),
        prev_seal: Some("tx_10:0".to_string()),
        prev_satoshis: Some(1000),
        prev_owner: Some("alice".to_string()),
        prev_state_data: Some(serde_json::json!({ "balance": 100 })),
        prev_updated_at_block: Some(10),
        prev_created_at_block: Some(10),
    }).unwrap();
    obj.seal = "tx_11:0".to_string();
    obj.satoshis = 1100;
    obj.updated_at_block = 11;
    obj.state_data = serde_json::json!({ "balance": 200 });
    store.save_object(&obj).unwrap();
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 11,
        block_hash: "hash_11".to_string(),
        prev_hash: Some("hash_10".to_string()),
        state_root: "root_11".to_string(),
        timestamp: 1100,
    }).unwrap();

    // --- Block 12: no contract mutations (empty block, no undo log) ---
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 12,
        block_hash: "hash_12".to_string(),
        prev_hash: Some("hash_11".to_string()),
        state_root: "root_12".to_string(),
        timestamp: 1200,
    }).unwrap();

    // --- Block 13: update object again (balance 200 -> 300) ---
    store.save_undo_log(&UndoLogRecord {
        id: 2,
        chain: "JKC".to_string(),
        block_height: 13,
        object_id: "obj_multi".to_string(),
        action: "UPDATE".to_string(),
        prev_code_hash: Some("wasm_code".to_string()),
        prev_seal: Some("tx_11:0".to_string()),
        prev_satoshis: Some(1100),
        prev_owner: Some("alice".to_string()),
        prev_state_data: Some(serde_json::json!({ "balance": 200 })),
        prev_updated_at_block: Some(11),
        prev_created_at_block: Some(10),
    }).unwrap();
    obj.seal = "tx_13:0".to_string();
    obj.satoshis = 1200;
    obj.updated_at_block = 13;
    obj.state_data = serde_json::json!({ "balance": 300 });
    store.save_object(&obj).unwrap();
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 13,
        block_hash: "hash_13".to_string(),
        prev_hash: Some("hash_12".to_string()),
        state_root: "root_13".to_string(),
        timestamp: 1300,
    }).unwrap();
    store.set_last_sync_block("JKC", 13).unwrap();

    // Verify pre-rollback state
    assert_eq!(store.get_object("obj_multi").unwrap().unwrap().satoshis, 1200);
    assert_eq!(store.get_last_sync_block("JKC").unwrap(), 13);

    // --- Rollback 3 blocks: 13 -> 12 -> 11 -> 10 ---
    let rolled = store.rollback_to_block("JKC", 10).unwrap();
    assert_eq!(rolled, 2, "Should roll back 2 undo logs (blocks 11 and 13)");

    // Object should be restored to block 10 state
    let restored = store.get_object("obj_multi").unwrap().unwrap();
    assert_eq!(restored.satoshis, 1000);
    assert_eq!(restored.seal, "tx_10:0");
    assert_eq!(restored.state_data, serde_json::json!({ "balance": 100 }));

    // Blocks 11, 12, 13 must all be cleaned up (including 12 which had no undo log)
    assert!(store.get_block("JKC", 10).unwrap().is_some(), "Block 10 must survive");
    assert!(store.get_block("JKC", 11).unwrap().is_none(), "Block 11 must be cleaned up");
    assert!(store.get_block("JKC", 12).unwrap().is_none(), "Block 12 (no undo log) must be cleaned up");
    assert!(store.get_block("JKC", 13).unwrap().is_none(), "Block 13 must be cleaned up");

    // Last sync block must be reset to 10
    assert_eq!(store.get_last_sync_block("JKC").unwrap(), 10);

    // SMT root must be consistent with the restored state
    let root = store.current_state_root();
    assert!(!root.is_empty());

    // Verify a fresh tree with only the block-10 object produces the same root
    let temp_dir2 = tempfile::tempdir().expect("tempdir2 failed");
    let db_path2 = temp_dir2.path().join("test_multi_reorg_ref.redb");
    let store_ref = StateStore::open(&db_path2).expect("StateStore ref open failed");
    store_ref.save_object(&SmartObjectRecord {
        object_id: "obj_multi".to_string(),
        code_hash: "wasm_code".to_string(),
        seal: "tx_10:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({ "balance": 100 }),
        created_at_block: 10,
        updated_at_block: 10,
    }).unwrap();
    assert_eq!(
        store.current_state_root(),
        store_ref.current_state_root(),
        "SMT root after multi-block rollback must match a fresh tree with the same state"
    );
}

#[test]
fn test_multi_chain_block_isolation() {
    // Pre-existing issue fix: BLOCKS_TABLE was keyed by height only, not
    // (chain, height). Two chains at the same height would collide, and
    // rolling back one chain would delete the other chain's blocks.
    // Now the table is keyed by "{chain}:{height:020}" so chains are isolated.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_multi_chain.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");

    // Save blocks at height 10 for two different chains
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 10,
        block_hash: "jkc_hash_10".to_string(),
        prev_hash: None,
        state_root: "jkc_root_10".to_string(),
        timestamp: 1000,
    }).unwrap();
    store.save_block(&BlockRecord {
        chain: "BTC".to_string(),
        block_height: 10,
        block_hash: "btc_hash_10".to_string(),
        prev_hash: None,
        state_root: "btc_root_10".to_string(),
        timestamp: 1000,
    }).unwrap();

    // Both chains should have their block at height 10
    let jkc_block = store.get_block("JKC", 10).unwrap().expect("JKC block 10 must exist");
    assert_eq!(jkc_block.block_hash, "jkc_hash_10");
    let btc_block = store.get_block("BTC", 10).unwrap().expect("BTC block 10 must exist");
    assert_eq!(btc_block.block_hash, "btc_hash_10");

    // Save a block at height 11 for JKC only
    store.save_block(&BlockRecord {
        chain: "JKC".to_string(),
        block_height: 11,
        block_hash: "jkc_hash_11".to_string(),
        prev_hash: Some("jkc_hash_10".to_string()),
        state_root: "jkc_root_11".to_string(),
        timestamp: 1100,
    }).unwrap();

    // Roll back JKC to height 10 — must NOT affect BTC's block at height 10
    let rolled = store.rollback_to_block("JKC", 10).unwrap();
    assert_eq!(rolled, 0, "No undo logs to roll back");

    // JKC block 11 must be cleaned up
    assert!(store.get_block("JKC", 11).unwrap().is_none(), "JKC block 11 must be cleaned up");

    // JKC block 10 must survive
    assert!(store.get_block("JKC", 10).unwrap().is_some(), "JKC block 10 must survive");

    // BTC block 10 must NOT be affected by JKC rollback
    let btc_block_after = store.get_block("BTC", 10).unwrap()
        .expect("BTC block 10 must survive JKC rollback");
    assert_eq!(btc_block_after.block_hash, "btc_hash_10", "BTC block 10 must be untouched");
}
