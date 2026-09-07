use std::sync::Arc;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use tower_http::cors::{Any, CorsLayer};

use crate::consensus::ConsensusManager;
use crate::p2p::P2pHandle;
use crate::storage::StateStore;

#[derive(Clone)]
pub struct AppState {
    pub store: StateStore,
    pub p2p: P2pHandle,
    pub consensus: Arc<ConsensusManager>,
    pub chain: String,
    pub electrs_url: String,
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

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
        "p2pPort": 2232,
        "p2pPeersCount": peers.len(),
        "status": "synced"
    }))
}

async fn get_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let obj = state
        .store
        .get_object(&id)
        .or_else(|_| state.store.get_object_by_seal(&id))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(o) = obj {
        Ok(Json(serde_json::to_value(o).unwrap()))
    } else {
        Err((StatusCode::NOT_FOUND, "Smart Object not found".to_string()))
    }
}

async fn get_object_proof(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let transitions = state
        .store
        .get_transitions_for_object(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(transitions).unwrap()))
}

async fn get_all_objects(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let objects = state
        .store
        .get_all_objects()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::to_value(objects).unwrap()))
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
    let bytes = hex::decode(&req.payload_hex)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid hex: {}", e)))?;

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
    let wasm_bytes = hex::decode(&req.wasm_hex)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid wasm hex: {}", e)))?;

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
