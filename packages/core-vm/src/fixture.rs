//! Replayable execution fixtures — the "you are not the indexer" check.
//!
//! A fixture is a JSON document describing an ordered list of operations
//! (`deploy` / `call`). Replaying it executes every op against a fresh
//! `VmRuntime` and produces a canonical `state_root` over the final object
//! states. Two independent replays of the same fixture — including in two
//! separate processes or on different machines — MUST produce the same root.
//!
//! Fixture schema (version 1):
//! ```json
//! {
//!   "version": 1,
//!   "operations": [
//!     {"type": "deploy", "wasm_hex": "...", "object_id": "obj1",
//!      "caller": "alice", "seal_txid": "...", "seal_vout": 0,
//!      "satoshis": 1000, "init_args_hex": "..."},
//!     {"type": "call", "object_id": "obj1", "method": "transfer",
//!      "args_hex": "...", "caller": "alice",
//!      "seal_txid": "...", "seal_vout": 1, "satoshis": 0}
//!   ]
//! }
//! ```
//!
//! `deploy` ops default `object_id` to the WASM code hash if omitted.
//! `call` ops must reference an `object_id` created by an earlier `deploy`.
//! State is threaded automatically: each `call` receives the updated state
//! left by the previous op on that object.

use crate::runtime::{VmConfig, VmRuntime};
use crate::state::{SingleUseSeal, SmartObjectState};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A replayable operation inside a fixture.
#[derive(Debug, Deserialize)]
struct FixtureOp {
    #[serde(rename = "type")]
    op_type: String,
    wasm_hex: Option<String>,
    caller: Option<String>,
    seal_txid: Option<String>,
    seal_vout: Option<u32>,
    satoshis: Option<u64>,
    init_args_hex: Option<String>,
    method: Option<String>,
    args_hex: Option<String>,
    object_id: Option<String>,
}

/// Fixture: ordered list of operations to replay.
#[derive(Debug, Deserialize)]
struct Fixture {
    version: u32,
    operations: Vec<FixtureOp>,
}

/// Canonical state root over a sequence of final object states.
/// SHA256("UTXOVM_STATE_ROOT_V1" || for each state sorted by object_id:
///   len-prefixed object_id, len-prefixed state_data)
/// Deterministic regardless of insertion order.
pub fn compute_fixture_state_root(states: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = states.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    hasher.update(b"UTXOVM_STATE_ROOT_V1");
    for (object_id, state_data) in sorted {
        hasher.update(&(object_id.len() as u32).to_be_bytes());
        hasher.update(object_id.as_bytes());
        hasher.update(&(state_data.len() as u32).to_be_bytes());
        hasher.update(state_data);
    }
    hex::encode(hasher.finalize())
}

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).map_err(|e| format!("invalid hex: {}", e))
}

/// Replay a fixture from raw JSON bytes. Returns a JSON object with
/// `state_root`, `ops_executed`, `total_gas`, `object_count`,
/// `runtime_version`, `runtime_hash`.
pub fn verify_fixture(fixture_bytes: &[u8]) -> Result<serde_json::Value, String> {
    let fixture: Fixture = serde_json::from_slice(fixture_bytes)
        .map_err(|e| format!("invalid fixture JSON: {}", e))?;
    if fixture.version != 1 {
        return Err(format!("unsupported fixture version {}", fixture.version));
    }

    let runtime = VmRuntime::new(VmConfig::default());
    // object_id -> (state_data, wasm bytes for code-hash continuity)
    let mut objects: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();
    let mut total_gas: u64 = 0;
    let mut ops_executed = 0usize;

    for (i, op) in fixture.operations.iter().enumerate() {
        let ctx = |what: &str| format!("op[{}] {}", i, what);
        match op.op_type.as_str() {
            "deploy" => {
                let wasm = hex_to_bytes(
                    op.wasm_hex.as_deref().ok_or_else(|| ctx("missing wasm_hex"))?,
                )?;
                let object_id = op
                    .object_id
                    .clone()
                    .unwrap_or_else(|| VmRuntime::calculate_code_hash(&wasm));
                let seal = SingleUseSeal {
                    txid: op.seal_txid.clone().unwrap_or_else(|| "0".repeat(64)),
                    vout: op.seal_vout.unwrap_or(0),
                };
                let init_args = match &op.init_args_hex {
                    Some(h) => hex_to_bytes(h)?,
                    None => Vec::new(),
                };
                let res = runtime
                    .deploy(
                        &wasm,
                        op.caller.clone().unwrap_or_else(|| "unknown".into()),
                        seal,
                        op.satoshis.unwrap_or(0),
                        &init_args,
                    )
                    .map_err(|e| ctx(&e.to_string()))?;
                total_gas = total_gas.saturating_add(res.gas_consumed);
                objects.push((object_id, res.updated_state_data, wasm));
                ops_executed += 1;
            }
            "call" => {
                let object_id = op
                    .object_id
                    .clone()
                    .ok_or_else(|| ctx("missing object_id"))?;
                let idx = objects
                    .iter()
                    .position(|(id, _, _)| *id == object_id)
                    .ok_or_else(|| ctx(&format!("unknown object_id '{}'", object_id)))?;
                let (_, prior_state, wasm) = objects[idx].clone();
                let caller = op.caller.clone().unwrap_or_else(|| "unknown".into());
                let state = SmartObjectState {
                    object_id: object_id.clone(),
                    code_hash: VmRuntime::calculate_code_hash(&wasm),
                    seal: SingleUseSeal {
                        txid: op.seal_txid.clone().unwrap_or_else(|| "0".repeat(64)),
                        vout: op.seal_vout.unwrap_or(0),
                    },
                    satoshis: op.satoshis.unwrap_or(0),
                    owner_pubkey: caller.clone(),
                    state_data: prior_state,
                };
                let method_args = match &op.args_hex {
                    Some(h) => hex_to_bytes(h)?,
                    None => Vec::new(),
                };
                let res = runtime
                    .execute(
                        &wasm,
                        &state,
                        caller,
                        op.method.as_deref().ok_or_else(|| ctx("missing method"))?,
                        &method_args,
                    )
                    .map_err(|e| ctx(&e.to_string()))?;
                total_gas = total_gas.saturating_add(res.gas_consumed);
                objects[idx].1 = res.updated_state_data;
                ops_executed += 1;
            }
            other => return Err(ctx(&format!("unknown op type '{}'", other))),
        }
    }

    let states: Vec<(String, Vec<u8>)> =
        objects.into_iter().map(|(id, s, _)| (id, s)).collect();
    let root = compute_fixture_state_root(&states);
    let v = VmRuntime::version();

    Ok(serde_json::json!({
        "state_root": root,
        "ops_executed": ops_executed,
        "total_gas": total_gas,
        "object_count": states.len(),
        "runtime_version": v.version,
        "runtime_hash": v.runtime_hash,
    }))
}
