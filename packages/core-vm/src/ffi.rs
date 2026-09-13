//! C ABI (FFI) entry points for UTXO-VM.
//!
//! All functions take a JSON request as `(ptr, len)` and return a JSON
//! response as a `*mut c_char` which the caller MUST release with
//! `utxovm_free_string`. Panics never cross the FFI boundary — every
//! entry point is wrapped in `catch_unwind`.
//!
//! ABI version: 1 (frozen). Changing the request/response schema is
//! consensus-breaking and requires a coordinated rollout.

use crate::state::{SingleUseSeal, SmartObjectState};
use crate::runtime::{VmConfig, VmRuntime};
use serde::Serialize;
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Read a `&[u8]` from raw parts. Returns empty slice on null/0.
/// # Safety: caller must pass a valid (ptr, len) pair.
unsafe fn read_bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Serialize a result to a heap-allocated C string.
/// Never returns null — errors are encoded as JSON.
fn to_c_json<T: Serialize>(value: &T) -> *mut c_char {
    let json = serde_json::to_string(value)
        .unwrap_or_else(|e| format!(r#"{{"success":false,"error":"json_encode:{}"}}"#, e));
    CString::new(json).map_or(std::ptr::null_mut(), |s| s.into_raw())
}

#[derive(Debug, Serialize)]
struct FfiResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl FfiResponse {
    fn ok(result: serde_json::Value) -> Self {
        Self { success: true, result: Some(result), error: None }
    }
    fn err<E: ToString>(error: E) -> Self {
        Self { success: false, result: None, error: Some(error.to_string()) }
    }
}

fn run_ffi<F>(input_ptr: *const u8, input_len: usize, f: F) -> *mut c_char
where
    F: FnOnce(&serde_json::Value) -> Result<serde_json::Value, String>,
{
    let result = catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { read_bytes(input_ptr, input_len) };
        let json: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid JSON request: {}", e))?;
        f(&json)
    }));

    let resp = match result {
        Ok(Ok(v)) => FfiResponse::ok(v),
        Ok(Err(e)) => FfiResponse::err(e),
        Err(_) => FfiResponse::err("panic inside VM"),
    };
    to_c_json(&resp)
}

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).map_err(|e| format!("invalid hex: {}", e))
}

fn get_str<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("missing field '{}'", key))
}

fn get_str_or<'a>(v: &'a serde_json::Value, key: &str, default: &'a str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(default)
}

fn get_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn seal_from_json(v: &serde_json::Value) -> SingleUseSeal {
    SingleUseSeal {
        txid: get_str_or(v, "seal_txid", "0".repeat(64).as_str()).to_string(),
        vout: get_u64(v, "seal_vout") as u32,
    }
}

fn hex_field(v: &serde_json::Value, key: &str) -> Result<Vec<u8>, String> {
    let s = get_str_or(v, key, "");
    if s.is_empty() {
        Ok(Vec::new())
    } else {
        hex_to_bytes(s)
    }
}

fn exec_result_json(r: &crate::runtime::ExecutionResult) -> serde_json::Value {
    serde_json::json!({
        "gas_consumed": r.gas_consumed,
        "return_code": r.return_code,
        "updated_state_hex": hex::encode(&r.updated_state_data),
        "events": r.events.iter().map(|e| serde_json::json!({
            "topic": e.topic, "data": e.data,
        })).collect::<Vec<_>>(),
        "created_objects": r.created_objects.len(),
        "stealth_settlements": r.stealth_settlements.len(),
        "mweb_peg_outs": r.mweb_peg_outs.len(),
    })
}

fn do_deploy(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wasm_bytes = hex_to_bytes(get_str(args, "wasm_hex")?)?;
    let runtime = VmRuntime::new(VmConfig::default());
    let result = runtime
        .deploy(
            &wasm_bytes,
            get_str_or(args, "caller", "unknown").to_string(),
            seal_from_json(args),
            get_u64(args, "satoshis"),
            &hex_field(args, "init_args_hex")?,
        )
        .map_err(|e| e.to_string())?;
    Ok(exec_result_json(&result))
}

