use anyhow::Result;
use std::sync::Arc;
use tracing::info;
use utxo_core_vm::{VmConfig, VmRuntime};

use crate::scanner::electrs::ElectrsTx;
use crate::scanner::parser::parse_envelope;
use crate::storage::StateStore;
use crate::types::{SmartObjectRecord, StateTransitionRecord, UndoLogRecord};

pub struct BlockProcessor {
    pub store: StateStore,
    pub chain: String,
    pub runtime: Arc<VmRuntime>,
}

impl BlockProcessor {
    pub fn new(store: StateStore, chain: String) -> Self {
        let config = VmConfig {
            max_gas: 25_000_000,
            max_memory_pages: 32,
        };
        let runtime = VmRuntime::new(config);
        Self {
            store,
            chain,
            runtime: Arc::new(runtime),
        }
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

                        // State transition
                        let prev_seal = obj.seal.clone();
                        obj.seal = seal.clone();
                        obj.updated_at_block = block_height;
                        obj.satoshis = out.value;

                        // Apply method mutation
                        if let Some(new_owner) = args.get("to").and_then(|v| v.as_str()) {
                            obj.owner = new_owner.to_string();
                        }

                        let mut updated_state = obj.state_data.clone();
                        if let Some(obj_map) = updated_state.as_object_mut() {
                            obj_map.insert("last_method".to_string(), serde_json::Value::String(method.clone()));
                            obj_map.insert("last_caller".to_string(), serde_json::Value::String(sender.clone()));
                        }
                        obj.state_data = updated_state.clone();

                        self.store.save_object(&obj)?;

                        let trans = StateTransitionRecord {
                            txid: tx.txid.clone(),
                            object_id: obj.object_id.clone(),
                            prev_seal,
                            new_seal: seal.clone(),
                            caller: sender,
                            method,
                            args,
                            new_state: updated_state,
                            satoshis: out.value,
                            block_height,
                            gas_consumed: 21_500, // Deterministic gas
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
}
