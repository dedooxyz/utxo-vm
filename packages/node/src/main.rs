use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use utxo_vmd::consensus::ConsensusManager;
use utxo_vmd::cross_chain::{BridgeManager, CrossChainVerifier, StateRelay};
use utxo_vmd::p2p::{P2pService, DEFAULT_P2P_PORT};
use utxo_vmd::rpc::{create_router, AppState};
use utxo_vmd::scanner::{BlockProcessor, ElectrsClient};
use utxo_vmd::storage::StateStore;
use utxo_vmd::types::{BlockRecord, StateAttestation};

#[derive(Parser, Debug)]
#[command(name = "utxo-vmd", about = "Universal UTXO-VM Node Daemon")]
struct Cli {
    #[arg(long, env = "CHAIN", default_value = "JKC_TESTNET")]
    chain: String,

    #[arg(long, env = "ELECTRS_URL")]
    electrs_url: Option<String>,

    #[arg(long, env = "DB_PATH", default_value = "./data/utxovm.redb")]
    db_path: PathBuf,

    #[arg(long, env = "P2P_PORT", default_value_t = DEFAULT_P2P_PORT)]
    p2p_port: u16,

    #[arg(long, env = "RPC_PORT", default_value_t = 9773)]
    rpc_port: u16,

    #[arg(long, env = "BOOTSTRAP_PEERS")]
    bootstrap_peers: Vec<String>,

    #[arg(long, env = "VALIDATOR_KEY")]
    validator_key: Option<String>,

    #[arg(long, env = "QUORUM", default_value_t = 1)]
    quorum: usize,

    #[arg(long, env = "START_HEIGHT")]
    start_height: Option<u64>,

    #[arg(long, env = "SYNC_INTERVAL_SECS", default_value_t = 15)]
    sync_interval_secs: u64,

    #[arg(long, env = "MIN_EXECUTION_FEE", default_value_t = 0)]
    min_execution_fee: u64,

    #[arg(long, env = "FEE_COLLECTOR")]
    fee_collector: Option<String>,

    #[arg(long, env = "P2P_KEY_PATH", default_value = "./data/p2p_identity.key")]
    p2p_key_path: PathBuf,

    #[arg(long, env = "RATE_LIMIT_RPS", default_value_t = 100)]
    rate_limit_rps: u64,

    #[arg(long, env = "MAX_WASM_SIZE_MB", default_value_t = 1)]
    max_wasm_size_mb: usize,

    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,

    #[arg(long, env = "ELECTRS_TIMEOUT_SECS", default_value_t = 15)]
    electrs_timeout_secs: u64,

    #[arg(long, env = "P2P_COMMAND_TIMEOUT_SECS", default_value_t = 5)]
    p2p_command_timeout_secs: u64,

    #[arg(long, env = "CHANNEL_SIZE", default_value_t = 100)]
    channel_size: usize,

    /// API key for bridge write endpoints. If not set, bridge writes are disabled.
    #[arg(long, env = "BRIDGE_API_KEY")]
    bridge_api_key: Option<String>,

    /// Enable JKC bonding/slashing. When enabled, the node verifies CSV+Taproot
    /// are active on the target chain at startup (one-time hard fail, no polling).
    #[arg(long, env = "BONDING_ENABLED", default_value_t = false)]
    bonding_enabled: bool,

    /// JKC JSON-RPC URL for the one-time startup capability check.
    /// e.g. "http://127.0.0.1:9771". Only used when --bonding-enabled is true.
    /// Credentials are read from JKC_RPC_USER and JKC_RPC_PASS env vars.
    #[arg(long, env = "JKC_RPC_URL")]
    jkc_rpc_url: Option<String>,

    /// Comma-separated list of allowed CORS origins for the RPC server.
    /// If not set, defaults to localhost:3000 and localhost:9773.
    /// Example: "https://app.utxovm.org,https://explorer.utxovm.org"
    #[arg(long, env = "CORS_ORIGINS", value_delimiter = ',')]
    cors_origins: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Parse log level from CLI/env
    let log_level = match cli.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Setting default subscriber failed")?;

