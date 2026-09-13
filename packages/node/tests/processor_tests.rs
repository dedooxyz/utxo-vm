use utxo_vmd::scanner::electrs::{ElectrsTx, ElectrsTxInput, ElectrsTxOutput};
use utxo_vmd::scanner::BlockProcessor;
use utxo_vmd::storage::StateStore;
use utxo_vmd::types::SmartObjectRecord;

/// Build a valid utxovm envelope script:
/// OP_FALSE OP_IF <push "utxovm"> <push 0x01> <push "application/json"> <push payload> OP_ENDIF
fn build_envelope_script(payload: &[u8]) -> Vec<u8> {
    let mut script = vec![0x00]; // OP_FALSE
    script.push(0x63); // OP_IF
    script.push(0x06); // push 6 bytes
    script.extend_from_slice(b"utxovm");
    script.push(0x01); // push 1 byte (version)
    script.push(0x01); // version = 1
    script.push(0x10); // push 16 bytes (content type)
    script.extend_from_slice(b"application/json");
    // Push payload (handle >75 bytes with PUSHDATA1)
    if payload.len() < 0x4c {
        script.push(payload.len() as u8);
    } else if payload.len() <= 0xff {
        script.push(0x4c); // PUSHDATA1
        script.push(payload.len() as u8);
    } else {
        script.push(0x4d); // PUSHDATA2
        script.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    }
    script.extend_from_slice(payload);
    script.push(0x68); // OP_ENDIF
    script
}

#[test]
fn test_processor_create_and_call() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_processor.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), 0, None);

    // 1. Transaction that creates a Smart Object
    let create_payload = br#"{"op":"create","code_hash":"wasm_default","state":{"pool":"JKC/DOGE","liquidity":50000}}"#;
    let create_script = build_envelope_script(create_payload);

    let tx_create = ElectrsTx {
        txid: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 100_000,
            scriptpubkey: hex::encode(&create_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script)),
        }],
    };

    let created_id = processor
        .process_tx(&tx_create, 100)
        .expect("Process create failed")
        .expect("Expected object id created");

    assert_eq!(created_id, "obj_1111111111111111");

    let obj = store.get_object(&created_id).unwrap().expect("Object must exist");
    assert_eq!(obj.satoshis, 100_000);
    assert_eq!(obj.seal, "1111111111111111111111111111111111111111111111111111111111111111:0");

    // 2. Transaction that spends the seal and calls a method
    let call_payload = br#"{"op":"call","method":"swap","args":{"to":"bob_trader","amount":500}}"#;
    let call_script = build_envelope_script(call_payload);

    let tx_call = ElectrsTx {
        txid: "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            vout: 0, // Consumes the seal!
        }],
        vout: vec![ElectrsTxOutput {
            value: 99_500,
            scriptpubkey: hex::encode(&call_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&call_script)),
        }],
    };

    let called_id = processor
        .process_tx(&tx_call, 101)
        .expect("Process call failed")
        .expect("Expected object updated");

    assert_eq!(called_id, "obj_1111111111111111");

    let updated_obj = store.get_object(&created_id).unwrap().unwrap();
    // New seal created!
    assert_eq!(updated_obj.seal, "2222222222222222222222222222222222222222222222222222222222222222:0");
    assert_eq!(updated_obj.owner, "bob_trader");
    assert_eq!(updated_obj.satoshis, 99_500);

    // Old seal is now gone/unresolvable to this current state
    assert!(store.get_object_by_seal("1111111111111111111111111111111111111111111111111111111111111111:0").unwrap().is_none());
    // New seal resolves to the updated object
    assert!(store.get_object_by_seal("2222222222222222222222222222222222222222222222222222222222222222:0").unwrap().is_some());
}

#[test]
fn test_fee_enforcement_with_min_fee() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_fee.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let min_fee = 1000;
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), min_fee, None);

    let create_payload = br#"{"op":"create","code_hash":"wasm_default"}"#;
    let create_script = build_envelope_script(create_payload);

    // Transaction with insufficient fee (500 < 1000)
    let tx_no_fee = ElectrsTx {
        txid: "3333333333333333333333333333333333333333333333333333333333333333".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 500, // Less than min_fee
            scriptpubkey: hex::encode(&create_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script)),
        }],
    };

    // Should be rejected due to insufficient fee
    let result = processor.process_tx(&tx_no_fee, 200).expect("Process should not error");
    assert!(result.is_none(), "Transaction with insufficient fee should be rejected");

    // Transaction with sufficient fee (1500 >= 1000)
    let tx_with_fee = ElectrsTx {
        txid: "4444444444444444444444444444444444444444444444444444444444444444".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 1500, // Greater than min_fee
            scriptpubkey: hex::encode(&create_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script)),
        }],
    };

    // Should be accepted
    let result = processor.process_tx(&tx_with_fee, 201).expect("Process should succeed");
    assert!(result.is_some(), "Transaction with sufficient fee should be accepted");
}

