use anyhow::{Result, anyhow};
use std::sync::Arc;
use tracing::{info, warn};
use utxo_core_vm::{VmConfig, VmRuntime};

use crate::scanner::electrs::ElectrsTx;
use crate::scanner::parser::parse_envelope;
use crate::storage::StateStore;
use crate::types::{SmartObjectRecord, StateTransitionRecord, UndoLogRecord};

pub struct BlockProcessor {
    pub store: StateStore,
    pub chain: String,
    pub runtime: Arc<VmRuntime>,
    pub min_execution_fee: u64,
    pub fee_collector: Option<String>,
}

impl BlockProcessor {
    pub fn new(
        store: StateStore,
        chain: String,
        min_execution_fee: u64,
        fee_collector: Option<String>,
    ) -> Self {
        let config = VmConfig {
            max_gas: 25_000_000,
            max_memory_pages: 32,
        };
        let runtime = VmRuntime::new(config);
        Self {
            store,
            chain,
            runtime: Arc::new(runtime),
            min_execution_fee,
            fee_collector,
        }
    }

    /// Validate that the transaction includes required micro-fee output.
    /// Fee validation ensures at least one output carries value >= min_execution_fee.
    /// When fee_collector is set, the output's scriptpubkey (ASM) must contain
    /// the fee collector address to confirm payment destination.
    fn validate_fee(&self, tx: &ElectrsTx) -> bool {
        if self.min_execution_fee == 0 {
            return true;
        }

        for out in &tx.vout {
            if out.value < self.min_execution_fee {
                continue;
            }

            if let Some(ref collector) = self.fee_collector {
                // Match scriptpubkey_asm or scriptpubkey against the collector address
                if let Some(ref asm) = out.scriptpubkey_asm {
                    if asm.contains(collector) {
                        return true;
                    }
                }
                // Fallback: check the raw scriptpubkey string
                if out.scriptpubkey.contains(collector) {
                    return true;
                }
            } else {
                // No specific collector; any output with sufficient value qualifies
                return true;
            }
        }

        false
    }

