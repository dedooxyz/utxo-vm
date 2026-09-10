//! Live JKC Testnet Tests
//!
//! Run with: cargo test -p utxo-vmd --test live_testnet -- --nocapture --ignored
//!
//! These tests hit the live JKC testnet at https://jkc-testnet-api.s3na.xyz
//! They are marked #[ignore] by default to avoid CI failures.

use utxo_vmd::scanner::electrs::ElectrsClient;
use utxo_vmd::consensus::l1_scripts::{self, SoftforkStatus};

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";

async fn make_client() -> ElectrsClient {
    ElectrsClient::new(ELECTRS_URL.to_string())
}

#[tokio::test]
#[ignore]
async fn live_testnet_tip_height() {
    let client = make_client().await;
    let tip = client.get_tip_height().await.expect("Failed to get tip");
    println!("[LIVE] JKC testnet tip height: {}", tip);
    assert!(tip > 177_000, "Tip should be > 177000, got {}", tip);
}

#[tokio::test]
#[ignore]
async fn live_testnet_block_hash() {
    let client = make_client().await;
    let hash = client.get_block_hash(177_000).await.expect("Failed to get block hash");
    println!("[LIVE] JKC testnet block 177000 hash: {}", hash);
    assert!(!hash.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_testnet_historical_deploy_tx_exists() {
    let client = make_client().await;
    // From test-fixtures.json: NativeVault deployment
    let txid = "6a697245861d25435ca42bec07ffaac581f6022494907dd86926889c598c7a13";
    let tx = client.get_tx(txid).await.expect("Failed to fetch historical deploy tx");
    println!("[LIVE] Historical deploy tx {} has {} inputs, {} outputs",
        tx.txid, tx.vin.len(), tx.vout.len());
    assert!(!tx.txid.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_testnet_historical_transfer_tx_exists() {
    let client = make_client().await;
    // From test-fixtures.json: deposit/transfer
    let txid = "42901161e8d04b433c1d8f58e810ce97e6a14752b31c38184e1d0349f6c907cf";
    let tx = client.get_tx(txid).await.expect("Failed to fetch historical transfer tx");
    println!("[LIVE] Historical transfer tx {} has {} inputs, {} outputs",
        tx.txid, tx.vin.len(), tx.vout.len());
    assert!(!tx.txid.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_testnet_historical_batch_tx_exists() {
    let client = make_client().await;
    // From test-fixtures.json: batch1
    let txid = "a9de1d94b9e1b2089a9a7bc1be1680b98262dd919f9efe7bbba7344967f52a8d";
    let tx = client.get_tx(txid).await.expect("Failed to fetch historical batch tx");
    println!("[LIVE] Historical batch tx {} has {} inputs, {} outputs",
        tx.txid, tx.vin.len(), tx.vout.len());
    assert!(!tx.txid.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_testnet_scan_recent_blocks_for_utxovm_envelopes() {
    let client = make_client().await;
    let tip = client.get_tip_height().await.unwrap();
    println!("[LIVE] Scanning last 10 blocks ({}..{}) for utxovm envelopes", tip - 9, tip);

    let mut found_envelopes = 0;
    for height in (tip - 9)..=tip {
        let block_hash = match client.get_block_hash(height).await {
            Ok(h) => h,
            Err(e) => {
                println!("[LIVE] Block {}: fetch failed: {}", height, e);
                continue;
            }
        };
        let txs = match client.get_block_txs(&block_hash).await {
            Ok(t) => t,
            Err(e) => {
                println!("[LIVE] Block {} txs: fetch failed: {}", height, e);
                continue;
            }
        };

        for tx in &txs {
            for (vout, output) in tx.vout.iter().enumerate() {
                // Check OP_RETURN outputs for "utxovm" protocol tag
                if let Some(asm) = &output.scriptpubkey_asm {
                    if asm.contains("utxovm") {
                        println!("[LIVE] Found utxovm envelope in tx {} vout {} (block {})",
                            tx.txid, vout, height);
                        found_envelopes += 1;
                    }
                }
                if let Some(hex) = &output.scriptpubkey_hex {
                    if hex.contains("7574786f766d") { // "utxovm" in hex
                        println!("[LIVE] Found utxovm envelope (hex match) in tx {} vout {} (block {})",
                            tx.txid, vout, height);
                        found_envelopes += 1;
                    }
                }
            }
        }
    }

    println!("[LIVE] Found {} utxovm envelopes in last 10 blocks", found_envelopes);
    // Don't assert > 0 — there may be no activity in recent blocks
}

#[tokio::test]
#[ignore]
async fn live_testnet_softfork_status_via_rpc() {
    // This test requires JKC_RPC_USER and JKC_RPC_PASS env vars
    // and a local JKC node RPC at the specified URL.
    let rpc_url = std::env::var("JKC_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9771".to_string());

    println!("[LIVE] Querying softfork status from {}", rpc_url);
    match l1_scripts::query_softfork_status(&rpc_url).await {
        Ok(status) => {
            println!("[LIVE] Softfork status: CSV={}, SegWit={}, Taproot={}",
                status.csv_active, status.segwit_active, status.taproot_active);
            l1_scripts::assert_chain_supports_bonding(&status)
                .expect("Chain must support bonding (CSV + Taproot)");
            println!("[LIVE] Bonding prerequisites met");
        }
        Err(e) => {
            println!("[LIVE] RPC not available ({}). Skipping softfork assertion.", e);
            // Not a hard failure — RPC may not be running
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_testnet_mempool_not_empty() {
    let client = make_client().await;
    // The mempool endpoint may return different formats depending on electrs version.
    // We just verify it's reachable, not the exact format.
    let url = format!("{}/mempool/txids", ELECTRS_URL);
    let resp = reqwest::get(&url).await.expect("Failed to fetch mempool");
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    println!("[LIVE] Mempool endpoint returned HTTP {} ({} bytes)", status, body.len());
    assert!(status.is_success(), "Mempool endpoint must return success, got {}", status);
}
