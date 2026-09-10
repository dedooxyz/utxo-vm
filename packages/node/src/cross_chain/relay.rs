use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::consensus::ConsensusManager;
use crate::storage::StateStore;

/// Relay message containing state root from a chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRelayMessage {
    pub source_chain: String,
    pub block_height: u64,
    pub block_hash: String,
    pub state_root: String,
    pub timestamp: i64,
    pub attestations: Vec<crate::types::StateAttestation>,
}

/// Relay status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RelayStatus {
    Pending,
    Relayed,
    Verified,
    Failed,
}

/// Relay record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayRecord {
    pub id: String,
    pub message: StateRelayMessage,
    pub dest_chain: String,
    pub status: RelayStatus,
    pub relayed_at: Option<i64>,
    pub verified_at: Option<i64>,
}

/// State root relay between chains
pub struct StateRelay {
    store: StateStore,
    consensus: Arc<ConsensusManager>,
    records: Arc<RwLock<HashMap<String, RelayRecord>>>,
}

impl StateRelay {
    pub fn new(store: StateStore, consensus: Arc<ConsensusManager>) -> Self {
        Self {
            store,
            consensus,
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a relay message from source chain
    pub fn create_relay_message(
        &self,
        source_chain: &str,
        block_height: u64,
    ) -> Result<StateRelayMessage> {
        // Get block from store
        let block = self
            .store
            .get_block(block_height)
            .context("Failed to get block")?
            .ok_or_else(|| anyhow::anyhow!("Block not found at height {}", block_height))?;

        // Get attestations for this block
        let attestations = self
            .consensus
            .get_attestations(source_chain, block_height);

        Ok(StateRelayMessage {
            source_chain: source_chain.to_string(),
            block_height,
            block_hash: block.block_hash,
            state_root: block.state_root,
            timestamp: block.timestamp,
            attestations,
        })
    }

    /// Relay state root to destination chain
    pub fn relay_to_chain(
        &self,
        message: &StateRelayMessage,
        dest_chain: &str,
    ) -> Result<RelayRecord> {
        let record_id = format!(
            "{}_{}_{}_{}",
            message.source_chain,
            dest_chain,
            message.block_height,
            chrono::Utc::now().timestamp()
        );

        let record = RelayRecord {
            id: record_id.clone(),
            message: message.clone(),
            dest_chain: dest_chain.to_string(),
            status: RelayStatus::Relayed,
            relayed_at: Some(chrono::Utc::now().timestamp()),
            verified_at: None,
        };

        self.records
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
            .insert(record_id, record.clone());

        Ok(record)
    }

    /// Verify a relayed state root
    pub fn verify_relay(&self, record_id: &str) -> Result<RelayRecord> {
        let mut record = {
            let records = self
                .records
                .read()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            records
                .get(record_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Relay record not found: {}", record_id))?
        };

        // Verify quorum — fail-closed on 0 validators
        let attestations = &record.message.attestations;
        let total_validators = self.consensus.get_validator_count();

        // F1.3: Fail-closed — zero validators means no trust anchor
        if total_validators == 0 {
            tracing::warn!("[Relay] Verification failed-closed: no validators registered");
            record.status = RelayStatus::Failed;
            self.records
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
                .insert(record_id.to_string(), record.clone());
            return Ok(record);
        }

        // Dedup by validator_pubkey and verify signatures
        let mut seen_pubkeys = std::collections::HashSet::new();
        let mut root_counts: HashMap<&str, usize> = HashMap::new();

        for att in attestations {
            // Verify ECDSA signature
            if !self.consensus.verify_attestation(att) {
                tracing::warn!("[Relay] Invalid signature from validator {}", att.validator_pubkey);
                continue;
            }
            // Dedup
            if !seen_pubkeys.insert(att.validator_pubkey.clone()) {
                tracing::warn!("[Relay] Duplicate attestation from {}", att.validator_pubkey);
                continue;
            }
            *root_counts.entry(att.state_root.as_str()).or_insert(0) += 1;
        }

        if root_counts.is_empty() {
            record.status = RelayStatus::Failed;
            self.records
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
                .insert(record_id.to_string(), record.clone());
            return Ok(record);
        }

        // Find winning root
        let (winning_root, max_count) = root_counts
            .into_iter()
            .max_by(|(root_a, count_a), (root_b, count_b)| {
                count_a.cmp(count_b).then_with(|| root_a.cmp(root_b))
            })
            .unwrap();

        let required = (total_validators * 2 + 2) / 3;

        // Winning root must match the message's state_root
        if winning_root == record.message.state_root && max_count >= required {
            record.status = RelayStatus::Verified;
            record.verified_at = Some(chrono::Utc::now().timestamp());
        } else {
            record.status = RelayStatus::Failed;
        }

        // Update record
        self.records
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
            .insert(record_id.to_string(), record.clone());

        Ok(record)
    }

    /// Get relay record
    pub fn get_record(&self, record_id: &str) -> Result<Option<RelayRecord>> {
        let records = self
            .records
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(records.get(record_id).cloned())
    }

    /// Get all relay records for a source chain
    pub fn get_records_for_chain(&self, source_chain: &str) -> Result<Vec<RelayRecord>> {
        let records = self
            .records
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(records
            .values()
            .filter(|r| r.message.source_chain == source_chain)
            .cloned()
            .collect())
    }

    /// Get latest relayed state root for a chain
    pub fn get_latest_relay(&self, source_chain: &str) -> Result<Option<RelayRecord>> {
        let records = self
            .records
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        Ok(records
            .values()
            .filter(|r| r.message.source_chain == source_chain && r.status == RelayStatus::Verified)
            .max_by_key(|r| r.message.block_height)
            .cloned())
    }

    /// Get all relay records
    pub fn get_all_records(&self) -> Result<Vec<RelayRecord>> {
        let records = self
            .records
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        Ok(records.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_relay_flow() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("relay_test.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let relay = StateRelay::new(store, consensus);

        // Create relay message
        let message = StateRelayMessage {
            source_chain: "JKC".to_string(),
            block_height: 100,
            block_hash: "hash_100".to_string(),
            state_root: "root_100".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            attestations: vec![],
        };

        // Relay to DOGE
        let record = relay.relay_to_chain(&message, "DOGE").unwrap();
        assert_eq!(record.status, RelayStatus::Relayed);
        assert_eq!(record.dest_chain, "DOGE");
    }
}