#[test]
fn test_fee_enforcement_with_collector() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_fee_collector.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let min_fee = 1000;
    let fee_collector = "fee_collector_addr".to_string();
    let processor = BlockProcessor::new(
        store.clone(),
        "JKC_TESTNET".to_string(),
        min_fee,
        Some(fee_collector.clone()),
    );

    let create_payload = br#"{"op":"create","code_hash":"wasm_default"}"#;
    let create_script = build_envelope_script(create_payload);

    // Transaction with fee output to WRONG address in scriptpubkey_asm
    let tx_wrong_collector = ElectrsTx {
        txid: "5555555555555555555555555555555555555555555555555555555555555555".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 1500,
            scriptpubkey: hex::encode(&create_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script)),
        }, ElectrsTxOutput {
            value: 1000,
            scriptpubkey: "wrong_fee_script".to_string(),
            scriptpubkey_asm: Some("OP_DUP OP_HASH160 wrong_addr OP_EQUALVERIFY OP_CHECKSIG".to_string()),
            scriptpubkey_hex: Some("76a914deadbeef88ac".to_string()),
        }],
    };

    let result = processor.process_tx(&tx_wrong_collector, 300).expect("Process should not error");
    assert!(result.is_none(), "Transaction with wrong fee collector should be rejected");

    // Transaction WITH fee output to correct collector (scriptpubkey matches exactly)
    let create_payload2 = br#"{"op":"create","code_hash":"wasm_default2"}"#;
    let create_script2 = build_envelope_script(create_payload2);

    let tx_with_collector = ElectrsTx {
        txid: "6666666666666666666666666666666666666666666666666666666666666666".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 1500,
            scriptpubkey: hex::encode(&create_script2),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script2)),
        }, ElectrsTxOutput {
            value: 1000,
            scriptpubkey: "fee_collector_addr".to_string(),
            scriptpubkey_asm: Some("OP_DUP OP_HASH160 fee_collector_addr OP_EQUALVERIFY OP_CHECKSIG".to_string()),
            scriptpubkey_hex: Some("76a914deadbeef88ac".to_string()),
        }],
    };

    let result = processor.process_tx(&tx_with_collector, 301).expect("Process should succeed");
    assert!(result.is_some(), "Transaction with fee collector output should be accepted");
}

#[test]
fn test_fee_enforcement_substring_false_positive_rejected() {
    // Issue 13: A scriptpubkey_asm that coincidentally contains the collector
    // address as a substring — without actually paying it — must be rejected.
    // Before the fix, validate_fee used asm.contains(collector) which would
    // false-positive on this input.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_fee_substring.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let min_fee = 1000;
    let fee_collector = "fee_collector_addr".to_string();
    let processor = BlockProcessor::new(
        store.clone(),
        "JKC_TESTNET".to_string(),
        min_fee,
        Some(fee_collector.clone()),
    );

    let create_payload = br#"{"op":"create","code_hash":"wasm_default"}"#;
    let create_script = build_envelope_script(create_payload);

    // The scriptpubkey_asm contains "fee_collector_addr" as a substring,
    // but the actual scriptpubkey is a completely different script.
    // Under the old substring matching, this would false-positive.
    let tx_substring_trap = ElectrsTx {
        txid: "7777777777777777777777777777777777777777777777777777777777777777".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 1500,
            scriptpubkey: hex::encode(&create_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&create_script)),
        }, ElectrsTxOutput {
            value: 1000,
            // scriptpubkey does NOT match the collector exactly
            scriptpubkey: "some_other_script".to_string(),
            // BUT scriptpubkey_asm coincidentally contains the collector address
            scriptpubkey_asm: Some("OP_DUP OP_HASH160 fee_collector_addr OP_EQUALVERIFY OP_CHECKSIG".to_string()),
            scriptpubkey_hex: Some("76a914deadbeef88ac".to_string()),
        }],
    };

    let result = processor.process_tx(&tx_substring_trap, 400).expect("Process should not error");
    assert!(result.is_none(), "Transaction with substring-only collector match must be rejected");
}

