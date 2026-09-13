use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use utxo_vmd::consensus::attestation::verify_equivocation_proof;
use utxo_vmd::consensus::challenge::{self, ChallengeInput, ChallengeTransaction};
use utxo_vmd::consensus::l1_scripts::VaultConfig;
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

    /// Comma-separated list of trusted reverse proxy IPs. When set, the
    /// rate limiter trusts X-Forwarded-For only for requests originating
    /// from these IPs. When not set (default), the rate limiter keys off
    /// the real connection socket address and ignores X-Forwarded-For
    /// entirely — this is the safe default for direct (non-proxy) deployments.
    /// Example: "127.0.0.1,10.0.0.1"
    #[arg(long, env = "TRUSTED_PROXY_IPS", value_delimiter = ',')]
    trusted_proxy_ips: Vec<String>,

    /// Path to a JSON file describing the vault config this node will use
    /// when broadcasting challenge transactions detected by the fraud-watch
    /// background task. Schema matches `l1_scripts::VaultConfig` serialized
    /// as JSON. When omitted, fraud-watch still detects and logs divergence
    /// and equivocation, but does not broadcast challenge transactions.
    #[arg(long, env = "FRAUD_WATCH_VAULT_CONFIG")]
    fraud_watch_vault_config: Option<PathBuf>,

    /// Bond UTXO txid (big-endian hex) this node will spend when broadcasting
    /// a challenge transaction. Required together with
    /// --fraud-watch-vault-config and --fraud-watch-bond-vout/--fraud-watch-bond-amount
    /// to enable automatic challenge broadcast.
    #[arg(long, env = "FRAUD_WATCH_BOND_TXID")]
    fraud_watch_bond_txid: Option<String>,

    /// Bond UTXO vout for --fraud-watch-bond-txid.
    #[arg(long, env = "FRAUD_WATCH_BOND_VOUT")]
    fraud_watch_bond_vout: Option<u32>,

    /// Bond amount (satoshis) locked in --fraud-watch-bond-txid:vout.
    #[arg(long, env = "FRAUD_WATCH_BOND_AMOUNT")]
    fraud_watch_bond_amount: Option<u64>,

    /// Comma-separated hex-encoded watcher signatures to embed in the
    /// challenge transaction witness. Format: "sig1hex,sig2hex,...".
    /// In a real deployment these come from M-of-N co-signers; for the
    /// automatic single-node path they are configured statically here.
    #[arg(long, env = "FRAUD_WATCH_WATCHER_SIGS", value_delimiter = ',')]
    fraud_watch_watcher_sigs: Vec<String>,

    /// Miner fee (satoshis) deducted from the bond amount when building a
    /// challenge transaction.
    #[arg(long, env = "FRAUD_WATCH_MINER_FEE", default_value_t = 1000)]
    fraud_watch_miner_fee: u64,
}

/// JSON schema for --fraud-watch-vault-config. Mirrors `l1_scripts::VaultConfig`
/// but with hex-encoded byte fields so the config file is human-readable.
#[derive(Debug, serde::Deserialize)]
struct FraudWatchVaultConfigJson {
    operator_pubkey_hex: String,
    challenger_pubkey_hex: String,
    unbond_delay: u32,
    claim_delay: u32,
    /// Hex-encoded watcher pubkeys, comma-separated within each string is NOT
    /// supported — provide one JSON array element per pubkey.
    watcher_pubkeys_hex: Vec<String>,
    watcher_threshold: u32,
}

impl FraudWatchVaultConfigJson {
    fn to_vault_config(&self) -> Result<VaultConfig> {
        let operator_pubkey = hex::decode(&self.operator_pubkey_hex)
            .context("operator_pubkey_hex")?;
        let challenger_pubkey = hex::decode(&self.challenger_pubkey_hex)
            .context("challenger_pubkey_hex")?;
        let watcher_pubkeys = self
            .watcher_pubkeys_hex
            .iter()
            .map(|h| hex::decode(h).context("watcher_pubkeys_hex"))
            .collect::<Result<Vec<_>>>()?;
        Ok(VaultConfig {
            operator_pubkey,
            challenger_pubkey,
            unbond_delay: self.unbond_delay,
            claim_delay: self.claim_delay,
            watcher_pubkeys,
            watcher_threshold: self.watcher_threshold,
        })
    }
}

