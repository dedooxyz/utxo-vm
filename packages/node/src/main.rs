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

    #[arg(long, env = "ELECTRS_URL", default_value = "https://jkc-testnet-api.s3na.xyz")]
    electrs_url: String,

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Setting default subscriber failed")?;

    let cli = Cli::parse();

    info!("============================================================");
    info!("Starting UTXO-VM Daemon (utxo-vmd)");
    info!("Chain:        {}", cli.chain);
    info!("Electrs:      {}", cli.electrs_url);
    info!("P2P Port:     {} (Default: 2232)", cli.p2p_port);
    info!("RPC Port:     {}", cli.rpc_port);
    info!("DB Path:      {:?}", cli.db_path);
    info!("============================================================");

    // Create database parent directories if needed
    if let Some(parent) = cli.db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Initialize StateStore with embedded SMT
    let store = StateStore::open(&cli.db_path)?;
    info!("[Storage] Initialized Redb + Sparse Merkle Tree (Current Root: {})", store.current_state_root());

    // Initialize P2P Swarm
    let (attestation_tx, mut attestation_rx) = mpsc::channel::<StateAttestation>(100);
    let (p2p_service, p2p_handle) = P2pService::new(cli.p2p_port, cli.bootstrap_peers);

    tokio::spawn(async move {
        if let Err(e) = p2p_service.run(Some(attestation_tx)).await {
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
    let electrs = ElectrsClient::new(cli.electrs_url.clone());
    let processor = Arc::new(BlockProcessor::new(store.clone(), cli.chain.clone()));

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
    let app_state = AppState {
        store,
        p2p: p2p_handle,
        consensus,
        chain: cli.chain,
        electrs_url: cli.electrs_url,
    };

    let router = create_router(app_state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cli.rpc_port));
    info!("[RPC] Axum JSON-RPC Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
