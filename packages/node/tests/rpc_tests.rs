use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use utxo_vmd::consensus::ConsensusManager;
use utxo_vmd::cross_chain::{BridgeManager, CrossChainVerifier, StateRelay};
use utxo_vmd::p2p::P2pService;
use utxo_vmd::rpc::{create_router, AppState};
use utxo_vmd::rpc::server::RateLimiter;
use utxo_vmd::storage::StateStore;
use utxo_vmd::types::SmartObjectRecord;

#[tokio::test]
async fn test_rpc_endpoints_and_merkle_proof() {
    let temp_dir = tempfile::tempdir().expect("tempdir failed");
    let db_path = temp_dir.path().join("rpc_test.redb");

    let store = StateStore::open(&db_path).expect("StateStore open failed");

    // Insert a sample smart object
    let obj = SmartObjectRecord {
        object_id: "obj_test_rpc_1".to_string(),
        code_hash: "wasm_contract_hash_abc".to_string(),
        seal: "tx_mock_123:0".to_string(),
        satoshis: 100_000,
        owner: "test_address_alice".to_string(),
        state_data: serde_json::json!({
            "ticker": "JKC20",
            "balance": 5000
        }),
        created_at_block: 50,
        updated_at_block: 50,
    };
    store.save_object(&obj).expect("Save object failed");
    store.set_last_sync_block("JKC_TESTNET", 50).unwrap();

    let (p2p_service, p2p_handle) = P2pService::new(29991, vec![], None);
    tokio::spawn(async move {
        let _ = p2p_service.run(None, None).await;
    });
    let consensus = Arc::new(ConsensusManager::new(1));

    // Initialize cross-chain components
    let verifier = Arc::new(CrossChainVerifier::new(consensus.clone()));
    let bridge = Arc::new(BridgeManager::new(verifier, store.clone()));
    let relay = Arc::new(StateRelay::new(store.clone(), consensus.clone()));

    let app_state = AppState {
        store: store.clone(),
        p2p: p2p_handle,
        consensus,
        chain: "JKC_TESTNET".to_string(),
        electrs_url: "https://jkc-testnet-api.s3na.xyz".to_string(),
        rate_limit_rps: 100,
        bridge,
        relay,
        rate_limiter: Arc::new(RateLimiter::new(100)),
    };


    let app = create_router(app_state);

    // 1. Test GET /api/v1/chain/info
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/chain/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["chain"], "JKC_TESTNET");
    assert_eq!(json["blockHeight"], 50);
    assert_eq!(json["status"], "synced");

    // 2. Test GET /api/v1/object/:id
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/object/obj_test_rpc_1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["satoshis"], 100_000);
    assert_eq!(json["owner"], "test_address_alice");

    // 3. Test GET /api/v1/object/:id/proof (Cryptographic Merkle Proof)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/object/obj_test_rpc_1/proof")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["verified"], true);
    assert!(json["merkleProof"]["root_hex"].is_string());

    // 4. Test GET /api/v1/state-root
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/state-root")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["blockHeight"], 50);
    assert_eq!(json["stateRoot"], store.current_state_root());

    // 5. Test GET 404 for non-existent object
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/object/non_existent_obj")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
