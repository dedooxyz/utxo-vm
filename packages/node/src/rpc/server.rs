use std::sync::Arc;
use std::time::Instant;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::{middleware, Router};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tower_http::cors::CorsLayer;

use crate::consensus::ConsensusManager;
use crate::cross_chain::{BridgeManager, CrossChainProof, StateRelay};
use crate::p2p::P2pHandle;
use crate::storage::StateStore;

const MAX_WASM_SIZE: usize = 1024 * 1024;
const MAX_BROADCAST_SIZE: usize = 1024 * 1024;
const RATE_LIMIT_WINDOW_SECS: u64 = 1;

#[derive(Clone)]
pub struct AppState {
    pub store: StateStore,
    pub p2p: P2pHandle,
    pub consensus: Arc<ConsensusManager>,
    pub chain: String,
    pub electrs_url: String,
    pub rate_limit_rps: u64,
    pub bridge: Arc<BridgeManager>,
    pub relay: Arc<StateRelay>,
    /// Per-IP rate limiter built from rate_limit_rps CLI arg
    pub rate_limiter: Arc<RateLimiter>,
    /// API key for bridge write endpoints (lock/mint). If None, bridge writes are disabled.
    pub bridge_api_key: Option<String>,
}

/// F1.4: Verify bridge API key from Authorization header.
/// Returns Ok(()) if valid, Err with status code if missing/invalid.
fn verify_bridge_auth(
    headers: &axum::http::HeaderMap,
    expected_key: &Option<String>,
) -> Result<(), (StatusCode, String)> {
    match expected_key {
        None => Err((
            StatusCode::FORBIDDEN,
            "Bridge write endpoints are disabled (no API key configured)".to_string(),
        )),
        Some(expected) => {
            let auth_header = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    (
                        StatusCode::UNAUTHORIZED,
                        "Missing Authorization header".to_string(),
                    )
                })?;

            let token = auth_header
                .strip_prefix("Bearer ")
                .ok_or_else(|| {
                    (
                        StatusCode::UNAUTHORIZED,
                        "Authorization header must use Bearer scheme".to_string(),
                    )
                })?;

            if token != expected {
                return Err((
                    StatusCode::FORBIDDEN,
                    "Invalid API key".to_string(),
                ));
            }
            Ok(())
        }
    }
}

fn validate_hex(s: &str, name: &str) -> Result<(), (StatusCode, String)> {
    if s.is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("{} cannot be empty", name)));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err((StatusCode::BAD_REQUEST, format!("{} must be valid hex", name)));
    }
    Ok(())
}

pub struct RateLimiter {
    windows: Mutex<HashMap<String, (Instant, usize)>>,
    max_requests: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u64) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            max_requests,
        }
    }

    pub fn check(&self, key: &str) -> bool {
        let mut windows = match self.windows.lock() {
            Ok(w) => w,
            Err(_) => return true, // Poisoned lock, fail open
        };
        let now = Instant::now();
        let entry = windows.entry(key.to_string()).or_insert((now, 0));

        if now.duration_since(entry.0).as_secs() >= RATE_LIMIT_WINDOW_SECS {
            *entry = (now, 1);
            true
        } else if (entry.1 as u64) < self.max_requests {
            entry.1 += 1;
            true
        } else {
            false
        }
    }
}

async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    // Extract client IP from X-Forwarded-For or socket address
    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if state.rate_limiter.check(&ip) {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"))
    }
}

