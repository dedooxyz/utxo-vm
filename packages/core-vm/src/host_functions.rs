use crate::state::{CreatedObject, EventLog, SingleUseSeal, SmartObjectState, StealthSettlement};

#[derive(Debug, Clone)]
pub struct HostContext {
    pub caller: String,
    pub seal: SingleUseSeal,
    pub satoshis: u64,
    pub events: Vec<EventLog>,
    pub created_objects: Vec<CreatedObject>,
    pub stealth_settlements: Vec<StealthSettlement>,
    pub mweb_peg_outs: Vec<StealthSettlement>,
}

impl HostContext {
    pub fn new(caller: String, seal: SingleUseSeal, satoshis: u64) -> Self {
        Self {
            caller,
            seal,
            satoshis,
            events: Vec::new(),
            created_objects: Vec::new(),
            stealth_settlements: Vec::new(),
            mweb_peg_outs: Vec::new(),
        }
    }

    pub fn from_state(state: &SmartObjectState, caller: String) -> Self {
        Self {
            caller,
            seal: state.seal.clone(),
            satoshis: state.satoshis,
            events: Vec::new(),
            created_objects: Vec::new(),
            stealth_settlements: Vec::new(),
            mweb_peg_outs: Vec::new(),
        }
    }
}
