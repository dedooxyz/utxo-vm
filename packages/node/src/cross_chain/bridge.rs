use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::verifier::{CrossChainProof, CrossChainVerifier};
use crate::storage::StateStore;
use crate::types::SmartObjectRecord;

/// Bridge transfer request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeTransfer {
    pub id: String,
    pub source_chain: String,
    pub source_object_id: String,
    pub dest_chain: String,
    pub dest_owner: String,
    pub amount: u64,
    pub status: BridgeStatus,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

/// Bridge transfer status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BridgeStatus {
    Pending,
    Verified,
    Locked,
    Minted,
    Completed,
    Failed,
}

/// Bridge event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub transfer_id: String,
    pub event_type: String,
    pub chain: String,
    pub height: u64,
    pub timestamp: i64,
    pub data: serde_json::Value,
}

/// Lock proof (source chain locked assets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockProof {
    pub txid: String,
    pub vout: u32,
    pub amount: u64,
    pub owner: String,
    pub chain: String,
    pub height: u64,
}

/// Mint proof (destination chain minted assets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintProof {
    pub txid: String,
    pub vout: u32,
    pub amount: u64,
    pub owner: String,
    pub chain: String,
    pub height: u64,
}

/// Bridge manager for cross-chain asset transfers
pub struct BridgeManager {
    verifier: Arc<CrossChainVerifier>,
    store: StateStore,
    transfers: Arc<RwLock<HashMap<String, BridgeTransfer>>>,
    events: Arc<RwLock<Vec<BridgeEvent>>>,
}

