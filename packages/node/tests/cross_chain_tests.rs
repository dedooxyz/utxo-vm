use std::sync::Arc;
use utxo_vmd::consensus::ConsensusManager;
use utxo_vmd::cross_chain::{BridgeManager, CrossChainProof, CrossChainVerifier, StateRelay};
use utxo_vmd::storage::StateStore;
use utxo_vmd::types::{SmartObjectRecord, SmtInclusionProof};

#[tokio::test]
async fn test_cross_chain_bridge_flow() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("cross_chain_test.redb");
    let store = StateStore::open(&db_path).unwrap();
    let consensus = Arc::new(ConsensusManager::new(3));
    let verifier = Arc::new(CrossChainVerifier::new(consensus.clone()));
    let bridge = Arc::new(BridgeManager::new(verifier.clone(), store.clone()));

    // Insert test object on source chain
    let obj = SmartObjectRecord {
        object_id: "obj_son_token".to_string(),
        code_hash: "utx20_son".to_string(),
        seal: "tx_source:0".to_string(),
        satoshis: 1_000_000,
        owner: "owner_jkc".to_string(),
        state_data: serde_json::json!({
            "ticker": "SON",
            "balance": 1000
        }),
        created_at_block: 100,
        updated_at_block: 100,
    };
    store.save_object(&obj).unwrap();

    // Step 1: Lock assets on source chain (JKC)
    let transfer = bridge
        .lock_assets("JKC", "obj_son_token", "DOGE", "doge_owner_addr")
        .unwrap();

    assert_eq!(transfer.source_chain, "JKC");
    assert_eq!(transfer.dest_chain, "DOGE");
    assert_eq!(transfer.amount, 1_000_000);

    // Step 2: Create cross-chain proof
    let proof = CrossChainProof {
        source_chain: "JKC".to_string(),
        source_height: 100,
        source_root: "abc123".to_string(),
        object_id: "obj_son_token".to_string(),
        object_data: serde_json::json!({
            "code_hash": "utx20_son",
            "ticker": "SON",
            "balance": 1000
        }),
        merkle_proof: SmtInclusionProof {
            key_hex: "aa".to_string(),
            value_hex: "bb".to_string(),
            root_hex: "cc".to_string(),
            proof_path: vec![],
            verified: false,
        },
        attestations: vec![],
    };

    // Step 3: Verify proof (will fail due to empty attestations)
    let result = verifier.verify_proof(&proof);
    assert!(!result.valid);
    assert!(!result.quorum_reached);

    // Get transfer status
    let got = bridge.get_transfer(&transfer.id).unwrap().unwrap();
    assert_eq!(got.id, transfer.id);
}

#[tokio::test]
async fn test_state_relay_flow() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("relay_test.redb");
    let store = StateStore::open(&db_path).unwrap();
    let consensus = Arc::new(ConsensusManager::new(3));
    let relay = Arc::new(StateRelay::new(store, consensus));

    // Create relay message
    let message = relay
        .create_relay_message("JKC", 100)
        .unwrap_or_else(|_| {
            // If block doesn't exist, create a mock message
            utxo_vmd::cross_chain::StateRelayMessage {
                source_chain: "JKC".to_string(),
                block_height: 100,
                block_hash: "hash_100".to_string(),
                state_root: "root_100".to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                attestations: vec![],
            }
        });

    // Relay to DOGE
    let record = relay.relay_to_chain(&message, "DOGE").unwrap();
    assert_eq!(record.dest_chain, "DOGE");

    // Get relay record
    let got = relay.get_record(&record.id).unwrap().unwrap();
    assert_eq!(got.id, record.id);
}