    // Resolve Electrs URL based on chain if not provided
    let electrs_url = cli.electrs_url.unwrap_or_else(|| {
        match cli.chain.to_uppercase().as_str() {
            "JKC" | "JKC_MAINNET" => "https://junk-api.s3na.xyz".to_string(),
            "JKC_TESTNET" | "TESTNET" => "https://jkc-testnet-api.s3na.xyz".to_string(),
            "DOGE" | "DOGE_MAINNET" => "https://doge-api.s3na.xyz".to_string(),
            "LTC" | "LTC_MAINNET" => "https://ltc-api.s3na.xyz".to_string(),
            "BTC" | "BTC_MAINNET" => "https://btc-api.s3na.xyz".to_string(),
            "PEP" | "PEP_MAINNET" => "https://pepe-api.s3na.xyz".to_string(),
            "LKY" | "LKY_MAINNET" => "https://lky-api.s3na.xyz".to_string(),
            "BEL" | "BEL_MAINNET" => "https://bel-api.s3na.xyz".to_string(),
            _ => "https://jkc-testnet-api.s3na.xyz".to_string(),
        }
    });

    info!("============================================================");
    info!("Starting UTXO-VM Daemon (utxo-vmd)");
    info!("Chain:        {}", cli.chain);
    info!("Electrs:      {}", electrs_url);
    info!("P2P Port:     {} (Default: 2232)", cli.p2p_port);
    info!("RPC Port:     {}", cli.rpc_port);
    info!("DB Path:      {:?}", cli.db_path);
    info!("Min Fee:      {} sats", cli.min_execution_fee);
    info!("Fee Collector: {:?}", cli.fee_collector);
    info!("============================================================");

