use std::path::Path;
use std::sync::{Arc, RwLock};
use anyhow::Result;
use redb::{Database, ReadableTable, TableDefinition};
use sha2::{Digest, Sha256};
use crate::storage::smt::SparseMerkleTree;
use crate::types::{BlockRecord, SmartObjectRecord, SmtInclusionProof, StateTransitionRecord, UndoLogRecord};

const OBJECTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("objects");
const SEALS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("seals");
const TRANSITIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("transitions");
const BLOCKS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const CHAIN_META_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("chain_meta");
const UNDO_LOGS_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("undo_logs");

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
        let mut hasher = Sha256::new();
        hasher.update(obj.object_id.as_bytes());
        hasher.update(obj.code_hash.as_bytes());
        hasher.update(obj.seal.as_bytes());
        hasher.update(&obj.satoshis.to_be_bytes());
        hasher.update(obj.owner.as_bytes());
        hasher.update(obj.state_data.to_string().as_bytes());
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

        // Update in-memory SMT
        let key_hash = Self::hash_key(&obj.object_id);
        let val_hash = Self::hash_object(obj);
        self.smt.write().unwrap().update(key_hash, val_hash);

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
        let root = self.smt.read().unwrap().root();
        hex::encode(root)
    }

    pub fn get_state_proof(&self, object_id: &str) -> Option<SmtInclusionProof> {
        let key_hash = Self::hash_key(object_id);
        self.smt.read().unwrap().get_proof(&key_hash)
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

            let mut to_delete_ids = Vec::new();
            for item in undo_table.iter()? {
                let (id_acc, val_acc) = item?;
                let log: UndoLogRecord = serde_json::from_slice(val_acc.value())?;
                if log.chain == chain && log.block_height > target_height {
                    to_delete_ids.push(id_acc.value());

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
                        // Restore previous state
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
                                created_at_block: 0,
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
        *self.smt.write().unwrap() = new_smt;

        Ok(rolled_back)
    }
}
