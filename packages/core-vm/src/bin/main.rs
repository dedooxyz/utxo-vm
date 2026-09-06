use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use utxo_core_vm::{VmConfig, VmRuntime, SmartObjectState, SingleUseSeal};

#[derive(Debug, Deserialize)]
struct VmRequest {
    command: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct VmResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl VmResponse {
    fn ok(result: serde_json::Value) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
        }
    }

    fn err(error: String) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(error),
        }
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    hex::decode(hex).map_err(|e| format!("Invalid hex: {}", e))
}

fn handle_deploy(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wasm_hex = args["wasm_hex"]
        .as_str()
        .ok_or("Missing wasm_hex")?;
    let wasm_bytes = hex_to_bytes(wasm_hex)?;

    let caller = args["caller"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let seal_txid = args["seal_txid"]
        .as_str()
        .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000")
        .to_string();
    let seal_vout = args["seal_vout"].as_u64().unwrap_or(0) as u32;

    let satoshis = args["satoshis"].as_u64().unwrap_or(0);

    let init_args_hex = args["init_args_hex"]
        .as_str()
        .unwrap_or("");
    let init_args = if init_args_hex.is_empty() {
        Vec::new()
    } else {
        hex_to_bytes(init_args_hex)?
    };

    let seal = SingleUseSeal {
        txid: seal_txid,
        vout: seal_vout,
    };

    let config = VmConfig::default();
    let runtime = VmRuntime::new(config);

    let result = runtime.deploy(
        &wasm_bytes,
        caller,
        seal,
        satoshis,
        &init_args,
    )?;

    Ok(serde_json::json!({
        "gas_consumed": result.gas_consumed,
        "return_code": result.return_code,
        "updated_state_hex": hex::encode(&result.updated_state_data),
        "events": result.events.iter().map(|e| {
            serde_json::json!({
                "topic": e.topic,
                "data": e.data,
            })
        }).collect::<Vec<_>>(),
        "created_objects": result.created_objects.len(),
        "stealth_settlements": result.stealth_settlements.len(),
    }))
}

fn handle_execute(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wasm_hex = args["wasm_hex"]
        .as_str()
        .ok_or("Missing wasm_hex")?;
    let wasm_bytes = hex_to_bytes(wasm_hex)?;

    let state_hex = args["state_hex"]
        .as_str()
        .unwrap_or("");
    let state_data = if state_hex.is_empty() {
        Vec::new()
    } else {
        hex_to_bytes(state_hex)?
    };

    let caller = args["caller"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let method = args["method"]
        .as_str()
        .ok_or("Missing method")?;

    let args_hex = args["args_hex"]
        .as_str()
        .unwrap_or("");
    let method_args = if args_hex.is_empty() {
        Vec::new()
    } else {
        hex_to_bytes(args_hex)?
    };

    let seal_txid = args["seal_txid"]
        .as_str()
        .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000")
        .to_string();
    let seal_vout = args["seal_vout"].as_u64().unwrap_or(0) as u32;

    let seal = SingleUseSeal {
        txid: seal_txid,
        vout: seal_vout,
    };

    let state = SmartObjectState {
        object_id: String::new(),
        code_hash: String::new(),
        seal,
        satoshis: 0,
        owner_pubkey: caller.clone(),
        state_data,
    };

    let config = VmConfig::default();
    let runtime = VmRuntime::new(config);

    let result = runtime.execute(
        &wasm_bytes,
        &state,
        caller,
        method,
        &method_args,
    )?;

    Ok(serde_json::json!({
        "gas_consumed": result.gas_consumed,
        "return_code": result.return_code,
        "updated_state_hex": hex::encode(&result.updated_state_data),
        "events": result.events.iter().map(|e| {
            serde_json::json!({
                "topic": e.topic,
                "data": e.data,
            })
        }).collect::<Vec<_>>(),
        "created_objects": result.created_objects.len(),
        "stealth_settlements": result.stealth_settlements.len(),
    }))
}

fn handle_code_hash(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wasm_hex = args["wasm_hex"]
        .as_str()
        .ok_or("Missing wasm_hex")?;
    let wasm_bytes = hex_to_bytes(wasm_hex)?;

    let code_hash = VmRuntime::calculate_code_hash(&wasm_bytes);

    Ok(serde_json::json!({
        "code_hash": code_hash,
    }))
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("Failed to read input");

    let request: VmRequest = serde_json::from_str(&input).expect("Failed to parse input");

    let response = match request.command.as_str() {
        "deploy" => handle_deploy(&request.args),
        "execute" => handle_execute(&request.args),
        "code_hash" => handle_code_hash(&request.args),
        _ => Err(format!("Unknown command: {}", request.command)),
    };

    let output = match response {
        Ok(result) => VmResponse::ok(result),
        Err(error) => VmResponse::err(error),
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
