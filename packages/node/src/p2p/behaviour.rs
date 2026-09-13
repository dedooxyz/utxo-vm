use libp2p::swarm::NetworkBehaviour;
use libp2p::{gossipsub, identify, kad, ping};

pub const DEFAULT_P2P_PORT: u16 = 2232;
pub const TOPIC_MEMPOOL: &str = "utxovm/mempool/v1";
pub const TOPIC_ATTESTATION: &str = "utxovm/attestation/v1";
pub const TOPIC_CONTRACT: &str = "utxovm/contract/v1";

// Issue 24: Kademlia uses an in-memory store (MemoryStore), not a disk-backed
// store. This is intentional and correct for this codebase:
//
// - The authoritative local source of contract bytecode is the persistent
//   Redb store (storage/db.rs::save_wasm / get_wasm). The scanner's
//   get_wasm_bytes checks Redb first; the DHT is only a fallback for
//   contracts not deployed locally.
// - The DHT MemoryStore is a cache for serving contracts to OTHER peers via
//   Kademlia get_record. Losing it on restart only affects this node's
//   ability to serve contracts to peers that request them via DHT — not its
//   own ability to execute contracts it has already seen (those are in Redb).
// - GossipSub (TOPIC_CONTRACT) replicates newly-deployed contracts to all
//   online peers' DHT caches on deployment, providing redundancy.
// - save_wasm's content-hash verification (sha256(wasm) == code_hash) means
//   a re-fetched copy from any peer is just as trustworthy as the original.
//
// The only failure mode is if ALL nodes that received a contract via
// GossipSub restart before any of them re-serves it via DHT — but even
// then, each restarting node still has the contract in its own Redb store
// for local execution. The DHT cache is not a source of truth.
#[derive(NetworkBehaviour)]
pub struct UtxoVmBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}