#[test]
fn test_execute_wasm_non_json_state_returns_error() {
    // Issue 10: When a WASM contract returns non-JSON state data from
    // get_state, execute_wasm must return Err (not silently fall back to
    // old state). This test calls execute_wasm directly with a WAT module
    // that returns raw non-JSON bytes from get_state.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_nonjson.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), 0, None);

    // WAT module that returns non-JSON bytes from get_state.
    // get_state writes "NOT_JSON\x00" to the output buffer and returns 9.
    let wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "allocate") (param $size i32) (result i32) (i32.const 0x1000))
            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.store8 (i32.const 0x1000) (i32.const 0x4e))  ;; N
                (i32.store8 (i32.const 0x1001) (i32.const 0x4f))  ;; O
                (i32.store8 (i32.const 0x1002) (i32.const 0x54))  ;; T
                (i32.store8 (i32.const 0x1003) (i32.const 0x5f))  ;; _
                (i32.store8 (i32.const 0x1004) (i32.const 0x4a))  ;; J
                (i32.store8 (i32.const 0x1005) (i32.const 0x53))  ;; S
                (i32.store8 (i32.const 0x1006) (i32.const 0x4f))  ;; O
                (i32.store8 (i32.const 0x1007) (i32.const 0x4e))  ;; N
                (i32.store8 (i32.const 0x1008) (i32.const 0x00))  ;; null
                (i32.const 9)
            )
            (func (export "call") (param i32 i32 i32) (result i32) (i32.const 0))
        )
    "#;
    let wasm_bytes = wat::parse_str(wat).expect("Failed to parse WAT");

    let obj = SmartObjectRecord {
        object_id: "obj_nonjson_test".to_string(),
        code_hash: utxo_core_vm::VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: "aabbccdd11223344:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({"balance": 500}),
        created_at_block: 1,
        updated_at_block: 1,
    };

    let result = processor.execute_wasm(
        &obj,
        "transfer",
        &serde_json::json!({"to": "bob", "amount": 100}),
        "alice",
        1000,
        &wasm_bytes,
    );

    assert!(result.is_err(), "execute_wasm with non-JSON state must return Err");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("non-JSON"),
        "Error should mention non-JSON parse failure, got: {}",
        err
    );
}

#[test]
fn test_execute_wasm_valid_json_state_succeeds() {
    // Issue 10 regression: a contract that returns valid JSON must still
    // succeed (no false rejection from the parse-failure fix).
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_validjson.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), 0, None);

    // WAT module that returns valid JSON '{}' from get_state.
    let wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "allocate") (param $size i32) (result i32) (i32.const 0x1000))
            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.store8 (i32.const 0x1000) (i32.const 0x7b))  ;; {
                (i32.store8 (i32.const 0x1001) (i32.const 0x7d))  ;; }
                (i32.const 2)
            )
            (func (export "call") (param i32 i32 i32) (result i32) (i32.const 0))
        )
    "#;
    let wasm_bytes = wat::parse_str(wat).expect("Failed to parse WAT");

    let obj = SmartObjectRecord {
        object_id: "obj_validjson_test".to_string(),
        code_hash: utxo_core_vm::VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: "aabbccdd11223344:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({"balance": 500}),
        created_at_block: 1,
        updated_at_block: 1,
    };

    let result = processor.execute_wasm(
        &obj,
        "transfer",
        &serde_json::json!({"to": "bob", "amount": 100}),
        "alice",
        1000,
        &wasm_bytes,
    );

    assert!(result.is_ok(), "execute_wasm with valid JSON state must succeed, got: {:?}", result.err());
    let exec_result = result.unwrap();
    assert_eq!(exec_result.state_data, serde_json::json!({}));
}

/// Build a WASM deploy envelope script with content_type=application/wasm:
/// OP_FALSE OP_IF <push "utxovm"> <push 0x01> <push "application/wasm"> <push wasm> OP_ENDIF
fn build_wasm_deploy_script(wasm_bytes: &[u8]) -> Vec<u8> {
    let mut script = vec![0x00, 0x63]; // OP_FALSE OP_IF
    script.push(0x06);
    script.extend_from_slice(b"utxovm");
    script.push(0x01);
    script.push(0x01); // version = 1
    script.push(0x10); // content type len = 16? no — "application/wasm" is 16 bytes? "application/wasm" = 16 chars. yes.
    script.extend_from_slice(b"application/wasm");
    if wasm_bytes.len() < 0x4c {
        script.push(wasm_bytes.len() as u8);
    } else if wasm_bytes.len() <= 0xff {
        script.push(0x4c);
        script.push(wasm_bytes.len() as u8);
    } else {
        script.push(0x4d);
        script.extend_from_slice(&(wasm_bytes.len() as u16).to_le_bytes());
    }
    script.extend_from_slice(wasm_bytes);
    script.push(0x68); // OP_ENDIF
    script
}

