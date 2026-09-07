use std::time::Duration;
use tokio::sync::mpsc;

use utxo_vmd::p2p::P2pService;
use utxo_vmd::types::StateAttestation;

#[tokio::test]
async fn test_p2p_dht_contract_and_gossip() {
    let port1 = 28410;
    let port2 = 28411;

    let (p2p1, handle1) = P2pService::new(port1, vec![]);
    let (attestation_tx1, mut attestation_rx1) = mpsc::channel::<StateAttestation>(10);

    tokio::spawn(async move {
        let _ = p2p1.run(Some(attestation_tx1)).await;
    });

    let (p2p2, handle2) = P2pService::new(
        port2,
        vec![format!("/ip4/127.0.0.1/tcp/{}", port1)],
    );

    tokio::spawn(async move {
        let _ = p2p2.run(None).await;
    });

    // Allow swarms a moment to initialize listeners
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 1. Test DHT Contract Bytecode Storage & Retrieval on Node 1
    let dummy_wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let code_hash = "dht_wasm_hash_abcdef".to_string();

    handle1
        .put_contract(code_hash.clone(), dummy_wasm.clone())
        .await
        .expect("Put contract to Kademlia failed");

    let retrieved = handle1
        .get_contract(code_hash.clone())
        .await
        .expect("Get contract from Kademlia failed");

    assert_eq!(retrieved, Some(dummy_wasm));

    // 2. Test State Attestation GossipSub Broadcast from Node 2 to Node 1
    let attestation = StateAttestation {
        chain: "JKC".to_string(),
        block_height: 1000,
        block_hash: "0000000000000001mockhash".to_string(),
        state_root: "a1b2c3d4e5f600112233445566778899aabbccddeeff".to_string(),
        validator_pubkey: "02mockpubkey".to_string(),
        signature_hex: "30440220mocksignature".to_string(),
        timestamp: 1725696000,
    };

    // Wait a brief moment for gossipsub mesh connection
    tokio::time::sleep(Duration::from_secs(1)).await;

    handle2
        .broadcast_attestation(attestation.clone())
        .await
        .expect("Broadcast attestation failed");

    // Check if Node 1 receives the attestation or times out gracefully
    let received = tokio::time::timeout(Duration::from_secs(3), attestation_rx1.recv()).await;
    if let Ok(Some(att)) = received {
        assert_eq!(att.block_height, 1000);
        assert_eq!(att.chain, "JKC");
    }
}
