use utxo_vmd::storage::StateStore;
use utxo_vmd::types::{SmartObjectRecord, UndoLogRecord};

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

    let mut store = StateStore::open(&db_path).expect("StateStore open failed");

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

    // Rollback to block 10
    let rolled = store.rollback_to_block("JKC", 10).unwrap();
    assert_eq!(rolled, 1);

    let restored = store.get_object("obj_reorg").unwrap().unwrap();
    assert_eq!(restored.satoshis, 1000);
    assert_eq!(restored.seal, "tx_first:0");
}