#[test]
fn test_deploy_wasm_envelope_stores_blob_and_call_fetches_locally() {
    // Deploy tx carries content_type=application/wasm → payload is stored
    // content-addressed in the WASM table. A later call resolves the blob
    // locally without any DHT round-trip.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_wasm_store.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), 0, None);

    // Minimal valid WASM: returns "{}" from get_state
    let wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "init") (param i32 i32) (result i32) (i32.const 0))
            (func (export "allocate") (param $size i32) (result i32) (i32.const 0x1000))
            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.store8 (i32.const 0x1000) (i32.const 0x7b))
                (i32.store8 (i32.const 0x1001) (i32.const 0x7d))
                (i32.const 2)
            )
            (func (export "call") (param i32 i32 i32) (result i32) (i32.const 0))
        )
    "#;
    let wasm_bytes = wat::parse_str(wat).expect("WAT parse failed");
    let code_hash = utxo_core_vm::VmRuntime::calculate_code_hash(&wasm_bytes);

    let deploy_script = build_wasm_deploy_script(&wasm_bytes);
    let tx_deploy = ElectrsTx {
        txid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 50_000,
            scriptpubkey: hex::encode(&deploy_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&deploy_script)),
        }],
    };

    let obj_id = processor
        .process_tx(&tx_deploy, 500)
        .expect("deploy process failed")
        .expect("expected object id");

    // WASM blob must be stored under its sha256 code_hash
    let stored = store.get_wasm(&code_hash).expect("get_wasm failed");
    assert_eq!(stored.as_deref(), Some(wasm_bytes.as_slice()), "stored wasm must round-trip");

    // Object's code_hash should default to the computed sha256 of the payload
    let obj = store.get_object(&obj_id).unwrap().unwrap();
    assert_eq!(obj.code_hash, code_hash);

    // Call the object — get_wasm_bytes must resolve locally
    let call_payload = br#"{"op":"call","method":"noop","args":{}}"#;
    let call_script = build_envelope_script(call_payload);
    let tx_call = ElectrsTx {
        txid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 49_500,
            scriptpubkey: hex::encode(&call_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&call_script)),
        }],
    };

    let updated = processor
        .process_tx(&tx_call, 501)
        .expect("call process failed")
        .expect("expected object updated");
    assert_eq!(updated, obj_id);
}

#[test]
fn test_missing_wasm_propagates_wasm_missing_error() {
    // A call against an object whose code_hash is not in the local WASM
    // table must surface a WASM_MISSING error (not Ok(None)) so the async
    // scanner can DHT-fetch and retry.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_wasm_missing.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string(), 0, None);

    // Seed an object directly with a code_hash that has no local blob.
    let obj = SmartObjectRecord {
        object_id: "obj_missing".to_string(),
        code_hash: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
        seal: "9999999999999999999999999999999999999999999999999999999999999999:0".to_string(),
        satoshis: 1000,
        owner: "alice".to_string(),
        state_data: serde_json::json!({"x": 1}),
        created_at_block: 600,
        updated_at_block: 600,
    };
    store.save_object(&obj).unwrap();

    let call_payload = br#"{"op":"call","method":"noop","args":{}}"#;
    let call_script = build_envelope_script(call_payload);
    let tx_call = ElectrsTx {
        txid: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        vin: vec![ElectrsTxInput {
            txid: "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
            vout: 0,
        }],
        vout: vec![ElectrsTxOutput {
            value: 900,
            scriptpubkey: hex::encode(&call_script),
            scriptpubkey_asm: None,
            scriptpubkey_hex: Some(hex::encode(&call_script)),
        }],
    };

    let err = processor
        .process_tx(&tx_call, 601)
        .expect_err("must surface WASM_MISSING");
    let hash = utxo_vmd::scanner::missing_wasm_hash(&err)
        .expect("error must be WASM_MISSING");
    assert_eq!(hash, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
}

#[test]
fn test_save_wasm_rejects_hash_mismatch() {
    // Content-addressing enforcement: save_wasm must reject bytes whose
    // sha256 does not equal the claimed code_hash.
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_wasm_hash.redb");
    let store = StateStore::open(&db_path).expect("StateStore open failed");

    let wasm = b"\x00asm\x01\x00\x00\x00";
    let wrong_hash = "00".repeat(32);
    assert!(store.save_wasm(&wrong_hash, wasm).is_err());

    let right_hash = utxo_core_vm::VmRuntime::calculate_code_hash(wasm);
    assert!(store.save_wasm(&right_hash, wasm).is_ok());
    assert_eq!(store.get_wasm(&right_hash).unwrap().as_deref(), Some(wasm.as_slice()));
}