impl BridgeManager {
    pub fn new(verifier: Arc<CrossChainVerifier>, store: StateStore) -> Self {
        Self {
            verifier,
            store,
            transfers: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initiate a cross-chain transfer (Lock on source chain)
    pub fn lock_assets(
        &self,
        source_chain: &str,
        object_id: &str,
        dest_chain: &str,
        dest_owner: &str,
    ) -> Result<BridgeTransfer> {
        // Get object from source chain
        let obj = self
            .store
            .get_object(object_id)
            .context("Object not found")?
            .ok_or_else(|| anyhow::anyhow!("Object not found: {}", object_id))?;

        // Create transfer record
        let transfer_id = format!(
            "{}_{}_{}_{}",
            source_chain,
            dest_chain,
            object_id,
            chrono::Utc::now().timestamp()
        );

        let transfer = BridgeTransfer {
            id: transfer_id.clone(),
            source_chain: source_chain.to_string(),
            source_object_id: object_id.to_string(),
            dest_chain: dest_chain.to_string(),
            dest_owner: dest_owner.to_string(),
            amount: obj.satoshis,
            status: BridgeStatus::Pending,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
        };

        // Store transfer
        self.transfers
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
            .insert(transfer_id.clone(), transfer.clone());

        // Emit event
        self.emit_event(BridgeEvent {
            transfer_id: transfer_id.clone(),
            event_type: "LOCK_INITIATED".to_string(),
            chain: source_chain.to_string(),
            height: 0,
            timestamp: chrono::Utc::now().timestamp(),
            data: serde_json::json!({
                "object_id": object_id,
                "dest_chain": dest_chain,
                "dest_owner": dest_owner,
                "amount": obj.satoshis,
            }),
        });

        Ok(transfer)
    }

    /// Verify and complete cross-chain transfer (Mint on destination chain)
    pub fn mint_from_proof(
        &self,
        transfer_id: &str,
        proof: &CrossChainProof,
    ) -> Result<BridgeTransfer> {
        // Get transfer record
        let mut transfer = {
            let transfers = self
                .transfers
                .read()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            transfers
                .get(transfer_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Transfer not found: {}", transfer_id))?
        };

        // Verify the cross-chain proof
        let verification = self.verifier.verify_proof(proof);

        if !verification.valid {
            transfer.status = BridgeStatus::Failed;
            self.update_transfer(&transfer)?;

            return Err(anyhow::anyhow!(
                "Proof verification failed: {:?}",
                verification.error
            ));
        }

        // Update status to verified
        transfer.status = BridgeStatus::Verified;
        self.update_transfer(&transfer)?;

        // Emit verified event
        self.emit_event(BridgeEvent {
            transfer_id: transfer_id.to_string(),
            event_type: "PROOF_VERIFIED".to_string(),
            chain: transfer.dest_chain.clone(),
            height: proof.source_height,
            timestamp: chrono::Utc::now().timestamp(),
            data: serde_json::json!({
                "source_chain": proof.source_chain,
                "source_height": proof.source_height,
                "quorum_reached": verification.quorum_reached,
                "merkle_verified": verification.merkle_verified,
            }),
        });

        // Create minted object on destination chain
        let minted_obj = SmartObjectRecord {
            object_id: format!("bridge_{}_{}", transfer_id, chrono::Utc::now().timestamp()),
            code_hash: proof.object_data
                .get("code_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("bridge_native")
                .to_string(),
            seal: format!("bridge_{}:0", transfer_id),
            satoshis: transfer.amount,
            owner: transfer.dest_owner.clone(),
            state_data: proof.object_data.clone(),
            created_at_block: 0,
            updated_at_block: 0,
        };

        self.store.save_object(&minted_obj)?;

        // Update status to minted
        transfer.status = BridgeStatus::Minted;
        transfer.completed_at = Some(chrono::Utc::now().timestamp());
        self.update_transfer(&transfer)?;

        // Emit minted event
        self.emit_event(BridgeEvent {
            transfer_id: transfer_id.to_string(),
            event_type: "ASSETS_MINTED".to_string(),
            chain: transfer.dest_chain.clone(),
            height: 0,
            timestamp: chrono::Utc::now().timestamp(),
            data: serde_json::json!({
                "new_object_id": minted_obj.object_id,
                "owner": transfer.dest_owner,
                "amount": transfer.amount,
            }),
        });

        Ok(transfer)
    }

    /// Get transfer status
    pub fn get_transfer(&self, transfer_id: &str) -> Result<Option<BridgeTransfer>> {
        let transfers = self
            .transfers
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(transfers.get(transfer_id).cloned())
    }

    /// Get all transfers
    pub fn get_all_transfers(&self) -> Result<Vec<BridgeTransfer>> {
        let transfers = self
            .transfers
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(transfers.values().cloned().collect())
    }

    /// Get events for a transfer
    pub fn get_events(&self, transfer_id: &str) -> Result<Vec<BridgeEvent>> {
        let events = self
            .events
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(events
            .iter()
            .filter(|e| e.transfer_id == transfer_id)
            .cloned()
            .collect())
    }

    /// Update transfer record
    fn update_transfer(&self, transfer: &BridgeTransfer) -> Result<()> {
        let mut transfers = self
            .transfers
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        transfers.insert(transfer.id.clone(), transfer.clone());
        Ok(())
    }

    /// Emit bridge event
    fn emit_event(&self, event: BridgeEvent) {
        if let Ok(mut events) = self.events.write() {
            events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::ConsensusManager;
    use crate::types::SmartObjectRecord;
    use tempfile::tempdir;

    #[test]
    fn test_bridge_transfer_flow() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_test.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let verifier = Arc::new(CrossChainVerifier::new(consensus));
        let bridge = BridgeManager::new(verifier, store.clone());

        // Insert test object
        let obj = SmartObjectRecord {
            object_id: "obj_test".to_string(),
            code_hash: "wasm_test".to_string(),
            seal: "tx_test:0".to_string(),
            satoshis: 100_000,
            owner: "owner_jkc".to_string(),
            state_data: serde_json::json!({"ticker": "SON"}),
            created_at_block: 100,
            updated_at_block: 100,
        };
        store.save_object(&obj).unwrap();

        // Lock assets
        let transfer = bridge
            .lock_assets("JKC", "obj_test", "DOGE", "doge_owner_addr")
            .unwrap();

        assert_eq!(transfer.status, BridgeStatus::Pending);
        assert_eq!(transfer.source_chain, "JKC");
        assert_eq!(transfer.dest_chain, "DOGE");

        // Get transfer
        let got = bridge.get_transfer(&transfer.id).unwrap().unwrap();
        assert_eq!(got.id, transfer.id);
    }
}