pub fn create_router(state: AppState) -> Router {
    // CORS: Allow configured origins or default to same-origin
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::list([
            "http://localhost:3000".parse().unwrap(),
            "http://localhost:9773".parse().unwrap(),
            "http://127.0.0.1:3000".parse().unwrap(),
            "http://127.0.0.1:9773".parse().unwrap(),
        ]))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    Router::new()
        .route("/api/v1/chain/info", get(get_chain_info))
        .route("/api/v1/object/:id", get(get_object))
        .route("/api/v1/object/:id/proof", get(get_object_proof))
        .route("/api/v1/object/:id/history", get(get_object_history))
        .route("/api/v1/objects", get(get_all_objects))
        .route("/api/v1/state-root", get(get_latest_state_root))
        .route("/api/v1/state-root/:height", get(get_state_root_at_height))
        .route("/api/v1/broadcast", post(broadcast_transaction))
        .route("/api/v1/p2p/peers", get(get_p2p_peers))
        .route("/api/v1/dht/contract", post(put_dht_contract))
        .route("/api/v1/dht/contract/:code_hash", get(get_dht_contract))
        .route("/api/v1/consensus/slashing-proofs", get(get_slashing_proofs))
        .route("/api/v1/bridge/lock", post(lock_assets))
        .route("/api/v1/bridge/mint", post(mint_from_proof))
        .route("/api/v1/bridge/cancel", post(cancel_transfer))
        .route("/api/v1/bridge/transfer/:id", get(get_transfer))
        .route("/api/v1/bridge/transfers", get(get_all_transfers))
        .route("/api/v1/relay/create", post(create_relay))
        .route("/api/v1/relay/verify/:id", post(verify_relay))
        .route("/api/v1/relay/latest/:chain", get(get_latest_relay))
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware))
        .layer(cors)
        .with_state(state)
}

async fn get_chain_info(State(state): State<AppState>) -> impl IntoResponse {
    let last_height = state.store.get_last_sync_block(&state.chain).unwrap_or(0);
    let state_root = state.store.current_state_root();
    let peers = state.p2p.get_peers().await.unwrap_or_default();

    Json(serde_json::json!({
        "chain": state.chain,
        "electrs": state.electrs_url,
        "blockHeight": last_height,
        "stateRoot": state_root,
        "protocolVersion": 1,
        "vm": "utxo-core-vm",
        "p2pPeerId": state.p2p.local_peer_id,
        "p2pPeersCount": peers.len(),
        "status": "synced"
    }))
}

async fn get_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if id.is_empty() || id.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "Invalid object ID".to_string()));
    }

    let obj = state
        .store
        .get_object(&id)
        .or_else(|_| state.store.get_object_by_seal(&id))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(o) = obj {
        let value = serde_json::to_value(o)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Serialization error: {}", e)))?;
        Ok(Json(value))
    } else {
        Err((StatusCode::NOT_FOUND, "Smart Object not found".to_string()))
    }
}

async fn get_object_proof(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if id.is_empty() || id.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "Invalid object ID".to_string()));
    }

    let obj = state
        .store
        .get_object(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(o) = obj {
        let proof = state.store.get_state_proof(&o.object_id);
        Ok(Json(serde_json::json!({
            "object": o,
            "merkleProof": proof,
            "verified": proof.as_ref().map(|p| p.verified).unwrap_or(false)
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Smart Object not found".to_string()))
    }
}

async fn get_object_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if id.is_empty() || id.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "Invalid object ID".to_string()));
    }

    let all_transitions = state
        .store
        .get_transitions_for_object(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Apply pagination
    let page: usize = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(0);
    let limit: usize = params.get("limit").and_then(|p| p.parse().ok()).unwrap_or(100).min(1000);

    let total = all_transitions.len();
    let start = page * limit;
    let transitions: Vec<_> = if start < total {
        all_transitions.into_iter().skip(start).take(limit).collect()
    } else {
        Vec::new()
    };

    Ok(Json(serde_json::json!({
        "total": total,
        "page": page,
        "limit": limit,
        "transitions": transitions
    })))
}

async fn get_all_objects(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let all_objects = state
        .store
        .get_all_objects()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Apply pagination
    let page: usize = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(0);
    let limit: usize = params.get("limit").and_then(|p| p.parse().ok()).unwrap_or(100).min(1000);

    let total = all_objects.len();
    let start = page * limit;
    let objects: Vec<_> = if start < total {
        all_objects.into_iter().skip(start).take(limit).collect()
    } else {
        Vec::new()
    };

    Ok(Json(serde_json::json!({
        "total": total,
        "page": page,
        "limit": limit,
        "objects": objects
    })))
}

async fn get_latest_state_root(State(state): State<AppState>) -> impl IntoResponse {
    let last_height = state.store.get_last_sync_block(&state.chain).unwrap_or(0);
    let state_root = state.store.current_state_root();

    Json(serde_json::json!({
        "chain": state.chain,
        "blockHeight": last_height,
        "stateRoot": state_root
    }))
}

async fn get_state_root_at_height(
    State(state): State<AppState>,
    Path(height): Path<u64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if height > 10_000_000 {
        return Err((StatusCode::BAD_REQUEST, "Block height too large".to_string()));
    }

    let block = state
        .store
        .get_block(height)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let attestations = state.consensus.get_attestations(&state.chain, height);

    if let Some(b) = block {
        Ok(Json(serde_json::json!({
            "chain": state.chain,
            "blockHeight": height,
            "blockHash": b.block_hash,
            "stateRoot": b.state_root,
            "attestations": attestations,
            "quorumReached": state.consensus.is_quorum_reached(&state.chain, height, &b.state_root)
        })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Block #{} not found", height)))
    }
}

#[derive(Deserialize)]
struct BroadcastRequest {
    pub payload_hex: String,
}

async fn broadcast_transaction(
    State(state): State<AppState>,
    Json(req): Json<BroadcastRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_hex(&req.payload_hex, "payload_hex")?;
    let bytes = hex::decode(&req.payload_hex)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid hex: {}", e)))?;

    if bytes.len() > MAX_BROADCAST_SIZE {
        return Err((StatusCode::BAD_REQUEST, format!("Payload too large: {} bytes (max {})", bytes.len(), MAX_BROADCAST_SIZE)));
    }

    state
        .p2p
        .broadcast_mempool(bytes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "broadcasted_to_p2p",
        "topic": "utxovm/mempool/v1"
    })))
}

