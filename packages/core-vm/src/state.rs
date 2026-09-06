use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleUseSeal {
    pub txid: String,
    pub vout: u32,
}

impl std::fmt::Display for SingleUseSeal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLog {
    pub topic: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedObject {
    pub code_hash: String,
    pub initial_state: Vec<u8>,
    pub satoshis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StealthSettlement {
    pub stealth_address: String,
    pub satoshis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartObjectState {
    pub object_id: String,
    pub code_hash: String,
    pub seal: SingleUseSeal,
    pub satoshis: u64,
    pub owner_pubkey: String,
    pub state_data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDelta {
    pub object_id: String,
    pub consumed_seal: SingleUseSeal,
    pub new_seal: SingleUseSeal,
    pub new_satoshis: u64,
    pub new_state_data: Vec<u8>,
    pub events: Vec<EventLog>,
    pub created_objects: Vec<CreatedObject>,
    pub stealth_settlements: Vec<StealthSettlement>,
}
