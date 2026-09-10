use std::path::Path;
use std::sync::{Arc, RwLock};
use anyhow::Result;
use redb::{Database, ReadableTable, TableDefinition};
use sha2::{Digest, Sha256};
use tracing;
use crate::storage::smt::SparseMerkleTree;
use crate::types::{BlockRecord, SmartObjectRecord, SmtInclusionProof, StateTransitionRecord, UndoLogRecord};

const OBJECTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("objects");
const SEALS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("seals");
const TRANSITIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("transitions");
const BLOCKS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const CHAIN_META_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("chain_meta");
const UNDO_LOGS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("undo_logs");
const BRIDGE_TRANSFERS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("bridge_transfers");
const MINTED_PROOFS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("minted_proofs");

#[derive(Clone)]
pub struct StateStore {
    db: Arc<Database>,
    pub smt: Arc<RwLock<SparseMerkleTree>>,
}

impl StateStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = Database::create(path)?;

        // Ensure tables exist
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(OBJECTS_TABLE)?;
            let _ = write_txn.open_table(SEALS_TABLE)?;
            let _ = write_txn.open_table(TRANSITIONS_TABLE)?;
            let _ = write_txn.open_table(BLOCKS_TABLE)?;
            let _ = write_txn.open_table(CHAIN_META_TABLE)?;
            let _ = write_txn.open_table(UNDO_LOGS_TABLE)?;
            let _ = write_txn.open_table(BRIDGE_TRANSFERS_TABLE)?;
            let _ = write_txn.open_table(MINTED_PROOFS_TABLE)?;
        }
        write_txn.commit()?;

        let mut smt = SparseMerkleTree::new();

        // Populate in-memory SMT from persisted objects
        let read_txn = db.begin_read()?;
        let objects_table = read_txn.open_table(OBJECTS_TABLE)?;
        for item in objects_table.iter()? {
            let (key_access, val_access) = item?;
            let obj_id = key_access.value();
            if let Ok(obj) = serde_json::from_slice::<SmartObjectRecord>(val_access.value()) {
                let key_hash = Self::hash_key(obj_id);
                let val_hash = Self::hash_object(&obj);
                smt.update(key_hash, val_hash);
            }
        }

        Ok(Self {
            db: Arc::new(db),
            smt: Arc::new(RwLock::new(smt)),
        })
    }

    pub fn hash_key(key: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let res = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&res);
        out
    }

    pub fn hash_object(obj: &SmartObjectRecord) -> [u8; 32] {
        // Serialize state_data canonically. serde_json without preserve_order
        // uses BTreeMap (sorted keys), so to_vec is deterministic.
        let state_bytes = serde_json::to_vec(&obj.state_data).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(obj.object_id.as_bytes());
        hasher.update(obj.code_hash.as_bytes());
        hasher.update(obj.seal.as_bytes());
        hasher.update(&obj.satoshis.to_be_bytes());
        hasher.update(obj.owner.as_bytes());
        hasher.update(&state_bytes);
        let res = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&res);
        out
    }

    pub fn save_object(&self, obj: &SmartObjectRecord) -> Result<()> {
        let serialized = serde_json::to_vec(obj)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut obj_table = write_txn.open_table(OBJECTS_TABLE)?;
            let mut seals_table = write_txn.open_table(SEALS_TABLE)?;

            if let Some(prev_val) = obj_table.get(obj.object_id.as_str())? {
                if let Ok(prev_obj) = serde_json::from_slice::<SmartObjectRecord>(prev_val.value()) {
                    if prev_obj.seal != obj.seal {
                        seals_table.remove(prev_obj.seal.as_str())?;
                    }
                }
            }

            obj_table.insert(obj.object_id.as_str(), serialized.as_slice())?;
            seals_table.insert(obj.seal.as_str(), obj.object_id.as_str())?;
        }
        write_txn.commit()?;

        // Update in-memory SMT — return error if this fails (don't silently succeed)
        let key_hash = Self::hash_key(&obj.object_id);
        let val_hash = Self::hash_object(obj);
        match self.smt.write() {
            Ok(mut smt) => {
                smt.update(key_hash, val_hash);
            }
            Err(e) => {
                // SMT update failed — DB is committed but SMT is stale.
                // Return error so caller knows state is inconsistent.
                return Err(anyhow::anyhow!("SMT update failed after DB commit: {}", e));
            }
        }

        Ok(())
    }

    pub fn get_object(&self, object_id: &str) -> Result<Option<SmartObjectRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(OBJECTS_TABLE)?;
        if let Some(val) = table.get(object_id)? {
            let obj: SmartObjectRecord = serde_json::from_slice(val.value())?;
            Ok(Some(obj))
        } else {
            Ok(None)
        }
    }

    pub fn get_object_by_seal(&self, seal: &str) -> Result<Option<SmartObjectRecord>> {
        let read_txn = self.db.begin_read()?;
        let seals_table = read_txn.open_table(SEALS_TABLE)?;
        if let Some(id_access) = seals_table.get(seal)? {
            let obj_id = id_access.value();
            self.get_object(obj_id)
        } else {
            Ok(None)
        }
    }

    pub fn get_all_objects(&self) -> Result<Vec<SmartObjectRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(OBJECTS_TABLE)?;
        let mut objects = Vec::new();
        for item in table.iter()? {
            let (_, val) = item?;
            let obj: SmartObjectRecord = serde_json::from_slice(val.value())?;
            objects.push(obj);
        }
        Ok(objects)
    }

    pub fn save_transition(&self, trans: &StateTransitionRecord) -> Result<()> {
        let serialized = serde_json::to_vec(trans)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TRANSITIONS_TABLE)?;
            table.insert(trans.txid.as_str(), serialized.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_transitions_for_object(&self, object_id: &str) -> Result<Vec<StateTransitionRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSITIONS_TABLE)?;
        let mut list = Vec::new();
        for item in table.iter()? {
            let (_, val) = item?;
            let rec: StateTransitionRecord = serde_json::from_slice(val.value())?;
            if rec.object_id == object_id {
                list.push(rec);
            }
        }
        list.sort_by_key(|t| t.block_height);
        Ok(list)
    }

    pub fn save_block(&self, block: &BlockRecord) -> Result<()> {
        let serialized = serde_json::to_vec(block)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(BLOCKS_TABLE)?;
            table.insert(block.block_height, serialized.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_block(&self, height: u64) -> Result<Option<BlockRecord>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(BLOCKS_TABLE)?;
        if let Some(val) = table.get(height)? {
            let block: BlockRecord = serde_json::from_slice(val.value())?;
            Ok(Some(block))
        } else {
            Ok(None)
        }
    }

    pub fn get_last_sync_block(&self, chain: &str) -> Result<u64> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(CHAIN_META_TABLE)?;
        let key = format!("{}_last_block", chain);
        if let Some(val) = table.get(key.as_str())? {
            let height: u64 = serde_json::from_slice(val.value())?;
            Ok(height)
        } else {
            Ok(0)
        }
    }

    pub fn set_last_sync_block(&self, chain: &str, height: u64) -> Result<()> {
        let key = format!("{}_last_block", chain);
        let serialized = serde_json::to_vec(&height)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(CHAIN_META_TABLE)?;
            table.insert(key.as_str(), serialized.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn current_state_root(&self) -> String {
        match self.smt.write() {
            Ok(mut smt) => hex::encode(smt.root()),
            Err(e) => {
                tracing::error!("[Storage] Failed to acquire write lock on SMT: {}", e);
                String::new()
            }
        }
    }

    pub fn get_state_proof(&self, object_id: &str) -> Option<SmtInclusionProof> {
        let key_hash = Self::hash_key(object_id);
        match self.smt.read() {
            Ok(smt) => smt.get_proof(&key_hash),
            Err(e) => {
                tracing::error!("[Storage] Failed to acquire read lock on SMT: {}", e);
                None
            }
        }
    }

    pub fn save_undo_log(&self, undo: &UndoLogRecord) -> Result<()> {
        let serialized = serde_json::to_vec(undo)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(UNDO_LOGS_TABLE)?;
            table.insert(undo.id, serialized.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn rollback_to_block(&mut self, chain: &str, target_height: u64) -> Result<usize> {
        let mut rolled_back = 0;
        let write_txn = self.db.begin_write()?;
        {
            let mut undo_table = write_txn.open_table(UNDO_LOGS_TABLE)?;
            let mut obj_table = write_txn.open_table(OBJECTS_TABLE)?;
            let mut seals_table = write_txn.open_table(SEALS_TABLE)?;
            let mut blocks_table = write_txn.open_table(BLOCKS_TABLE)?;
            let mut transitions_table = write_txn.open_table(TRANSITIONS_TABLE)?;
            let mut chain_meta_table = write_txn.open_table(CHAIN_META_TABLE)?;

            let mut to_delete_ids = Vec::new();
            let mut stale_block_heights = Vec::new();
            for item in undo_table.iter()? {
                let (id_acc, val_acc) = item?;
                let log: UndoLogRecord = serde_json::from_slice(val_acc.value())?;
                if log.chain == chain && log.block_height > target_height {
                    to_delete_ids.push(id_acc.value());
                    stale_block_heights.push(log.block_height);

                    if log.action == "CREATE" {
                        // Undo create means delete object & remove its seal
                        if let Some(obj_val) = obj_table.get(log.object_id.as_str())? {
                            if let Ok(existing) = serde_json::from_slice::<SmartObjectRecord>(obj_val.value()) {
                                seals_table.remove(existing.seal.as_str())?;
                            }
                        }
                        obj_table.remove(log.object_id.as_str())?;
                    } else if log.action == "UPDATE" {
                        // Remove newer seal
                        if let Some(obj_val) = obj_table.get(log.object_id.as_str())? {
                            if let Ok(existing) = serde_json::from_slice::<SmartObjectRecord>(obj_val.value()) {
                                seals_table.remove(existing.seal.as_str())?;
                            }
                        }
                        // Restore previous state — preserve created_at_block
                        if let (Some(code_hash), Some(seal), Some(sats), Some(owner), Some(state)) = (
                            log.prev_code_hash,
                            log.prev_seal,
                            log.prev_satoshis,
                            log.prev_owner,
                            log.prev_state_data,
                        ) {
                            let prev_obj = SmartObjectRecord {
                                object_id: log.object_id.clone(),
                                code_hash,
                                seal: seal.clone(),
                                satoshis: sats,
                                owner,
                                state_data: state,
                                created_at_block: log.prev_created_at_block.unwrap_or(0),
                                updated_at_block: log.prev_updated_at_block.unwrap_or(target_height),
                            };
                            let ser = serde_json::to_vec(&prev_obj)?;
                            obj_table.insert(log.object_id.as_str(), ser.as_slice())?;
                            seals_table.insert(seal.as_str(), log.object_id.as_str())?;
                        }
                    }
                    rolled_back += 1;
                }
            }

            for id in to_delete_ids {
                undo_table.remove(id)?;
            }

            // Delete stale block records (blocks above target_height)
            for h in stale_block_heights {
                let _ = blocks_table.remove(h);
            }

            // Delete stale transition records — scan and remove by block_height
            let mut stale_txids = Vec::new();
            for item in transitions_table.iter()? {
                let (txid_acc, val_acc) = item?;
                if let Ok(rec) = serde_json::from_slice::<StateTransitionRecord>(val_acc.value()) {
                    if rec.block_height > target_height {
                        stale_txids.push(txid_acc.value().to_string());
                    }
                }
            }
            for txid in stale_txids {
                let _ = transitions_table.remove(txid.as_str());
            }

            // Reset last_sync_block to target_height
            let key = format!("{}_last_block", chain);
            let serialized = serde_json::to_vec(&target_height)?;
            chain_meta_table.insert(key.as_str(), serialized.as_slice())?;
        }
        write_txn.commit()?;

        // Rebuild in-memory SMT
        let all_objects = self.get_all_objects()?;
        let mut new_smt = SparseMerkleTree::new();
        for obj in all_objects {
            let k = Self::hash_key(&obj.object_id);
            let v = Self::hash_object(&obj);
            new_smt.update(k, v);
        }
        
        match self.smt.write() {
            Ok(mut smt) => {
                *smt = new_smt;
            }
            Err(e) => {
                tracing::error!("[Storage] Failed to acquire write lock on SMT for rebuild: {}", e);
            }
        }

        Ok(rolled_back)
    }

    // ── Bridge transfer persistence ─────────────────────────────────────────

    pub fn save_bridge_transfer(
        &self,
        transfer: &crate::cross_chain::bridge::BridgeTransfer,
    ) -> Result<()> {
        let serialized = serde_json::to_vec(transfer)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(BRIDGE_TRANSFERS_TABLE)?;
            table.insert(transfer.id.as_str(), serialized.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_bridge_transfer(
        &self,
        transfer_id: &str,
    ) -> Result<Option<crate::cross_chain::bridge::BridgeTransfer>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(BRIDGE_TRANSFERS_TABLE)?;
        if let Some(val) = table.get(transfer_id)? {
            let transfer: crate::cross_chain::bridge::BridgeTransfer =
                serde_json::from_slice(val.value())?;
            Ok(Some(transfer))
        } else {
            Ok(None)
        }
    }

    // ── Minted proof persistence (replay guard survives restart) ───────────

    /// Atomically check whether `object_id` was already minted, and if not,
    /// record it as minted by `transfer_id`. Returns Ok(true) if newly inserted,
    /// Ok(false) if already present (replay), Err on storage failure.
    pub fn check_and_insert_minted_proof(
        &self,
        object_id: &str,
        transfer_id: &str,
    ) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(MINTED_PROOFS_TABLE)?;
            if let Some(existing) = table.get(object_id)? {
                // Already minted — check if it's the same transfer (idempotent)
                if existing.value() == transfer_id {
                    return Ok(false); // same transfer, not a replay
                }
                // Different transfer — this is a replay
                return Ok(false);
            }
            // Not yet minted — insert atomically
            table.insert(object_id, transfer_id)?;
        }
        write_txn.commit()?;
        Ok(true)
    }

    /// Read-through check: was this object_id already minted?
    pub fn get_minted_proof(&self, object_id: &str) -> Result<Option<String>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(MINTED_PROOFS_TABLE)?;
        if let Some(val) = table.get(object_id)? {
            Ok(Some(val.value().to_string()))
        } else {
            Ok(None)
        }
    }
}