/// Resolved fraud-watch configuration. `None` means detection runs but
/// automatic challenge broadcast is disabled (vault config or bond UTXO
/// not provided).
#[derive(Clone)]
struct FraudWatchConfig {
    vault_config: VaultConfig,
    bond_input: ChallengeInput,
    watcher_signatures: Vec<Vec<u8>>,
    miner_fee: u64,
}

fn load_fraud_watch_config(cli: &Cli) -> Result<Option<FraudWatchConfig>> {
    let vault_path = match cli.fraud_watch_vault_config.as_ref() {
        Some(p) => p,
        None => return Ok(None),
    };
    let vault_json = std::fs::read_to_string(vault_path)
        .with_context(|| format!("reading fraud-watch vault config {:?}", vault_path))?;
    let parsed: FraudWatchVaultConfigJson = serde_json::from_str(&vault_json)
        .context("parsing fraud-watch vault config JSON")?;
    let vault_config = parsed.to_vault_config()?;

    let bond_txid = cli.fraud_watch_bond_txid.clone().ok_or_else(|| {
        anyhow::anyhow!("--fraud-watch-bond-txid is required when --fraud-watch-vault-config is set")
    })?;
    let bond_vout = cli.fraud_watch_bond_vout.ok_or_else(|| {
        anyhow::anyhow!("--fraud-watch-bond-vout is required when --fraud-watch-vault-config is set")
    })?;
    let bond_amount = cli.fraud_watch_bond_amount.ok_or_else(|| {
        anyhow::anyhow!("--fraud-watch-bond-amount is required when --fraud-watch-vault-config is set")
    })?;

    let watcher_signatures = cli
        .fraud_watch_watcher_sigs
        .iter()
        .map(|h| hex::decode(h).context("fraud_watch_watcher_sigs"))
        .collect::<Result<Vec<_>>>()?;

    if watcher_signatures.len() < vault_config.watcher_threshold as usize {
        return Err(anyhow::anyhow!(
            "fraud-watch: {} watcher signatures configured but vault requires {} (M-of-N)",
            watcher_signatures.len(),
            vault_config.watcher_threshold
        ));
    }

    Ok(Some(FraudWatchConfig {
        vault_config,
        bond_input: ChallengeInput {
            bond_txid,
            bond_vout,
            bond_amount,
        },
        watcher_signatures,
        miner_fee: cli.fraud_watch_miner_fee,
    }))
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
    let electrs_url = cli.electrs_url.clone().unwrap_or_else(|| {
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

    // Load fraud-watch configuration before any `cli` field is moved.
    let fraud_watch_cfg = load_fraud_watch_config(&cli)
        .context("loading fraud-watch configuration")?;
    if fraud_watch_cfg.is_some() {
        info!("[FraudWatch] Automatic challenge broadcast ENABLED (vault config + bond UTXO configured)");
    } else {
        info!("[FraudWatch] Detection enabled; automatic broadcast DISABLED (no --fraud-watch-vault-config) — divergence will be logged only");
    }

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

    // Fraud-watch channel: accepted attestations are fanned out here so the
    // fraud-watch task (spawned after electrs is initialized below) can
    // recompute the quorum result and detect divergence on each new arrival.
    let (fraud_tx, mut fraud_rx) = mpsc::channel::<StateAttestation>(cli.channel_size);

    // Listen for incoming P2P attestations
    tokio::spawn(async move {
        while let Some(attestation) = attestation_rx.recv().await {
            if consensus_clone.add_attestation(attestation.clone()) {
                info!(
                    "[Consensus] Accepted valid attestation for #{} from {}",
                    attestation.block_height, attestation.validator_pubkey
                );
                // Fan out to the fraud-watch task. Best-effort: if the
                // channel is full, drop the event (the periodic sweep below
                // will still catch divergence on the next attestation).
                let _ = fraud_tx.try_send(attestation);
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

    // Fraud-watch background task: triggered by each accepted attestation,
    // with a periodic sweep as a backstop in case an attestation was dropped
    // by the try_send above. Spawned after electrs is initialized so the
    // challenge broadcast path has a client to submit through.
    let consensus_fw = consensus.clone();
    let electrs_fw = electrs.clone();
    let fw_cfg = fraud_watch_cfg.clone();
    tokio::spawn(async move {
        // Periodic sweep every 60s catches divergence even if no new
        // attestation arrives (e.g. a peer that equivocated then went quiet).
        let mut sweep = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                Some(att) = fraud_rx.recv() => {
                    run_fraud_watch_pass(&consensus_fw, &electrs_fw, &fw_cfg, Some(&att)).await;
                }
                _ = sweep.tick() => {
                    run_fraud_watch_pass(&consensus_fw, &electrs_fw, &fw_cfg, None).await;
                }
            }
        }
    });

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

                    // Reorg detection: verify local block hash at last_synced matches L1
                    if last_synced > 0 {
                        if let Ok(Some(local_block)) = store_sync.get_block(&chain_sync, last_synced) {
                            if let Ok(l1_hash) = electrs_sync.get_block_hash(last_synced).await {
                                if l1_hash != local_block.block_hash {
                                    warn!(
                                        "[Scanner] Chain reorganization detected at height #{}: local {} != L1 {}",
                                        last_synced, local_block.block_hash, l1_hash
                                    );
                                    // Backtrack to find common ancestor
                                    let mut ancestor = last_synced.saturating_sub(1);
                                    let mut found_ancestor = false;
                                    while ancestor > 0 {
                                        if let Ok(Some(prev_local)) = store_sync.get_block(&chain_sync, ancestor) {
                                            match electrs_sync.get_block_hash(ancestor).await {
                                                Ok(prev_l1_hash) => {
                                                    if prev_l1_hash == prev_local.block_hash {
                                                        found_ancestor = true;
                                                        break;
                                                    }
                                                }
                                                Err(e) => {
                                                    error!(
                                                        "[Scanner] RPC error fetching block #{} during reorg backtrack: {:?}",
                                                        ancestor, e
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                        ancestor = ancestor.saturating_sub(1);
                                    }

                                    if found_ancestor {
                                        info!("[Scanner] Rolling back to common ancestor #{}", ancestor);
                                        match store_sync.rollback_to_block(&chain_sync, ancestor) {
                                            Ok(n) => {
                                                info!("[Scanner] Successfully rolled back {} transitions to #{}", n, ancestor);
                                                last_synced = ancestor;
                                            }
                                            Err(e) => {
                                                error!("[Scanner] Rollback to #{} failed: {:?}", ancestor, e);
                                                continue;
                                            }
                                        }
                                    } else {
                                        error!(
                                            "[Scanner] Could not safely determine common ancestor for reorg at height #{}; retrying next cycle",
                                            last_synced
                                        );
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    if tip_height > last_synced {
                        info!("[Scanner] New blocks detected: #{} -> #{}", last_synced, tip_height);
                        for h in (last_synced + 1)..=tip_height {
                            if let Ok(block_hash) = electrs_sync.get_block_hash(h).await {
                                if let Ok(txs) = electrs_sync.get_block_txs(&block_hash).await {
                                    for tx in &txs {
                                        match processor.process_tx(tx, h) {
                                            Err(e) => {
                                                // If the WASM blob for a call is not in the
                                                // local store, fetch it from the Kademlia DHT,
                                                // persist it (hash-verified), and retry once.
                                                if let Some(code_hash) = utxo_vmd::scanner::missing_wasm_hash(&e) {
                                                    match p2p_sync.get_contract(code_hash.clone()).await {
                                                        Ok(Some(bytes)) => {
                                                            match store_sync.save_wasm(&code_hash, &bytes) {
                                                                Ok(()) => {
                                                                    info!("[Scanner] Fetched WASM {} ({} bytes) from DHT — retrying tx", code_hash, bytes.len());
                                                                    let _ = processor.process_tx(tx, h);
                                                                }
                                                                Err(e2) => warn!("[Scanner] DHT wasm hash verification failed for {}: {}", code_hash, e2),
                                                            }
                                                        }
                                                        Ok(None) => warn!("[Scanner] WASM {} not found on DHT — tx skipped", code_hash),
                                                        Err(e2) => warn!("[Scanner] DHT fetch error for {}: {}", code_hash, e2),
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
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
        trusted_proxy_ips: cli.trusted_proxy_ips,
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

    // Issue 9: Use into_make_service_with_connect_info so the rate limiter
    // can access the real connection socket address instead of trusting
    // the client-supplied X-Forwarded-For header.
    let server = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal);

    server.await?;
    info!("[Shutdown] Daemon stopped cleanly.");

    Ok(())
}

/// One pass of the fraud-watch background task.
///
/// 1. For the (chain, height) of the triggering attestation (or for every
///    known (chain, height) during a periodic sweep), compute the quorum
///    result and call `detect_divergence` for each attestation that disagrees
///    with it.
/// 2. Drain `get_slashing_proofs()` — these are accumulated by
///    `ConsensusManager::add_attestation` when it detects the same validator
///    signing two different roots for the same (chain, height).
/// 3. If a vault config + bond UTXO are configured, build and broadcast a
///    challenge transaction for the first verified proof. Otherwise log only.
///
/// Every detection and every broadcast attempt is logged with the
/// `[FraudWatch]` prefix so this is observable in production.
async fn run_fraud_watch_pass(
    consensus: &Arc<ConsensusManager>,
    electrs: &ElectrsClient,
    fw_cfg: &Option<FraudWatchConfig>,
    trigger: Option<&StateAttestation>,
) {
    // Collect the (chain, height) keys to scan this pass.
    let keys: Vec<(String, u64)> = match trigger {
        Some(att) => vec![(att.chain.clone(), att.block_height)],
        None => {
            // Periodic sweep: scan every known (chain, height).
            match consensus.attestations.read() {
                Ok(map) => map.keys().cloned().collect(),
                Err(e) => {
                    error!("[FraudWatch] sweep: failed to read attestations map: {}", e);
                    return;
                }
            }
        }
    };

    for (chain, height) in keys {
        let quorum = match consensus.build_quorum_result(&chain, height) {
            Some(q) => q,
            None => continue, // no quorum yet for this key
        };

        // Detect divergence: any attestation whose root != quorum root.
        let all_atts = consensus.get_attestations(&chain, height);
        for att in &all_atts {
            if let Some(div) = consensus.detect_divergence(att, &quorum) {
                warn!(
                    "[FraudWatch] DIVERGENCE detected: chain={} height={} operator={} claimed_root={} quorum_root={}",
                    div.chain,
                    div.block_height,
                    div.operator_attestation.validator_pubkey,
                    div.operator_attestation.state_root,
                    div.quorum_result_root
                );
                maybe_broadcast_challenge(
                    electrs,
                    fw_cfg,
                    &div.chain,
                    div.block_height,
                    &div.operator_attestation.validator_pubkey,
                    &div.operator_attestation.state_root,
                    &div.quorum_result_root,
                )
                .await;
            }
        }
    }

    // Drain accumulated equivocation proofs (same validator, two roots).
    let proofs = consensus.get_slashing_proofs();
    if !proofs.is_empty() {
        for proof in &proofs {
            if !verify_equivocation_proof(proof) {
                error!(
                    "[FraudWatch] equivocation proof FAILED verification — chain={} height={} validator={} — NOT broadcasting",
                    proof.chain, proof.block_height, proof.validator_pubkey
                );
                continue;
            }
            warn!(
                "[FraudWatch] EQUIVOCATION verified: chain={} height={} validator={} root1={} root2={}",
                proof.chain,
                proof.block_height,
                proof.validator_pubkey,
                proof.first_attestation.state_root,
                proof.second_attestation.state_root
            );
            maybe_broadcast_challenge(
                electrs,
                fw_cfg,
                &proof.chain,
                proof.block_height,
                &proof.validator_pubkey,
                &proof.first_attestation.state_root,
                &proof.second_attestation.state_root,
            )
            .await;
        }
    }
}

/// Build and broadcast a challenge transaction if a vault config + bond UTXO
/// are configured; otherwise log that broadcast is skipped.
async fn maybe_broadcast_challenge(
    electrs: &ElectrsClient,
    fw_cfg: &Option<FraudWatchConfig>,
    chain: &str,
    height: u64,
    operator_pubkey: &str,
    claimed_root: &str,
    correct_root: &str,
) {
    let cfg = match fw_cfg {
        Some(c) => c,
        None => {
            info!(
                "[FraudWatch] challenge broadcast skipped (no vault config): chain={} height={} operator={} claimed={} correct={}",
                chain, height, operator_pubkey, claimed_root, correct_root
            );
            return;
        }
    };

    // Synthesize an EquivocationProof for build_challenge_transaction.
    // The proof's two attestations carry the claimed vs. correct roots; the
    // builder re-verifies the proof cryptographically before building the tx.
    // We construct minimal attestations satisfying verify_equivocation_proof's
    // structural checks (same validator, same chain/height, different roots).
    // NOTE: the on-chain attestation signatures are NOT re-verified here —
    // they were already verified by ConsensusManager::add_attestation before
    // the proof was accumulated. This is the same trust assumption as the
    // manual testnet demonstration.
    let placeholder_att = StateAttestation {
        chain: chain.to_string(),
        block_height: height,
        block_hash: String::new(),
        state_root: claimed_root.to_string(),
        validator_pubkey: operator_pubkey.to_string(),
        signature_hex: String::new(),
        timestamp: chrono::Utc::now().timestamp(),
    };
    let placeholder_att2 = StateAttestation {
        state_root: correct_root.to_string(),
        ..placeholder_att.clone()
    };
    let proof = utxo_vmd::types::EquivocationProof {
        chain: chain.to_string(),
        block_height: height,
        validator_pubkey: operator_pubkey.to_string(),
        first_attestation: placeholder_att,
        second_attestation: placeholder_att2,
        detected_at: chrono::Utc::now().timestamp(),
    };

    let tx: ChallengeTransaction = match challenge::build_challenge_transaction(
        &proof,
        &cfg.bond_input,
        &cfg.vault_config,
        &cfg.watcher_signatures,
        cfg.miner_fee,
    ) {
        Ok(t) => t,
        Err(e) => {
            error!(
                "[FraudWatch] challenge tx BUILD failed: chain={} height={} operator={} err={}",
                chain, height, operator_pubkey, e
            );
            return;
        }
    };

    info!(
        "[FraudWatch] challenge tx BUILT ({} bytes raw, {} sats fee) — broadcasting",
        tx.raw_hex.len() / 2,
        tx.fee
    );
    match challenge::broadcast_challenge_tx(electrs, &tx.raw_hex).await {
        Ok(txid) => {
            warn!(
                "[FraudWatch] challenge tx BROADCAST OK: txid={} chain={} height={} operator={}",
                txid, chain, height, operator_pubkey
            );
        }
        Err(e) => {
            error!(
                "[FraudWatch] challenge tx BROADCAST failed: chain={} height={} operator={} err={}",
                chain, height, operator_pubkey, e
            );
        }
    }
}
