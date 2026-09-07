use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartObjectRecord {
    pub object_id: String,
    pub code_hash: String,
    pub seal: String, // txid:vout
    pub satoshis: u64,
    pub owner: String,
    pub state_data: serde_json::Value,
    pub created_at_block: u64,
    pub updated_at_block: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateTransitionRecord {
    pub txid: String,
    pub object_id: String,
    pub prev_seal: String,
    pub new_seal: String,
    pub caller: String,
    pub method: String,
    pub args: serde_json::Value,
    pub new_state: serde_json::Value,
    pub satoshis: u64,
    pub block_height: u64,
    pub gas_consumed: u64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockRecord {
    pub chain: String,
    pub block_height: u64,
    pub block_hash: String,
    pub prev_hash: Option<String>,
    pub state_root: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoLogRecord {
    pub id: u64,
    pub chain: String,
    pub block_height: u64,
    pub object_id: String,
    pub action: String, // "CREATE" or "UPDATE"
    pub prev_code_hash: Option<String>,
    pub prev_seal: Option<String>,
    pub prev_satoshis: Option<u64>,
    pub prev_owner: Option<String>,
    pub prev_state_data: Option<serde_json::Value>,
    pub prev_updated_at_block: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoVmEnvelope {
    pub protocol: String,
    pub version: u8,
    pub content_type: String,
    pub payload: Vec<u8>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateAttestation {
    pub chain: String,
    pub block_height: u64,
    pub block_hash: String,
    pub state_root: String,
    pub validator_pubkey: String,
    pub signature_hex: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtProofNode {
    pub position: String, // "left" or "right"
    pub hash_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtInclusionProof {
    pub key_hex: String,
    pub value_hex: String,
    pub root_hex: String,
    pub proof_path: Vec<SmtProofNode>,
    pub verified: bool,
}
