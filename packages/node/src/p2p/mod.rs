pub mod behaviour;
pub mod node;

pub use behaviour::{DEFAULT_P2P_PORT, TOPIC_ATTESTATION, TOPIC_MEMPOOL};
pub use node::{P2pHandle, P2pService};
