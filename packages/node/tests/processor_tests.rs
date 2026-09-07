use utxo_vmd::scanner::electrs::{ElectrsTx, ElectrsTxInput, ElectrsTxOutput};
use utxo_vmd::scanner::BlockProcessor;
use utxo_vmd::storage::StateStore;

#[test]
fn test_processor_create_and_call() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("test_processor.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");
    let processor = BlockProcessor::new(store.clone(), "JKC_TESTNET".to_string());

    // 1. Transaction that creates a Smart Object
    let mut create_script = vec![0x6a];
    create_script.extend_from_slice(b"utxovm");
    create_script.extend_from_slice(
        br#"{"op":"create","code_hash":"wasm_dex_pool","state":{"pool":"JKC/DOGE","liquidity":50000}}"#,
    );

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
    let mut call_script = vec![0x6a];
    call_script.extend_from_slice(b"utxovm");
    call_script.extend_from_slice(
        br#"{"op":"call","method":"swap","args":{"to":"bob_trader","amount":500}}"#,
    );

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
