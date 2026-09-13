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

// ---------------------------------------------------------------------------
// Issue 22: response body size cap
// ---------------------------------------------------------------------------

/// Spawn a local TCP server that responds with a Content-Length exceeding the
/// 64 MB cap. The client must reject the response without allocating the full
/// body. Uses a raw TcpListener to avoid adding a mock-server dev-dependency.
async fn spawn_oversized_server() -> String {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Advertise a 100 MB body (exceeds the 64 MB cap) but send only a few
        // bytes. The client's Content-Length check must reject before reading.
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n[]",
            100 * 1024 * 1024
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        // Keep the connection alive briefly so the client can read the headers.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    url
}

#[tokio::test]
async fn test_issue22_get_mempool_tx_ids_rejects_oversized_response() {
    let url = spawn_oversized_server().await;
    let client = ElectrsClient::with_timeout(url, 5).expect("loopback http must be allowed");
    let result = client.get_mempool_tx_ids().await;
    assert!(result.is_err(), "oversized response must be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("too large") || err.contains("cap"),
        "error must mention the size cap, got: {}",
        err
    );
}

#[tokio::test]
async fn test_issue22_get_block_txs_rejects_oversized_response() {
    let url = spawn_oversized_server().await;
    let client = ElectrsClient::with_timeout(url, 5).expect("loopback http must be allowed");
    let valid_hash = "a".repeat(64);
    let result = client.get_block_txs(&valid_hash).await;
    assert!(result.is_err(), "oversized response must be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("too large") || err.contains("cap"),
        "error must mention the size cap, got: {}",
        err
    );
}

// ---------------------------------------------------------------------------
// Issue 23: TLS / scheme enforcement on electrs URL
// ---------------------------------------------------------------------------

#[test]
fn test_issue23_https_url_allowed() {
    let result = ElectrsClient::with_timeout("https://jkc-testnet-api.s3na.xyz".to_string(), 5);
    assert!(result.is_ok(), "https URL must be allowed for any host");
}

#[test]
fn test_issue23_loopback_http_allowed() {
    let result = ElectrsClient::with_timeout("http://127.0.0.1:3000".to_string(), 5);
    assert!(result.is_ok(), "loopback http must be allowed");

    let result = ElectrsClient::with_timeout("http://localhost:3000".to_string(), 5);
    assert!(result.is_ok(), "localhost http must be allowed");

    let result = ElectrsClient::with_timeout("http://[::1]:3000".to_string(), 5);
    assert!(result.is_ok(), "[::1] http must be allowed");
}

#[test]
fn test_issue23_non_loopback_http_rejected() {
    let result = ElectrsClient::with_timeout("http://jkc-testnet-api.s3na.xyz".to_string(), 5);
    assert!(result.is_err(), "non-loopback http must be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("plain http") || err.contains("non-loopback") || err.contains("https"),
        "error must explain the scheme policy, got: {}",
        err
    );
}

#[test]
fn test_issue23_non_loopback_http_rejected_no_port() {
    let result = ElectrsClient::with_timeout("http://example.com".to_string(), 5);
    assert!(result.is_err(), "non-loopback http without port must be rejected");
}

#[test]
fn test_issue23_invalid_scheme_rejected() {
    let result = ElectrsClient::with_timeout("ftp://127.0.0.1".to_string(), 5);
    assert!(result.is_err(), "non-http(s) scheme must be rejected");
}
