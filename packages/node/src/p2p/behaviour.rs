use libp2p::swarm::NetworkBehaviour;
use libp2p::{gossipsub, identify, kad, ping};

pub const DEFAULT_P2P_PORT: u16 = 2232;
pub const TOPIC_MEMPOOL: &str = "utxovm/mempool/v1";
pub const TOPIC_ATTESTATION: &str = "utxovm/attestation/v1";
pub const TOPIC_CONTRACT: &str = "utxovm/contract/v1";

#[derive(NetworkBehaviour)]
pub struct UtxoVmBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}