async fn get_p2p_peers(State(state): State<AppState>) -> impl IntoResponse {
    let peers = state.p2p.get_peers().await.unwrap_or_default();
    Json(serde_json::json!({
        "peerCount": peers.len(),
        "peers": peers
    }))
}

#[derive(Deserialize)]
struct PutContractRequest {
    pub code_hash: String,
    pub wasm_hex: String,
}

async fn put_dht_contract(
    State(state): State<AppState>,
    Json(req): Json<PutContractRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_hex(&req.code_hash, "code_hash")?;
    validate_hex(&req.wasm_hex, "wasm_hex")?;

    let wasm_bytes = hex::decode(&req.wasm_hex)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid wasm hex: {}", e)))?;

    if wasm_bytes.len() > MAX_WASM_SIZE {
        return Err((StatusCode::BAD_REQUEST, format!("WASM too large: {} bytes (max {})", wasm_bytes.len(), MAX_WASM_SIZE)));
    }

    state
        .p2p
        .put_contract(req.code_hash.clone(), wasm_bytes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "published_to_dht",
        "codeHash": req.code_hash
    })))
}

async fn get_dht_contract(
    State(state): State<AppState>,
    Path(code_hash): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    validate_hex(&code_hash, "code_hash")?;

    let wasm_opt = state
        .p2p
        .get_contract(code_hash.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(wasm) = wasm_opt {
        Ok(Json(serde_json::json!({
            "codeHash": code_hash,
            "wasmHex": hex::encode(wasm)
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Contract bytecode not found in DHT".to_string()))
    }
}

async fn get_slashing_proofs(State(state): State<AppState>) -> impl IntoResponse {
    let proofs = state.consensus.get_slashing_proofs();
    Json(serde_json::json!({
        "count": proofs.len(),
        "proofs": proofs
    }))
}

// Cross-chain bridge endpoints

#[derive(Deserialize)]
struct LockRequest {
    source_chain: String,
    object_id: String,
    dest_chain: String,
    dest_owner: String,
    /// P0-fu.3: secp256k1 hex pubkey of the caller (must match object owner)
    caller_pubkey: String,
    /// P0-fu.3: secp256k1 compact hex signature over lock message
    signature: String,
}

async fn lock_assets(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<LockRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // F1.4: Verify API key before allowing bridge write
    verify_bridge_auth(&headers, &state.bridge_api_key)?;

    let transfer = state
        .bridge
        .lock_assets(
            &req.source_chain,
            &req.object_id,
            &req.dest_chain,
            &req.dest_owner,
            &req.caller_pubkey,
            &req.signature,
        )
        .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "locked",
        "transfer": transfer
    })))
}

#[derive(Deserialize)]
struct MintRequest {
    transfer_id: String,
    proof: CrossChainProof,
}

#[derive(Deserialize)]
struct CancelRequest {
    transfer_id: String,
    caller: String,
}

async fn cancel_transfer(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CancelRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    verify_bridge_auth(&headers, &state.bridge_api_key)?;

    let transfer = state
        .bridge
        .cancel_transfer(&req.transfer_id, &req.caller)
        .map_err(|e| {
            if e.to_string().contains("not owner") {
                (StatusCode::FORBIDDEN, e.to_string())
            } else if e.to_string().contains("timeout") {
                (StatusCode::CONFLICT, e.to_string())
            } else {
                (StatusCode::BAD_REQUEST, e.to_string())
            }
        })?;

    Ok(Json(serde_json::json!({
        "status": "cancelled",
        "transfer": transfer
    })))
}

async fn mint_from_proof(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<MintRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // F1.4: Verify API key before allowing bridge write
    verify_bridge_auth(&headers, &state.bridge_api_key)?;

    let transfer = state
        .bridge
        .mint_from_proof(&req.transfer_id, &req.proof)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "minted",
        "transfer": transfer
    })))
}

