//! Tests for the ElectrsClient URL-path validation and hardening (Issues 21-23).

use utxo_vmd::scanner::electrs::ElectrsClient;

#[tokio::test]
async fn test_issue21_get_tx_rejects_path_traversal_txid() {
    // A txid containing "../" must be rejected before any HTTP request is made.
    // The validation runs synchronously before the .send() call, so no network
    // access is attempted even without a mock server.
    let client = ElectrsClient::new("http://127.0.0.1:9".to_string());
    let result = client.get_tx("../etc/passwd").await;
    assert!(result.is_err(), "path-traversal txid must be rejected");
    // The error must come from validation, not from a network/HTTP failure.
    let err = result.unwrap_err().to_string();
    assert!(
        !err.contains("error_for_status") && !err.contains("connect") && !err.contains("timeout"),
        "must fail on validation, not on network access. got: {}",
        err
    );
}

#[tokio::test]
async fn test_issue21_get_block_txs_rejects_path_traversal_block_hash() {
    let client = ElectrsClient::new("http://127.0.0.1:9".to_string());
    let result = client.get_block_txs("../../admin").await;
    assert!(result.is_err(), "path-traversal block_hash must be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        !err.contains("error_for_status") && !err.contains("connect") && !err.contains("timeout"),
        "must fail on validation, not on network access. got: {}",
        err
    );
}

#[tokio::test]
async fn test_issue21_get_tx_rejects_wrong_length_hex() {
    // Valid hex but wrong length (not 64 chars / 32 bytes) must also be rejected.
    let client = ElectrsClient::new("http://127.0.0.1:9".to_string());
    let result = client.get_tx("deadbeef").await;
    assert!(result.is_err(), "short hex txid must be rejected");
}

#[tokio::test]
async fn test_issue21_get_tx_rejects_non_hex() {
    let client = ElectrsClient::new("http://127.0.0.1:9".to_string());
    // 64 chars but contains non-hex characters
    let bad = "z".repeat(64);
    let result = client.get_tx(&bad).await;
    assert!(result.is_err(), "non-hex txid must be rejected");
}