    // One-time startup assertion: if bonding is enabled, verify CSV+Taproot
    // are active on the target JKC chain. Hard fail — no polling, no fallback.
    if cli.bonding_enabled {
        let rpc_url = cli.jkc_rpc_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Bonding is enabled (--bonding-enabled) but no JKC RPC URL provided. \
                 Set --jkc-rpc-url or JKC_RPC_URL env var to the JKC daemon JSON-RPC endpoint."
            )
        })?;
        info!("[Bonding] Checking chain capabilities at {} (one-time startup check)...", rpc_url);
        let status = utxo_vmd::consensus::l1_scripts::query_softfork_status(rpc_url)
            .await
            .context("Failed to query JKC chain capabilities")?;
        info!("[Bonding] CSV={}, SegWit={}, Taproot={}",
            status.csv_active, status.segwit_active, status.taproot_active);
        utxo_vmd::consensus::l1_scripts::assert_chain_supports_bonding(&status)
            .context("Chain capability check failed — bonding disabled")?;
        info!("[Bonding] Chain supports CSV+Taproot — bonding enabled.");
    }

    // Create database parent directories if needed
    if let Some(parent) = cli.db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Initialize StateStore with embedded SMT
    let store = StateStore::open(&cli.db_path)?;
    info!("[Storage] Initialized Redb + Sparse Merkle Tree (Current Root: {})", store.current_state_root());

    // Initialize P2P Swarm
    let (attestation_tx, mut attestation_rx) = mpsc::channel::<StateAttestation>(cli.channel_size);
    let (mempool_tx, mut mempool_rx) = mpsc::channel::<Vec<u8>>(cli.channel_size);
    let (p2p_service, p2p_handle) = P2pService::new(cli.p2p_port, cli.bootstrap_peers, Some(cli.p2p_key_path));

    tokio::spawn(async move {
        if let Err(e) = p2p_service.run(Some(attestation_tx), Some(mempool_tx)).await {
            error!("[P2P] Fatal error in P2P service: {:?}", e);
        }
    });

    // Initialize Consensus Manager
    let consensus = Arc::new(ConsensusManager::new(cli.quorum));
    let consensus_clone = consensus.clone();

    // Listen for incoming P2P attestations
    tokio::spawn(async move {
        while let Some(attestation) = attestation_rx.recv().await {
            if consensus_clone.add_attestation(attestation.clone()) {
                info!(
                    "[Consensus] Accepted valid attestation for #{} from {}",
                    attestation.block_height, attestation.validator_pubkey
                );
            }
        }
    });

    // Initialize Electrs client & processor
    let electrs = ElectrsClient::with_timeout(electrs_url.clone(), cli.electrs_timeout_secs);
    let processor = Arc::new(BlockProcessor::new(
        store.clone(),
        cli.chain.clone(),
        cli.min_execution_fee,
        cli.fee_collector.clone(),
    ));

    // Process incoming mempool transactions
    let processor_mempool = processor.clone();
    tokio::spawn(async move {
        while let Some(tx_bytes) = mempool_rx.recv().await {
            match serde_json::from_slice::<utxo_vmd::scanner::electrs::ElectrsTx>(&tx_bytes) {
                Ok(tx) => {
                    match processor_mempool.process_tx(&tx, 0) {
                        Ok(Some(obj_id)) => {
                            info!("[Mempool] Processed transaction for object: {}", obj_id);
                        }
                        Ok(None) => {
                            // No envelope found, skip
                        }
                        Err(e) => {
                            warn!("[Mempool] Failed to process mempool tx: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("[Mempool] Failed to deserialize mempool tx: {:?}", e);
                }
            }
        }
    });

    // Background Block Sync Loop
    let store_sync = store.clone();
    let electrs_sync = electrs.clone();
    let chain_sync = cli.chain.clone();
    let p2p_sync = p2p_handle.clone();
    let consensus_sync = consensus.clone();
    let validator_key_str = cli.validator_key.clone();
    let sync_interval = Duration::from_secs(cli.sync_interval_secs);
    let start_height_opt = cli.start_height;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sync_interval);
        loop {
            interval.tick().await;

            match electrs_sync.get_tip_height().await {
                Ok(tip_height) => {
                    let mut last_synced = store_sync.get_last_sync_block(&chain_sync).unwrap_or(0);
                    if last_synced == 0 {
                        if let Some(sh) = start_height_opt {
                            last_synced = sh.saturating_sub(1);
                        }
                    }
                    if tip_height > last_synced {
                        info!("[Scanner] New blocks detected: #{} -> #{}", last_synced, tip_height);
                        for h in (last_synced + 1)..=tip_height {
                            if let Ok(block_hash) = electrs_sync.get_block_hash(h).await {
                                if let Ok(txs) = electrs_sync.get_block_txs(&block_hash).await {
                                    for tx in &txs {
                                        let _ = processor.process_tx(tx, h);
                                    }
                                }

                                let state_root = store_sync.current_state_root();
                                let block_record = BlockRecord {
                                    chain: chain_sync.clone(),
                                    block_height: h,
                                    block_hash: block_hash.clone(),
                                    prev_hash: None,
                                    state_root: state_root.clone(),
                                    timestamp: chrono::Utc::now().timestamp(),
                                };
                                let _ = store_sync.save_block(&block_record);
                                let _ = store_sync.set_last_sync_block(&chain_sync, h);

                                // Sign & Broadcast Attestation if validator key provided
                                if let Some(ref key_hex) = validator_key_str {
                                    if let Ok(sk_bytes) = hex::decode(key_hex) {
                                        if let Ok(sk) = secp256k1::SecretKey::from_slice(&sk_bytes) {
                                            if let Ok(attestation) = consensus_sync.sign_state_root(&sk, &chain_sync, h, &block_hash, &state_root) {
                                                consensus_sync.add_attestation(attestation.clone());
                                                let _ = p2p_sync.broadcast_attestation(attestation).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("[Scanner] Failed to fetch tip from electrs: {:?}", e);
                }
            }
        }
    });

    // Start Axum HTTP RPC Server
    // Initialize cross-chain components
    let verifier = Arc::new(CrossChainVerifier::new(consensus.clone()));
    let bridge = Arc::new(BridgeManager::new(verifier, store.clone()));
    let relay = Arc::new(StateRelay::new(store.clone(), consensus.clone()));

    let app_state = AppState {
        store,
        p2p: p2p_handle,
        consensus,
        chain: cli.chain,
        electrs_url,
        rate_limit_rps: cli.rate_limit_rps,
        bridge,
        relay,
        rate_limiter: std::sync::Arc::new(utxo_vmd::rpc::server::RateLimiter::new(cli.rate_limit_rps)),
        bridge_api_key: cli.bridge_api_key,
        cors_origins: cli.cors_origins,
    };

    let router = create_router(app_state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cli.rpc_port));
    info!("[RPC] Axum JSON-RPC Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Graceful shutdown on Ctrl+C
    let shutdown_signal = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!("[Shutdown] Failed to install Ctrl+C handler: {}", e);
            return;
        }
        info!("[Shutdown] Received Ctrl+C, shutting down gracefully...");
    };

    let server = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal);

    server.await?;
    info!("[Shutdown] Daemon stopped cleanly.");

    Ok(())
}
