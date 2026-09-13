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
        // Issue 13: Warn when min_execution_fee > 0 but no fee_collector is set.
        // Without a collector, any output with sufficient value satisfies the
        // fee check — including self-paid change. This provides minimal
        // anti-spam guarantee. Production/mainnet configs SHOULD set fee_collector.
        if min_execution_fee > 0 && fee_collector.is_none() {
            warn!(
                "[Processor] min_execution_fee={} but fee_collector is not set. \
                Self-paid outputs will satisfy the fee check, providing minimal \
                anti-spam guarantee. Set --fee-collector for production use.",
                min_execution_fee
            );
        }
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
    ///
    /// Issue 13: When `fee_collector` is set, the output's `scriptpubkey` (hex)
    /// must match the collector EXACTLY — not as a substring. Substring matching
    /// could false-positive on an unrelated script that coincidentally contains
    /// the collector address as a substring.
    ///
    /// Issue 13: When `fee_collector` is NOT set, any output with sufficient
    /// value qualifies — including a change output paid back to the sender.
    /// This provides little anti-spam guarantee beyond L1 dust limits.
    /// Production/mainnet configs SHOULD set `fee_collector`. A startup warning
    /// is logged when `min_execution_fee > 0` but `fee_collector` is unset.
    fn validate_fee(&self, tx: &ElectrsTx) -> bool {
        if self.min_execution_fee == 0 {
            return true;
        }

        for out in &tx.vout {
            if out.value < self.min_execution_fee {
                continue;
            }

            if let Some(ref collector) = self.fee_collector {
                // Exact match on scriptpubkey hex — not substring containment.
                if out.scriptpubkey == *collector {
                    return true;
                }
            } else {
                // No specific collector; any output with sufficient value qualifies.
                // WARNING: this includes self-paid change outputs and provides
                // minimal anti-spam guarantee. See Issue 13.
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
                // application/wasm payloads ARE deploys — the WASM bytecode is
                // the artifact being deployed. application/json envelopes use
                // metadata.op to distinguish "create"/"deploy" from "call".
                let op = if envelope.content_type == "application/wasm" {
                    "deploy"
                } else {
                    metadata.get("op").and_then(|v| v.as_str()).unwrap_or("call")
                };

                if op == "create" || op == "deploy" {
                    // Fee validation for create/deploy
                    if !self.validate_fee(tx) {
                        warn!(
                            "[Processor] Rejected create/deploy: insufficient fee (required: {} sats)",
                            self.min_execution_fee
                        );
                        return Ok(None);
                    }

                    // If the envelope carries WASM bytecode directly, persist it
                    // content-addressed so later calls can fetch it locally.
                    // sha256(payload) is the canonical code_hash — a metadata
                    // code_hash that disagrees is logged but the computed hash
                    // wins (content-addressing).
                    if envelope.content_type == "application/wasm" && !envelope.payload.is_empty() {
                        use sha2::{Digest, Sha256};
                        let computed = hex::encode(Sha256::digest(&envelope.payload));
                        match self.store.save_wasm(&computed, &envelope.payload) {
                            Ok(()) => info!("[Processor] Stored WASM blob ({} bytes) as {}", envelope.payload.len(), computed),
                            Err(e) => warn!("[Processor] WASM store rejected: {}", e),
                        }
                    }

                    let obj_id = format!("obj_{}", &tx.txid[..16]);
                    let code_hash = metadata
                        .get("code_hash")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            if envelope.content_type == "application/wasm" {
                                use sha2::{Digest, Sha256};
                                hex::encode(Sha256::digest(&envelope.payload))
                            } else {
                                "wasm_default".to_string()
                            }
                        });

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
                    // Use txid hash prefix to avoid collision between txs in same block
                    let undo_id = ((block_height << 32) | (vout_idx as u64))
                        ^ (u64::from_str_radix(&tx.txid.get(..8).unwrap_or("0"), 16).unwrap_or(0));
                    let undo = UndoLogRecord {
                        id: undo_id,
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
                        prev_created_at_block: None,
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
                        let undo_id = ((block_height << 32) | (vout_idx as u64))
                            ^ (u64::from_str_radix(&tx.txid.get(..8).unwrap_or("0"), 16).unwrap_or(0));
                        let undo = UndoLogRecord {
                            id: undo_id,
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
                            prev_created_at_block: Some(obj.created_at_block),
                        };
                        self.store.save_undo_log(&undo)?;

                        // Execute WASM if code_hash is available (not default placeholder)
                        let (new_state, gas_consumed) = if obj.code_hash != "wasm_default" {
                            // Fetch WASM bytecode before executing
                            let wasm_bytes = match self.get_wasm_bytes(&obj.code_hash) {
                                Ok(bytes) => bytes,
                                Err(e) => {
                                    // WASM_MISSING is propagated to the caller so the
                                    // async scanner loop can fetch the blob from the
                                    // P2P DHT, persist it, and retry this transaction.
                                    if missing_wasm_hash(&e).is_some() {
                                        return Err(e);
                                    }
                                    warn!(
                                        "[Processor] Failed to fetch WASM for {}: {} — reverting, no state change",
                                        obj.object_id, e
                                    );
                                    return Ok(None);
                                }
                            };
                            match self.execute_wasm(&obj, &method, &args, &sender, out.value, &wasm_bytes) {
                                Ok(result) => (result.state_data, result.gas_consumed),
                                Err(e) => {
                                    // WASM execution failed (including non-JSON state parse failure)
                                    // — must NOT mutate state. Revert: skip this transaction entirely.
                                    warn!(
                                        "[Processor] WASM execution failed for {}: {} — reverting, no state change",
                                        obj.object_id, e
                                    );
                                    return Ok(None);
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
    pub fn execute_wasm(
        &self,
        obj: &SmartObjectRecord,
        method: &str,
        args: &serde_json::Value,
        caller: &str,
        satoshis: u64,
        wasm_bytes: &[u8],
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

        // Execute via runtime
        let exec_result = self.runtime.execute(
            &wasm_bytes,
            &state,
            caller.to_string(),
            method,
            args.to_string().as_bytes(),
        ).map_err(|e| anyhow!("VM execution failed: {}", e))?;

        // Parse new state from execution result.
        // Issue 10: A parse failure must be a hard error, not a silent fallback
        // to old state. Without this, the object's seal/owner/satoshis move
        // forward while contract state quietly stays frozen — a partially-
        // applied transition indistinguishable from a contract that legitimately
        // returned unchanged state.
        let new_state: serde_json::Value = serde_json::from_slice(&exec_result.updated_state_data)
            .map_err(|e| anyhow!(
                "VM returned non-JSON state data ({} bytes): parse failed: {} — reverting transaction",
                exec_result.updated_state_data.len(), e
            ))?;

        Ok(WasmExecutionResult {
            state_data: new_state,
            gas_consumed: exec_result.gas_consumed,
        })
    }

    /// Retrieve WASM bytecode by code_hash.
    /// Resolution order:
    ///   1. Local WASM table (populated by deploy txs carrying application/wasm
    ///      payloads, or by P2P DHT fetches the scanner loop performs).
    ///   2. If absent locally, return a `WASM_MISSING:<code_hash>` error so the
    ///      async caller can fetch it from the Kademlia DHT, save it via
    ///      `store.save_wasm`, and retry the transaction.
    fn get_wasm_bytes(&self, code_hash: &str) -> Result<Vec<u8>> {
        if code_hash == "wasm_default" || code_hash.is_empty() {
            // Placeholder — no real WASM to execute
            Ok(Vec::new())
        } else {
            match self.store.get_wasm(code_hash) {
                Ok(Some(bytes)) => Ok(bytes),
                Ok(None) => Err(anyhow!("WASM_MISSING:{}", code_hash)),
                Err(e) => Err(anyhow!("WASM store read error for {}: {}", code_hash, e)),
            }
        }
    }
}

/// Extract the code_hash from a `WASM_MISSING:<hash>` processor error,
/// if that is what the error represents.
pub fn missing_wasm_hash(err: &anyhow::Error) -> Option<String> {
    err.to_string()
        .strip_prefix("WASM_MISSING:")
        .map(|s| s.trim().to_string())
}

#[derive(Debug)]
pub struct WasmExecutionResult {
    pub state_data: serde_json::Value,
    pub gas_consumed: u64,
}