fn do_execute(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wasm_bytes = hex_to_bytes(get_str(args, "wasm_hex")?)?;
    let caller = get_str_or(args, "caller", "unknown").to_string();
    let state = SmartObjectState {
        object_id: get_str_or(args, "object_id", "").to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: seal_from_json(args),
        satoshis: get_u64(args, "satoshis"),
        owner_pubkey: caller.clone(),
        state_data: hex_field(args, "state_hex")?,
    };
    let runtime = VmRuntime::new(VmConfig::default());
    let result = runtime
        .execute(
            &wasm_bytes,
            &state,
            caller,
            get_str(args, "method")?,
            &hex_field(args, "args_hex")?,
        )
        .map_err(|e| e.to_string())?;
    Ok(exec_result_json(&result))
}

/// Deploy a contract. Request JSON schema:
/// `{wasm_hex, caller?, seal_txid?, seal_vout?, satoshis?, init_args_hex?}`
///
/// # Safety
/// `req_ptr` must point to `req_len` bytes of valid UTF-8 JSON (or be null with len=0).
/// Returned pointer must be freed with `utxovm_free_string`.
#[no_mangle]
pub extern "C" fn utxovm_deploy(req_ptr: *const u8, req_len: usize) -> *mut c_char {
    run_ffi(req_ptr, req_len, do_deploy)
}

/// Execute a method call on an existing object.
/// `{wasm_hex, state_hex?, caller?, method, args_hex?, seal_txid?, seal_vout?, satoshis?}`
///
/// # Safety
/// Same contract as `utxovm_deploy`.
#[no_mangle]
pub extern "C" fn utxovm_execute(req_ptr: *const u8, req_len: usize) -> *mut c_char {
    run_ffi(req_ptr, req_len, do_execute)
}

/// Replay a fixture: JSON `{version:1, operations:[{type:"deploy"|"call", ...}]}`.
/// Returns `{success, result:{state_root, ops_executed, total_gas, object_count,
/// runtime_version, runtime_hash}}`.
/// Two independent processes replaying the same fixture MUST return the same
/// `state_root` — this is the "you are not the indexer" check.
///
/// # Safety
/// Same contract as `utxovm_deploy`.
#[no_mangle]
pub extern "C" fn utxovm_verify(fixture_ptr: *const u8, fixture_len: usize) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { read_bytes(fixture_ptr, fixture_len) };
        crate::fixture::verify_fixture(bytes)
    }));
    let resp = match result {
        Ok(Ok(v)) => FfiResponse::ok(v),
        Ok(Err(e)) => FfiResponse::err(e),
        Err(_) => FfiResponse::err("panic inside VM"),
    };
    to_c_json(&resp)
}

/// Compute SHA-256 code hash of WASM bytes.
/// Request: raw wasm bytes (NOT JSON). Response JSON: `{success, result:{code_hash}}`.
///
/// # Safety
/// `wasm_ptr` must point to `wasm_len` valid bytes.
#[no_mangle]
pub extern "C" fn utxovm_code_hash(wasm_ptr: *const u8, wasm_len: usize) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { read_bytes(wasm_ptr, wasm_len) };
        if bytes.is_empty() {
            return Err("empty wasm input".to_string());
        }
        Ok::<_, String>(serde_json::json!({
            "code_hash": VmRuntime::calculate_code_hash(bytes)
        }))
    }));
    let resp = match result {
        Ok(Ok(v)) => FfiResponse::ok(v),
        Ok(Err(e)) => FfiResponse::err(e),
        Err(_) => FfiResponse::err("panic inside VM"),
    };
    to_c_json(&resp)
}

/// Return runtime version info as JSON string.
/// Caller frees with `utxovm_free_string`.
#[no_mangle]
pub extern "C" fn utxovm_runtime_version() -> *mut c_char {
    let result = catch_unwind(|| {
        let v = VmRuntime::version();
        FfiResponse::ok(serde_json::json!({
            "version": v.version,
            "wasmtime_version": v.wasmtime_version,
            "runtime_hash": v.runtime_hash,
        }))
    });
    let resp = result.unwrap_or_else(|_| FfiResponse::err("panic inside VM"));
    to_c_json(&resp)
}

/// Free a string previously returned by this library.
///
/// # Safety
/// `s` must be a pointer returned by one of the `utxovm_*` functions,
/// or null (no-op). Passing any other pointer is UB.
#[no_mangle]
pub unsafe extern "C" fn utxovm_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