    pub fn process_tx(&self, tx: &ElectrsTx, block_height: u64) -> Result<Option<String>> {
        // Look for inscription envelope in outputs
        for (vout_idx, out) in tx.vout.iter().enumerate() {
            let script_hex = out.scriptpubkey_hex.as_deref().unwrap_or(&out.scriptpubkey);
            let script_bytes = match hex::decode(script_hex) {
                Ok(b) => b,
                Err(_) => continue,
            };

            if let Some(envelope) = parse_envelope(&script_bytes) {
                let seal = format!("{}:{}", tx.txid, vout_idx);
                let sender = if !tx.vin.is_empty() {
                    format!("{}:{}", tx.vin[0].txid, tx.vin[0].vout)
                } else {
                    "coinbase".to_string()
                };

                let metadata = envelope.metadata.unwrap_or_default();
                let op = metadata.get("op").and_then(|v| v.as_str()).unwrap_or("call");

                if op == "create" || op == "deploy" {
                    // Fee validation for create/deploy
                    if !self.validate_fee(tx) {
                        warn!(
                            "[Processor] Rejected create/deploy: insufficient fee (required: {} sats)",
                            self.min_execution_fee
                        );
                        return Ok(None);
                    }

                    let obj_id = format!("obj_{}", &tx.txid[..16]);
                    let code_hash = metadata
                        .get("code_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("wasm_default")
                        .to_string();

                    let initial_state = metadata.get("state").cloned().unwrap_or(serde_json::json!({
                        "initialized": true,
                        "owner": sender,
                    }));

                    let record = SmartObjectRecord {
                        object_id: obj_id.clone(),
                        code_hash: code_hash.clone(),
                        seal: seal.clone(),
                        satoshis: out.value,
                        owner: sender.clone(),
                        state_data: initial_state,
                        created_at_block: block_height,
                        updated_at_block: block_height,
                    };

                    // Record undo log for reorg safety
                    let undo = UndoLogRecord {
                        id: (block_height << 32) | (vout_idx as u64),
                        chain: self.chain.clone(),
                        block_height,
                        object_id: obj_id.clone(),
                        action: "CREATE".to_string(),
                        prev_code_hash: None,
                        prev_seal: None,
                        prev_satoshis: None,
                        prev_owner: None,
                        prev_state_data: None,
                        prev_updated_at_block: None,
                    };
                    self.store.save_undo_log(&undo)?;
                    self.store.save_object(&record)?;

                    info!(
                        "[Processor] Created Smart Object {} (seal: {}) at height #{}",
                        obj_id, seal, block_height
                    );
                    return Ok(Some(obj_id));
                } else {
                    // Fee validation for call operations
                    if !self.validate_fee(tx) {
                        warn!(
                            "[Processor] Rejected call: insufficient fee (required: {} sats)",
                            self.min_execution_fee
                        );
                        return Ok(None);
                    }

                    // Call method on smart object
                    // Identify spent seal from inputs
                    let mut target_obj = None;
                    for vin in &tx.vin {
                        let in_seal = format!("{}:{}", vin.txid, vin.vout);
                        if let Ok(Some(obj)) = self.store.get_object_by_seal(&in_seal) {
                            target_obj = Some(obj);
                            break;
                        }
                    }

                    if let Some(mut obj) = target_obj {
                        let method = metadata
                            .get("method")
                            .and_then(|v| v.as_str())
                            .unwrap_or("transfer")
                            .to_string();
                        let args = metadata.get("args").cloned().unwrap_or(serde_json::json!({}));

                        // Record undo log before updating
                        let undo = UndoLogRecord {
                            id: (block_height << 32) | (vout_idx as u64),
                            chain: self.chain.clone(),
                            block_height,
                            object_id: obj.object_id.clone(),
                            action: "UPDATE".to_string(),
                            prev_code_hash: Some(obj.code_hash.clone()),
                            prev_seal: Some(obj.seal.clone()),
                            prev_satoshis: Some(obj.satoshis),
                            prev_owner: Some(obj.owner.clone()),
                            prev_state_data: Some(obj.state_data.clone()),
                            prev_updated_at_block: Some(obj.updated_at_block),
                        };
                        self.store.save_undo_log(&undo)?;

                        // Execute WASM if code_hash is available (not default placeholder)
                        let (new_state, gas_consumed) = if obj.code_hash != "wasm_default" {
                            match self.execute_wasm(&obj, &method, &args, &sender, out.value) {
                                Ok(result) => (result.state_data, result.gas_consumed),
                                Err(e) => {
                                    warn!(
                                        "[Processor] WASM execution failed for {}: {}",
                                        obj.object_id, e
                                    );
                                    // Fallback to simple state mutation
                                    let mut state = obj.state_data.clone();
                                    if let Some(obj_map) = state.as_object_mut() {
                                        obj_map.insert("last_method".to_string(), serde_json::Value::String(method.clone()));
                                        obj_map.insert("last_caller".to_string(), serde_json::Value::String(sender.clone()));
                                    }
                                    (state, 21_500)
                                }
                            }
                        } else {
                            // No WASM bytecode, simple state mutation
                            let mut state = obj.state_data.clone();
                            if let Some(obj_map) = state.as_object_mut() {
                                obj_map.insert("last_method".to_string(), serde_json::Value::String(method.clone()));
                                obj_map.insert("last_caller".to_string(), serde_json::Value::String(sender.clone()));
                            }
                            (state, 21_500)
                        };

                        // State transition
                        let prev_seal = obj.seal.clone();
                        obj.seal = seal.clone();
                        obj.updated_at_block = block_height;
                        obj.satoshis = out.value;

                        // Apply method mutation for simple transfers
                        if let Some(new_owner) = args.get("to").and_then(|v| v.as_str()) {
                            obj.owner = new_owner.to_string();
                        }

                        obj.state_data = new_state.clone();

                        self.store.save_object(&obj)?;

                        let trans = StateTransitionRecord {
                            txid: tx.txid.clone(),
                            object_id: obj.object_id.clone(),
                            prev_seal,
                            new_seal: seal.clone(),
                            caller: sender,
                            method,
                            args,
                            new_state,
                            satoshis: out.value,
                            block_height,
                            gas_consumed,
                            timestamp: chrono::Utc::now().timestamp(),
                        };
                        self.store.save_transition(&trans)?;

                        info!(
                            "[Processor] Executed method on Object {} -> New seal {}",
                            obj.object_id, seal
                        );
                        return Ok(Some(obj.object_id));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Execute WASM contract via core-vm runtime
    fn execute_wasm(
        &self,
        obj: &SmartObjectRecord,
        method: &str,
        args: &serde_json::Value,
        caller: &str,
        satoshis: u64,
    ) -> Result<WasmExecutionResult> {
        use utxo_core_vm::state::{SingleUseSeal, SmartObjectState};

        // Parse seal from object (txid:vout format)
        let seal_parts: Vec<&str> = obj.seal.split(':').collect();
        let seal = SingleUseSeal {
            txid: seal_parts.first().unwrap_or(&"").to_string(),
            vout: seal_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        };

        // Build SmartObjectState for core-vm
        let state = SmartObjectState {
            object_id: obj.object_id.clone(),
            code_hash: obj.code_hash.clone(),
            seal: seal.clone(),
            satoshis,
            owner_pubkey: obj.owner.clone(),
            state_data: serde_json::to_vec(&obj.state_data).unwrap_or_default(),
        };

        // Get WASM bytecode from DHT or use placeholder
        let wasm_bytes = self.get_wasm_bytes(&obj.code_hash)?;

        // Execute via runtime
        let exec_result = self.runtime.execute(
            &wasm_bytes,
            &state,
            caller.to_string(),
            method,
            args.to_string().as_bytes(),
        ).map_err(|e| anyhow!("VM execution failed: {}", e))?;

        // Parse new state from execution result
        let new_state: serde_json::Value = serde_json::from_slice(&exec_result.updated_state_data)
            .unwrap_or(obj.state_data.clone());

        Ok(WasmExecutionResult {
            state_data: new_state,
            gas_consumed: exec_result.gas_consumed,
        })
    }

    /// Retrieve WASM bytecode - from DHT or empty if not found
    fn get_wasm_bytes(&self, code_hash: &str) -> Result<Vec<u8>> {
        // For now, return empty bytes if code_hash is a placeholder
        // In production, this would fetch from Kademlia DHT
        if code_hash == "wasm_default" || code_hash.is_empty() {
            Ok(Vec::new())
        } else {
            // Attempt to decode as hex (actual WASM bytes stored in code_hash field)
            hex::decode(code_hash).map_err(|e| anyhow::anyhow!("Invalid WASM hex: {}", e))
        }
    }
}

struct WasmExecutionResult {
    state_data: serde_json::Value,
    gas_consumed: u64,
}