async fn get_transfer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let transfer = state
        .bridge
        .get_transfer(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match transfer {
        Some(t) => Ok(Json(serde_json::json!(t))),
        None => Err((StatusCode::NOT_FOUND, "Transfer not found".to_string())),
    }
}

async fn get_all_transfers(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let transfers = state
        .bridge
        .get_all_transfers()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "count": transfers.len(),
        "transfers": transfers
    })))
}

// State relay endpoints

#[derive(Deserialize)]
struct RelayRequest {
    source_chain: String,
    block_height: u64,
    dest_chain: String,
}

async fn create_relay(
    State(state): State<AppState>,
    Json(req): Json<RelayRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let message = state
        .relay
        .create_relay_message(&req.source_chain, req.block_height)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let record = state
        .relay
        .relay_to_chain(&message, &req.dest_chain)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "relayed",
        "record": record
    })))
}

async fn verify_relay(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let record = state
        .relay
        .verify_relay(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!(record)))
}

async fn get_latest_relay(
    State(state): State<AppState>,
    Path(chain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let record = state
        .relay
        .get_latest_relay(&chain)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match record {
        Some(r) => Ok(Json(serde_json::json!(r))),
        None => Err((StatusCode::NOT_FOUND, "No verified relay found".to_string())),
    }
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let last_height = state.store.get_last_sync_block(&state.chain).unwrap_or(0);
    Json(serde_json::json!({
        "status": "ok",
        "chain": state.chain,
        "block_height": last_height,
    }))
}

async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let last_height = state.store.get_last_sync_block(&state.chain).unwrap_or(0);
    let peers = state.p2p.get_peers().await.unwrap_or_default();
    let transfers = state.bridge.get_all_transfers().unwrap_or_default();
    let relay_count = state.relay.get_all_records().map(|r| r.len()).unwrap_or(0);

    let mut metrics = String::new();
    metrics.push_str("# HELP utxovm_block_height Current synced block height\n");
    metrics.push_str("# TYPE utxovm_block_height gauge\n");
    metrics.push_str(&format!("utxovm_block_height{{chain=\"{}\"}} {}\n", state.chain, last_height));
    metrics.push_str("# HELP utxovm_p2p_peers Connected P2P peers\n");
    metrics.push_str("# TYPE utxovm_p2p_peers gauge\n");
    metrics.push_str(&format!("utxovm_p2p_peers {}\n", peers.len()));
    metrics.push_str("# HELP utxovm_bridge_transfers Total bridge transfers\n");
    metrics.push_str("# TYPE utxovm_bridge_transfers gauge\n");
    metrics.push_str(&format!("utxovm_bridge_transfers {}\n", transfers.len()));
    metrics.push_str("# HELP utxovm_relay_count Total state relays\n");
    metrics.push_str("# TYPE utxovm_relay_count gauge\n");
    metrics.push_str(&format!("utxovm_relay_count {}\n", relay_count));

    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics,
    )
}

