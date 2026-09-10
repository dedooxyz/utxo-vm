use anyhow::{Context, Result};
use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tracing;

use super::verifier::{CrossChainProof, CrossChainVerifier};
use crate::storage::StateStore;
use crate::types::SmartObjectRecord;

/// Reclaim timeout in seconds. After this duration since lock creation,
/// the original owner can reclaim a locked object without a successful mint.
/// Matches the order of magnitude of unbond_delay (60 blocks ≈ 60 min on JKC testnet).
const RECLAIM_TIMEOUT_SECS: i64 = 3600; // 1 hour

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
    minted_objects: Arc<RwLock<HashMap<String, String>>>,
    /// Per-transfer_id mutex to serialize concurrent mint_from_proof retries
    transfer_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
}

impl BridgeManager {
    pub fn new(verifier: Arc<CrossChainVerifier>, store: StateStore) -> Self {
        // Rehydrate in-memory caches from StateStore
        let mut transfers = HashMap::new();
        let mut minted_objects = HashMap::new();

        if let Ok(persisted_transfers) = store.get_all_bridge_transfers() {
            for t in persisted_transfers {
                transfers.insert(t.id.clone(), t);
            }
            tracing::info!("[Bridge] Rehydrated {} transfers from StateStore", transfers.len());
        }

        if let Ok(persisted_proofs) = store.get_all_minted_proofs() {
            for (obj_id, transfer_id) in persisted_proofs {
                minted_objects.insert(obj_id, transfer_id);
            }
            tracing::info!("[Bridge] Rehydrated {} minted proofs from StateStore", minted_objects.len());
        }

        Self {
            verifier,
            store,
            transfers: Arc::new(RwLock::new(transfers)),
            events: Arc::new(RwLock::new(Vec::new())),
            minted_objects: Arc::new(RwLock::new(minted_objects)),
            transfer_locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Initiate a cross-chain transfer (Lock on source chain).
    /// F1.1: Marks source object as locked so it cannot be re-used.
    /// P0-fu.3: Verifies caller owns the object via secp256k1 signature.
    pub fn lock_assets(
        &self,
        source_chain: &str,
        object_id: &str,
        dest_chain: &str,
        dest_owner: &str,
        caller_pubkey: &str,
        signature_hex: &str,
    ) -> Result<BridgeTransfer> {
        // Get object from source chain
        let obj = self
            .store
            .get_object(object_id)
            .context("Object not found")?
            .ok_or_else(|| anyhow::anyhow!("Object not found: {}", object_id))?;

        // F1.1: Check that object is not already locked
        if let Some(locked_owner) = obj.owner.strip_prefix("bridge_locked:") {
            return Err(anyhow::anyhow!(
                "Object {} is already locked for bridge transfer (locked_by: {}). Cannot re-lock.",
                object_id,
                locked_owner,
            ));
        }

        // P0-fu.3: Verify caller owns the object
        // 1. caller_pubkey must match obj.owner
        if caller_pubkey != obj.owner {
            return Err(anyhow::anyhow!(
                "Caller pubkey '{}' does not match object owner '{}'. Only the owner can lock.",
                caller_pubkey,
                obj.owner,
            ));
        }

        // 2. Verify secp256k1 signature over canonical message
        let msg = format!("lock:{}:{}:{}:{}", source_chain, object_id, dest_chain, dest_owner);
        let mut hasher = Sha256::new();
        hasher.update(msg.as_bytes());
        let digest = hasher.finalize();
        let mut msg_bytes = [0u8; 32];
        msg_bytes.copy_from_slice(&digest);
        let message = Message::from_digest(msg_bytes);

        let pubkey_bytes = hex::decode(caller_pubkey)
            .map_err(|e| anyhow::anyhow!("Invalid hex in caller_pubkey: {}", e))?;
        let pubkey = PublicKey::from_slice(&pubkey_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid secp256k1 pubkey: {}", e))?;

        let sig_bytes = hex::decode(signature_hex)
            .map_err(|e| anyhow::anyhow!("Invalid hex in signature: {}", e))?;
        let sig = Signature::from_compact(&sig_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid secp256k1 signature: {}", e))?;

        let secp = Secp256k1::new();
        secp.verify_ecdsa(&message, &sig, &pubkey)
            .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))?;

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
            status: BridgeStatus::Locked,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
        };

        // F1.1: Lock the source object by rewriting owner prefix
        let mut locked_obj = obj.clone();
        locked_obj.owner = format!("bridge_locked:{}", obj.owner);
        self.store.save_object(&locked_obj)?;

        // Persist transfer to store
        self.store.save_bridge_transfer(&transfer)?;

        // Also keep in-memory cache for fast reads
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

    /// Verify and complete cross-chain transfer (Mint on destination chain).
    /// F1.2: Checks replay — same source object cannot be minted twice.
    /// P0-follow-up.1: Replay guard is persisted to redb (survives restart).
    pub fn mint_from_proof(
        &self,
        transfer_id: &str,
        proof: &CrossChainProof,
    ) -> Result<BridgeTransfer> {
        // Serialize concurrent calls for the same transfer_id to prevent
        // duplicate mints from concurrent retries.
        let lock = {
            let mut locks = self
                .transfer_locks
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            locks
                .entry(transfer_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock
            .lock()
            .map_err(|e| anyhow::anyhow!("Transfer lock poisoned: {}", e))?;

        // F1.2 + P0-fu.1: Atomic check-and-insert in redb.
        // redb serializes write transactions per-table, so two concurrent
        // calls for the same object_id cannot both succeed.
        let newly_inserted = self
            .store
            .check_and_insert_minted_proof(&proof.object_id, transfer_id)
            .context("Failed to check minted proofs store")?;

        if !newly_inserted {
            // Either same transfer (idempotent) or a different transfer (replay).
            // Read back to see which case.
            if let Some(existing_transfer) = self.store.get_minted_proof(&proof.object_id)? {
                if existing_transfer != transfer_id {
                    return Err(anyhow::anyhow!(
                        "Replay rejected: source object '{}' was already minted via transfer '{}'. \
                         Cannot mint again in transfer '{}'.",
                        proof.object_id,
                        existing_transfer,
                        transfer_id,
                    ));
                }
                // Same transfer — check status
                let transfers = self
                    .transfers
                    .read()
                    .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
                if let Some(t) = transfers.get(transfer_id) {
                    if t.status == BridgeStatus::Completed || t.status == BridgeStatus::Minted {
                        return Err(anyhow::anyhow!(
                            "Transfer '{}' is already in status {:?}. Cannot re-mint.",
                            transfer_id,
                            t.status,
                        ));
                    }
                }
            }
        }

        // Update in-memory cache
        self.minted_objects
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
            .insert(proof.object_id.clone(), transfer_id.to_string());

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

        // F1.2: Reject if already completed or minted
        if transfer.status == BridgeStatus::Completed || transfer.status == BridgeStatus::Minted {
            return Err(anyhow::anyhow!(
                "Transfer '{}' is already in status {:?}. Cannot re-mint.",
                transfer_id,
                transfer.status,
            ));
        }

        // Verify the cross-chain proof
        let verification = self.verifier.verify_proof(proof);

        if !verification.valid {
            transfer.status = BridgeStatus::Failed;
            self.update_transfer(&transfer)?;

            // P0-follow-up.2: Unlock the source object on failed proof
            if let Err(unlock_err) = self.unlock_object(&transfer) {
                tracing::warn!(
                    "[Bridge] Failed to unlock source object after proof failure: {}",
                    unlock_err
                );
            }

            return Err(anyhow::anyhow!(
                "Proof verification failed: {:?}",
                verification.error
            ));
        }

        // Bind proof to transfer: proof must reference the same source object and chain
        if proof.object_id != transfer.source_object_id {
            return Err(anyhow::anyhow!(
                "Proof object_id '{}' does not match transfer source_object_id '{}'. \
                 Cannot mint a transfer with a proof for a different object.",
                proof.object_id,
                transfer.source_object_id,
            ));
        }
        if proof.source_chain != transfer.source_chain {
            return Err(anyhow::anyhow!(
                "Proof source_chain '{}' does not match transfer source_chain '{}'.",
                proof.source_chain,
                transfer.source_chain,
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
            code_hash: proof
                .object_data
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
        // Try in-memory first
        {
            let transfers = self
                .transfers
                .read()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            if let Some(t) = transfers.get(transfer_id) {
                return Ok(Some(t.clone()));
            }
        }
        // Fall back to store
        self.store.get_bridge_transfer(transfer_id)
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

    /// Update transfer record (memory + store)
    fn update_transfer(&self, transfer: &BridgeTransfer) -> Result<()> {
        self.transfers
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?
            .insert(transfer.id.clone(), transfer.clone());
        self.store.save_bridge_transfer(transfer)?;
        Ok(())
    }

    /// Emit bridge event
    fn emit_event(&self, event: BridgeEvent) {
        if let Ok(mut events) = self.events.write() {
            events.push(event);
        }
    }

    /// P0-follow-up.2: Unlock a source object that was locked by lock_assets().
    /// Strips the "bridge_locked:" prefix from the owner field.
    fn unlock_object(&self, transfer: &BridgeTransfer) -> Result<()> {
        let obj = self
            .store
            .get_object(&transfer.source_object_id)
            .context("Failed to load source object for unlock")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Source object {} not found during unlock",
                    transfer.source_object_id
                )
            })?;

        if let Some(original_owner) = obj.owner.strip_prefix("bridge_locked:") {
            let mut unlocked_obj = obj.clone();
            unlocked_obj.owner = original_owner.to_string();
            self.store.save_object(&unlocked_obj)?;
        }
        // If the object isn't locked (shouldn't happen), silently succeed.

        self.emit_event(BridgeEvent {
            transfer_id: transfer.id.clone(),
            event_type: "OBJECT_UNLOCKED".to_string(),
            chain: transfer.source_chain.clone(),
            height: 0,
            timestamp: chrono::Utc::now().timestamp(),
            data: serde_json::json!({
                "object_id": transfer.source_object_id,
                "reason": "transfer_failed_or_cancelled",
            }),
        });

        Ok(())
    }

    /// P0-follow-up.2: Cancel a pending/locked transfer and reclaim the source object.
    /// Only allowed after RECLAIM_TIMEOUT_SECS have elapsed since lock creation,
    /// and only by the original (unprefixed) owner.
    pub fn cancel_transfer(
        &self,
        transfer_id: &str,
        caller: &str,
        signature_hex: &str,
    ) -> Result<BridgeTransfer> {
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

        // Only Pending or Locked transfers can be cancelled
        if transfer.status != BridgeStatus::Pending && transfer.status != BridgeStatus::Locked {
            return Err(anyhow::anyhow!(
                "Transfer '{}' is in status {:?} — only Pending or Locked transfers can be cancelled.",
                transfer_id,
                transfer.status,
            ));
        }

        // Timeout check
        let now = chrono::Utc::now().timestamp();
        let elapsed = now - transfer.created_at;
        if elapsed < RECLAIM_TIMEOUT_SECS {
            return Err(anyhow::anyhow!(
                "Reclaim timeout not reached: {}s elapsed, {}s required. Try again later.",
                elapsed,
                RECLAIM_TIMEOUT_SECS,
            ));
        }

        // Owner check: caller must match the original (unprefixed) owner
        let source_obj = self
            .store
            .get_object(&transfer.source_object_id)?
            .ok_or_else(|| anyhow::anyhow!("Source object not found"))?;

        let original_owner = source_obj
            .owner
            .strip_prefix("bridge_locked:")
            .unwrap_or(&source_obj.owner);

        if caller != original_owner {
            return Err(anyhow::anyhow!(
                "Caller '{}' is not the original owner '{}' of the locked object.",
                caller,
                original_owner,
            ));
        }

        // Verify ECDSA signature: caller must sign "cancel:{transfer_id}" with their key
        let secp = Secp256k1::new();
        let pubkey_bytes = hex::decode(original_owner)
            .map_err(|e| anyhow::anyhow!("Invalid owner pubkey hex: {}", e))?;
        let pubkey = PublicKey::from_slice(&pubkey_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;
        let sig_bytes = hex::decode(signature_hex)
            .map_err(|e| anyhow::anyhow!("Invalid signature hex: {}", e))?;
        let sig = Signature::from_compact(&sig_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid signature format: {}", e))?;

        let mut hasher = Sha256::new();
        hasher.update(b"cancel:");
        hasher.update(transfer_id.as_bytes());
        let digest = hasher.finalize();
        let mut msg_arr = [0u8; 32];
        msg_arr.copy_from_slice(&digest);
        let msg = Message::from_digest(msg_arr);

        secp.verify_ecdsa(&msg, &sig, &pubkey)
            .map_err(|_| anyhow::anyhow!("ECDSA signature verification failed for cancel_transfer"))?;

        // Unlock the source object
        self.unlock_object(&transfer)?;

        // Mark transfer as failed (cancelled)
        transfer.status = BridgeStatus::Failed;
        transfer.completed_at = Some(now);
        self.update_transfer(&transfer)?;

        self.emit_event(BridgeEvent {
            transfer_id: transfer_id.to_string(),
            event_type: "TRANSFER_CANCELLED".to_string(),
            chain: transfer.source_chain.clone(),
            height: 0,
            timestamp: now,
            data: serde_json::json!({
                "source_object_id": transfer.source_object_id,
                "caller": caller,
                "reason": "timeout_reclaim",
            }),
        });

        Ok(transfer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::ConsensusManager;
    use crate::types::SmartObjectRecord;
    use secp256k1::rand::rngs::OsRng;
    use tempfile::tempdir;

    /// Helper: sign a lock message with secp256k1
    fn sign_lock_msg(
        secp: &Secp256k1<secp256k1::All>,
        sk: &secp256k1::SecretKey,
        source_chain: &str,
        object_id: &str,
        dest_chain: &str,
        dest_owner: &str,
    ) -> (String, String) {
        let pk = secp256k1::PublicKey::from_secret_key(secp, sk);
        let pk_hex = hex::encode(pk.serialize());
        let msg = format!("lock:{}:{}:{}:{}", source_chain, object_id, dest_chain, dest_owner);
        let mut hasher = Sha256::new();
        hasher.update(msg.as_bytes());
        let digest = hasher.finalize();
        let mut msg_bytes = [0u8; 32];
        msg_bytes.copy_from_slice(&digest);
        let message = Message::from_digest(msg_bytes);
        let sig = secp.sign_ecdsa(&message, sk);
        (pk_hex, hex::encode(sig.serialize_compact()))
    }

    fn sign_cancel_msg(
        secp: &Secp256k1<secp256k1::All>,
        sk: &secp256k1::SecretKey,
        transfer_id: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"cancel:");
        hasher.update(transfer_id.as_bytes());
        let digest = hasher.finalize();
        let mut msg_bytes = [0u8; 32];
        msg_bytes.copy_from_slice(&digest);
        let message = Message::from_digest(msg_bytes);
        let sig = secp.sign_ecdsa(&message, sk);
        hex::encode(sig.serialize_compact())
    }

    #[test]
    fn test_bridge_transfer_flow() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_test.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let verifier = Arc::new(CrossChainVerifier::new(consensus));
        let bridge = BridgeManager::new(verifier, store.clone());

        let secp = Secp256k1::new();
        let (sk, _pk) = secp.generate_keypair(&mut OsRng);

        // Insert test object — owner is the hex pubkey of someone else
        let (_, real_owner_pk) = secp.generate_keypair(&mut OsRng);
        let real_owner_hex = hex::encode(real_owner_pk.serialize());

        let obj = SmartObjectRecord {
            object_id: "obj_test".to_string(),
            code_hash: "wasm_test".to_string(),
            seal: "tx_test:0".to_string(),
            satoshis: 100_000,
            owner: real_owner_hex.clone(),
            state_data: serde_json::json!({"ticker": "SON"}),
            created_at_block: 100,
            updated_at_block: 100,
        };
        store.save_object(&obj).unwrap();

        // Lock with correct owner's signature
        let (pk_hex, sig_hex) = sign_lock_msg(&secp, &sk, "JKC", "obj_test", "DOGE", "doge_owner_addr");
        // Use sk's pubkey as owner
        let owner_hex = hex::encode(secp256k1::PublicKey::from_secret_key(&secp, &sk).serialize());
        let mut obj_with_sk_owner = obj.clone();
        obj_with_sk_owner.owner = owner_hex.clone();
        store.save_object(&obj_with_sk_owner).unwrap();

        let transfer = bridge
            .lock_assets("JKC", "obj_test", "DOGE", "doge_owner_addr", &pk_hex, &sig_hex)
            .unwrap();

        assert_eq!(transfer.status, BridgeStatus::Locked);
        assert_eq!(transfer.source_chain, "JKC");
        assert_eq!(transfer.dest_chain, "DOGE");

        // F1.1: Source object must now be locked
        let locked_obj = store.get_object("obj_test").unwrap().unwrap();
        assert!(
            locked_obj.owner.starts_with("bridge_locked:"),
            "Source object owner should start with 'bridge_locked:', got: {}",
            locked_obj.owner
        );

        // F1.1: Re-locking must fail
        let (pk2, sig2) = sign_lock_msg(&secp, &sk, "LTC", "obj_test", "LTC", "ltc_owner");
        let relock_result = bridge.lock_assets("JKC", "obj_test", "LTC", "ltc_owner", &pk2, &sig2);
        assert!(relock_result.is_err(), "Re-locking a locked object must fail");
        let err_msg = relock_result.unwrap_err().to_string();
        assert!(
            err_msg.contains("already locked"),
            "Error should say 'already locked', got: {}",
            err_msg
        );

        // Get transfer
        let got = bridge.get_transfer(&transfer.id).unwrap().unwrap();
        assert_eq!(got.id, transfer.id);
    }

    /// P0-fu.3: Lock must be rejected when signature doesn't match object owner
    #[test]
    fn test_lock_rejects_wrong_owner_signature() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_auth.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let verifier = Arc::new(CrossChainVerifier::new(consensus));
        let bridge = BridgeManager::new(verifier, store.clone());

        let secp = Secp256k1::new();
        let (_sk_alice, pk_alice) = secp.generate_keypair(&mut OsRng);
        let (sk_bob, pk_bob) = secp.generate_keypair(&mut OsRng);

        // Object owned by alice
        let alice_hex = hex::encode(pk_alice.serialize());
        let obj = SmartObjectRecord {
            object_id: "obj_auth".to_string(),
            code_hash: "wasm_auth".to_string(),
            seal: "tx_auth:0".to_string(),
            satoshis: 50_000,
            owner: alice_hex,
            state_data: serde_json::json!({}),
            created_at_block: 1,
            updated_at_block: 1,
        };
        store.save_object(&obj).unwrap();

        // Bob tries to lock alice's object — should fail
        let bob_pk_hex = hex::encode(pk_bob.serialize());
        let msg = format!("lock:JKC:obj_auth:DOGE:doge_addr");
        let mut hasher = Sha256::new();
        hasher.update(msg.as_bytes());
        let digest = hasher.finalize();
        let mut msg_bytes = [0u8; 32];
        msg_bytes.copy_from_slice(&digest);
        let message = Message::from_digest(msg_bytes);
        let bob_sig = secp.sign_ecdsa(&message, &sk_bob);
        let bob_sig_hex = hex::encode(bob_sig.serialize_compact());

        let result = bridge.lock_assets(
            "JKC", "obj_auth", "DOGE", "doge_addr", &bob_pk_hex, &bob_sig_hex,
        );
        assert!(result.is_err(), "Lock with wrong owner signature must fail");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not match") || err.contains("Signature verification failed"),
            "Error should indicate owner mismatch or bad sig, got: {}",
            err
        );
    }

    /// P0-follow-up.1: Minted proof must survive node restart.
    /// After minting, a fresh BridgeManager against the same store path must
    /// reject a replay of the same source object via a different transfer.
    #[test]
    fn test_minted_proof_persists_across_restart() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_restart.redb");

        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut OsRng);
        let pk_hex = hex::encode(pk.serialize());

        // --- Phase 1: first "node lifetime" — lock and mint ---
        {
            let store = StateStore::open(&db_path).unwrap();
            let consensus = Arc::new(ConsensusManager::new(3));
            let verifier = Arc::new(CrossChainVerifier::new(consensus));
            let bridge = BridgeManager::new(verifier, store.clone());

            let obj = SmartObjectRecord {
                object_id: "obj_restart".to_string(),
                code_hash: "wasm_restart".to_string(),
                seal: "tx_restart:0".to_string(),
                satoshis: 50_000,
                owner: pk_hex.clone(),
                state_data: serde_json::json!({}),
                created_at_block: 1,
                updated_at_block: 1,
            };
            store.save_object(&obj).unwrap();

            let (caller_pk, sig) = sign_lock_msg(&secp, &sk, "JKC", "obj_restart", "DOGE", "bob");
            let transfer = bridge
                .lock_assets("JKC", "obj_restart", "DOGE", "bob", &caller_pk, &sig)
                .unwrap();

            // Manually insert into minted_proofs to simulate a successful mint.
            store
                .check_and_insert_minted_proof("obj_restart", &transfer.id)
                .unwrap();
        }

        // --- Phase 2: "restart" — fresh BridgeManager, same db_path ---
        {
            let store = StateStore::open(&db_path).unwrap();
            let consensus = Arc::new(ConsensusManager::new(3));
            let verifier = Arc::new(CrossChainVerifier::new(consensus));
            let bridge = BridgeManager::new(verifier, store.clone());

            // Insert a second object to lock
            let obj2 = SmartObjectRecord {
                object_id: "obj_restart_2".to_string(),
                code_hash: "wasm_restart".to_string(),
                seal: "tx_restart2:0".to_string(),
                satoshis: 30_000,
                owner: pk_hex.clone(),
                state_data: serde_json::json!({}),
                created_at_block: 2,
                updated_at_block: 2,
            };
            store.save_object(&obj2).unwrap();

            let (caller_pk2, sig2) = sign_lock_msg(&secp, &sk, "JKC", "obj_restart_2", "DOGE", "dave");
            let transfer2 = bridge
                .lock_assets("JKC", "obj_restart_2", "DOGE", "dave", &caller_pk2, &sig2)
                .unwrap();

            // Try to mint obj_restart again via transfer2 — must be rejected
            let fake_proof = CrossChainProof {
                source_chain: "JKC".to_string(),
                source_height: 100,
                source_root: "abc".to_string(),
                object_id: "obj_restart".to_string(),
                object_data: serde_json::json!({}),
                merkle_proof: crate::types::SmtInclusionProof {
                    key_hex: "aa".to_string(),
                    value_hex: "bb".to_string(),
                    root_hex: "cc".to_string(),
                    proof_path: vec![],
                    verified: false,
                },
                attestations: vec![],
            };

            let result = bridge.mint_from_proof(&transfer2.id, &fake_proof);
            assert!(
                result.is_err(),
                "Replay must be rejected after restart — minted_proofs persisted"
            );
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("already minted"),
                "Error should mention 'already minted', got: {}",
                err
            );
        }
    }

    /// P0-follow-up.2: Failed proof must unlock the source object.
    #[test]
    fn test_failed_proof_unlocks_source_object() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_unlock.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let verifier = Arc::new(CrossChainVerifier::new(consensus));
        let bridge = BridgeManager::new(verifier, store.clone());

        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut OsRng);
        let pk_hex = hex::encode(pk.serialize());

        let obj = SmartObjectRecord {
            object_id: "obj_unlock".to_string(),
            code_hash: "wasm_unlock".to_string(),
            seal: "tx_unlock:0".to_string(),
            satoshis: 80_000,
            owner: pk_hex.clone(),
            state_data: serde_json::json!({}),
            created_at_block: 10,
            updated_at_block: 10,
        };
        store.save_object(&obj).unwrap();

        let (caller_pk, sig) = sign_lock_msg(&secp, &sk, "JKC", "obj_unlock", "LTC", "grace");
        let transfer = bridge
            .lock_assets("JKC", "obj_unlock", "LTC", "grace", &caller_pk, &sig)
            .unwrap();

        // Verify object is locked
        let locked = store.get_object("obj_unlock").unwrap().unwrap();
        assert!(locked.owner.starts_with("bridge_locked:"));

        // Submit a proof that will FAIL verification (empty attestations)
        let bad_proof = CrossChainProof {
            source_chain: "JKC".to_string(),
            source_height: 50,
            source_root: "deadbeef".to_string(),
            object_id: "obj_unlock".to_string(),
            object_data: serde_json::json!({}),
            merkle_proof: crate::types::SmtInclusionProof {
                key_hex: "aa".to_string(),
                value_hex: "bb".to_string(),
                root_hex: "cc".to_string(),
                proof_path: vec![],
                verified: false,
            },
            attestations: vec![], // No attestations → quorum fails
        };

        let result = bridge.mint_from_proof(&transfer.id, &bad_proof);
        assert!(result.is_err(), "Proof verification should fail");

        // Source object must be unlocked
        let unlocked = store.get_object("obj_unlock").unwrap().unwrap();
        assert_eq!(
            unlocked.owner, pk_hex,
            "Source object owner must be restored after failed proof"
        );
    }

    /// P0-follow-up.2: cancel_transfer — reclaim a lock that was never minted.
    #[test]
    fn test_cancel_transfer_reclaim_timeout() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("bridge_cancel.redb");
        let store = StateStore::open(&db_path).unwrap();
        let consensus = Arc::new(ConsensusManager::new(3));
        let verifier = Arc::new(CrossChainVerifier::new(consensus));
        let bridge = BridgeManager::new(verifier, store.clone());

        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut OsRng);
        let pk_hex = hex::encode(pk.serialize());

        let obj = SmartObjectRecord {
            object_id: "obj_cancel".to_string(),
            code_hash: "wasm_cancel".to_string(),
            seal: "tx_cancel:0".to_string(),
            satoshis: 120_000,
            owner: pk_hex.clone(),
            state_data: serde_json::json!({}),
            created_at_block: 1,
            updated_at_block: 1,
        };
        store.save_object(&obj).unwrap();

        let (caller_pk, sig) = sign_lock_msg(&secp, &sk, "JKC", "obj_cancel", "DOGE", "ivan");
        let transfer = bridge
            .lock_assets("JKC", "obj_cancel", "DOGE", "ivan", &caller_pk, &sig)
            .unwrap();

        // Cancel immediately — should fail (timeout not reached)
        let cancel_sig = sign_cancel_msg(&secp, &sk, &transfer.id);
        let result = bridge.cancel_transfer(&transfer.id, &pk_hex, &cancel_sig);
        assert!(
            result.is_err(),
            "Cancel before timeout must be rejected"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timeout") || err.contains("not reached"),
            "Error should mention timeout, got: {}",
            err
        );

        // Simulate old transfer by backdating created_at
        {
            let mut transfers = bridge.transfers.write().unwrap();
            let mut t = transfers.get(&transfer.id).cloned().unwrap();
            t.created_at = chrono::Utc::now().timestamp() - 10_000; // far in the past
            transfers.insert(transfer.id.clone(), t.clone());
            // Also persist the backdated transfer
            store.save_bridge_transfer(&t).unwrap();
        }

        // Cancel after timeout — should succeed
        let cancel_sig = sign_cancel_msg(&secp, &sk, &transfer.id);
        let result = bridge.cancel_transfer(&transfer.id, &pk_hex, &cancel_sig);
        assert!(result.is_ok(), "Cancel after timeout should succeed: {:?}", result.err());

        // Object must be unlocked
        let unlocked = store.get_object("obj_cancel").unwrap().unwrap();
        assert_eq!(unlocked.owner, pk_hex, "Owner must be restored after cancel");
    }
}
