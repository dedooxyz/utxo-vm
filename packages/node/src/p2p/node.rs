use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use futures::StreamExt;
use libp2p::gossipsub::{
    self, IdentTopic as Topic, MessageAuthenticity, ValidationMode,
};
use libp2p::kad::store::RecordStore;
use libp2p::kad::{self, QueryId, QueryResult, Record, RecordKey};
use libp2p::kad::GetRecordOk;
use libp2p::noise;
use libp2p::swarm::SwarmEvent;
use libp2p::tcp;
use libp2p::yamux;
use libp2p::{identify, ping, Multiaddr, PeerId};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{error, info, warn};

use crate::p2p::behaviour::{
    UtxoVmBehaviour, UtxoVmBehaviourEvent, TOPIC_ATTESTATION, TOPIC_CONTRACT, TOPIC_MEMPOOL,
};
use crate::types::StateAttestation;

pub enum P2pCommand {
    BroadcastMempool(Vec<u8>),
    BroadcastAttestation(StateAttestation),
    PutContract {
        code_hash: String,
        wasm_bytes: Vec<u8>,
        resp: oneshot::Sender<Result<()>>,
    },
    GetContract {
        code_hash: String,
        resp: oneshot::Sender<Option<Vec<u8>>>,
    },
    GetPeers(oneshot::Sender<Vec<String>>),
    ProcessMempoolTx(Vec<u8>),
}

#[derive(Clone)]
pub struct P2pHandle {
    sender: mpsc::Sender<P2pCommand>,
    pub local_peer_id: String,
}

impl P2pHandle {
    pub async fn broadcast_mempool(&self, data: Vec<u8>) -> Result<()> {
        self.sender.send(P2pCommand::BroadcastMempool(data)).await?;
        Ok(())
    }

    pub async fn broadcast_attestation(&self, attestation: StateAttestation) -> Result<()> {
        self.sender
            .send(P2pCommand::BroadcastAttestation(attestation))
            .await?;
        Ok(())
    }

    pub async fn put_contract(&self, code_hash: String, wasm_bytes: Vec<u8>) -> Result<()> {
        let (resp, rx) = oneshot::channel();
        self.sender
            .send(P2pCommand::PutContract {
                code_hash,
                wasm_bytes,
                resp,
            })
            .await?;
        match tokio::time::timeout(Duration::from_secs(2), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => Err(anyhow::anyhow!("P2P channel error: {:?}", e)),
            Err(_) => Err(anyhow::anyhow!("Timeout waiting for P2P DHT put response")),
        }
    }

    pub async fn get_contract(&self, code_hash: String) -> Result<Option<Vec<u8>>> {
        let (resp, rx) = oneshot::channel();
        self.sender
            .send(P2pCommand::GetContract { code_hash, resp })
            .await?;
        match tokio::time::timeout(Duration::from_secs(2), rx).await {
            Ok(Ok(opt)) => Ok(opt),
            _ => Ok(None),
        }
    }

    pub async fn get_peers(&self) -> Result<Vec<String>> {
        let (resp, rx) = oneshot::channel();
        self.sender.send(P2pCommand::GetPeers(resp)).await?;
        match tokio::time::timeout(Duration::from_millis(500), rx).await {
            Ok(Ok(peers)) => Ok(peers),
            _ => Ok(Vec::new()),
        }
    }
}

pub struct P2pService {
    port: u16,
    bootstrap_peers: Vec<String>,
    cmd_rx: mpsc::Receiver<P2pCommand>,
    id_keys: libp2p::identity::Keypair,
}

/// Load or generate Ed25519 keypair for persistent peer identity.
/// Key is saved to `key_path` (hex-encoded) and reused across restarts.
fn load_or_generate_keypair(key_path: &std::path::Path) -> libp2p::identity::Keypair {
    if key_path.exists() {
        if let Ok(hex_str) = std::fs::read_to_string(key_path) {
            if let Ok(bytes) = hex::decode(hex_str.trim()) {
                if let Ok(keypair) = libp2p::identity::Keypair::ed25519_from_bytes(bytes.clone()) {
                    info!("[P2P] Loaded existing identity key from {:?}", key_path);
                    return keypair;
                }
            }
        }
        warn!("[P2P] Could not read key file, generating new identity");
    }

    let keypair = libp2p::identity::Keypair::generate_ed25519();

    // Persist the key via protobuf encoding
    if let Ok(proto_bytes) = keypair.clone().to_protobuf_encoding() {
        if let Some(parent) = key_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(key_path, hex::encode(proto_bytes));
        info!("[P2P] Generated and saved new identity key to {:?}", key_path);
    } else {
        warn!("[P2P] Generated ephemeral key (cannot persist)");
    }

    keypair
}

impl P2pService {
    pub fn new(port: u16, bootstrap_peers: Vec<String>, key_path: Option<PathBuf>) -> (Self, P2pHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(100);

        let id_keys = match key_path {
            Some(ref path) => load_or_generate_keypair(path),
            None => {
                warn!("[P2P] No key path provided, using ephemeral identity");
                libp2p::identity::Keypair::generate_ed25519()
            }
        };

        let local_peer_id = id_keys.public().to_peer_id().to_string();
        let handle = P2pHandle {
            sender: cmd_tx,
            local_peer_id,
        };
        (
            Self {
                port,
                bootstrap_peers,
                cmd_rx,
                id_keys,
            },
            handle,
        )
    }

    pub async fn run(mut self, on_attestation: Option<mpsc::Sender<StateAttestation>>, on_mempool: Option<mpsc::Sender<Vec<u8>>>) -> Result<()> {
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(self.id_keys)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                // Gossipsub configuration
                // Use SHA256 for deterministic message IDs — DefaultHasher is non-deterministic
                // across processes and Rust versions, which breaks gossip deduplication.
                let message_id_fn = |message: &gossipsub::Message| {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&message.data);
                    gossipsub::MessageId::from(hex::encode(hasher.finalize()))
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(1))
                    .validation_mode(ValidationMode::Strict)
                    .message_id_fn(message_id_fn)
                    .build()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

                let mut gossipsub = gossipsub::Behaviour::new(
                    MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

                // Subscribe to default topics
                gossipsub.subscribe(&Topic::new(TOPIC_MEMPOOL))?;
                gossipsub.subscribe(&Topic::new(TOPIC_ATTESTATION))?;
                gossipsub.subscribe(&Topic::new(TOPIC_CONTRACT))?;

                // Kademlia DHT configuration
                let peer_id = key.public().to_peer_id();
                let store = kad::store::MemoryStore::new(peer_id);
                let kad_config = kad::Config::new(kad::PROTOCOL_NAME);
                let kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);

                // Identify & Ping
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/utxovm/1.0.0".to_string(),
                    key.public(),
                ));
                let ping = ping::Behaviour::default();

                Ok(UtxoVmBehaviour {
                    gossipsub,
                    kademlia,
                    identify,
                    ping,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        let local_peer_id = *swarm.local_peer_id();
        info!("[P2P] Local peer identity: {}", local_peer_id);

        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.port).parse()?;
        swarm.listen_on(listen_addr.clone())?;
        info!("[P2P] Listening on multiaddr: {}", listen_addr);

        // Connect to bootstrap peers
        for peer_addr in &self.bootstrap_peers {
            if let Ok(addr) = peer_addr.parse::<Multiaddr>() {
                if let Err(e) = swarm.dial(addr.clone()) {
                    warn!("[P2P] Failed to dial bootstrap peer {}: {:?}", addr, e);
                } else {
                    info!("[P2P] Dialing bootstrap peer: {}", addr);
                }
            }
        }

        let connected_peers = Arc::new(RwLock::new(Vec::<PeerId>::new()));
        let local_peer_id_for_kad = local_peer_id;
        let mut pending_dht_gets: HashMap<QueryId, oneshot::Sender<Option<Vec<u8>>>> = HashMap::new();

        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(P2pCommand::BroadcastMempool(data)) => {
                            let topic = Topic::new(TOPIC_MEMPOOL);
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                                error!("[P2P] Failed to publish mempool tx: {:?}", e);
                            }
                        }
                        Some(P2pCommand::BroadcastAttestation(attestation)) => {
                            if let Ok(data) = serde_json::to_vec(&attestation) {
                                let topic = Topic::new(TOPIC_ATTESTATION);
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                                    error!("[P2P] Failed to publish state attestation: {:?}", e);
                                }
                            }
                        }
                        Some(P2pCommand::PutContract { code_hash, wasm_bytes, resp }) => {
                            // Store locally in Kademlia store
                            let key = RecordKey::new(&code_hash);
                            let record = Record {
                                key,
                                value: wasm_bytes.clone(),
                                publisher: Some(local_peer_id_for_kad),
                                expires: None,
                            };
                            if let Err(e) = swarm.behaviour_mut().kademlia.store_mut().put(record) {
                                warn!("[P2P] Local store failed: {:?}", e);
                            }
                            // Broadcast via GossipSub for cross-node replication
                            let payload = serde_json::json!({
                                "code_hash": code_hash,
                                "wasm_hex": hex::encode(&wasm_bytes),
                            });
                            if let Ok(data) = serde_json::to_vec(&payload) {
                                let topic = Topic::new(TOPIC_CONTRACT);
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                                    warn!("[P2P] Failed to broadcast contract: {:?}", e);
                                }
                            }
                            let _ = resp.send(Ok(()));
                        }
                        Some(P2pCommand::GetContract { code_hash, resp }) => {
                            // First try local store
                            let key = RecordKey::new(&code_hash);
                            if let Some(record) = swarm.behaviour_mut().kademlia.store_mut().get(&key) {
                                let _ = resp.send(Some(record.value.to_vec()));
                            } else {
                                // Query DHT network
                                let query_id = swarm.behaviour_mut().kademlia.get_record(key);
                                // Store pending query to resolve when result arrives
                                pending_dht_gets.insert(query_id, resp);
                            }
                        }
                        Some(P2pCommand::GetPeers(resp)) => {
                            let peers = connected_peers.read().await.iter().map(|p| p.to_string()).collect();
                            let _ = resp.send(peers);
                        }
                        Some(P2pCommand::ProcessMempoolTx(_)) => {
                            // Mempool tx processing is handled via the on_mempool channel
                        }
                        None => break,
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!("[P2P] Swarm listening on {}", address);
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                            info!("[P2P] Connection established with peer {}", peer_id);
                            connected_peers.write().await.push(peer_id);
                            let observed_addr = endpoint.get_remote_address();
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, observed_addr.clone());
                            // Re-bootstrap Kademlia after adding new peer
                            if let Ok(query_id) = swarm.behaviour_mut().kademlia.bootstrap() {
                                info!("[P2P] Kademlia bootstrap triggered: {:?}", query_id);
                            }
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            info!("[P2P] Connection closed with peer {}", peer_id);
                            connected_peers.write().await.retain(|p| *p != peer_id);
                        }
                        SwarmEvent::Behaviour(UtxoVmBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                            message,
                            ..
                        })) => {
                            if message.topic.as_str() == TOPIC_ATTESTATION {
                                if let Ok(attestation) = serde_json::from_slice::<StateAttestation>(&message.data) {
                                    info!("[P2P] Received StateAttestation for height #{} from {}", attestation.block_height, attestation.validator_pubkey);
                                    if let Some(ref sender) = on_attestation {
                                        let _ = sender.send(attestation).await;
                                    }
                                }
                            } else if message.topic.as_str() == TOPIC_MEMPOOL {
                                info!("[P2P] Received mempool transaction ({} bytes)", message.data.len());
                                if let Some(ref sender) = on_mempool {
                                    let _ = sender.send(message.data).await;
                                }
                            } else if message.topic.as_str() == TOPIC_CONTRACT {
                                if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                                    if let (Some(code_hash), Some(wasm_hex)) = (payload.get("code_hash"), payload.get("wasm_hex")) {
                                        if let (Some(ch), Some(wh)) = (code_hash.as_str(), wasm_hex.as_str()) {
                                            if let Ok(wasm_bytes) = hex::decode(wh) {
                                                let key = RecordKey::new(&ch.as_bytes());
                                                let record = Record {
                                                    key,
                                                    value: wasm_bytes,
                                                    publisher: None,
                                                    expires: None,
                                                };
                                                if swarm.behaviour_mut().kademlia.store_mut().put(record).is_ok() {
                                                    info!("[P2P] Stored remote contract via GossipSub: {}", ch);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SwarmEvent::Behaviour(UtxoVmBehaviourEvent::Kademlia(
                            kad::Event::OutboundQueryProgressed { result, id, .. },
                        )) => {
                            if let Some(tx) = pending_dht_gets.remove(&id) {
                                match result {
                                    QueryResult::GetRecord(Ok(GetRecordOk::FoundRecord(record))) => {
                                        let _ = tx.send(Some(record.record.value));
                                    }
                                    QueryResult::GetRecord(Err(e)) => {
                                        warn!("[P2P] DHT get_record query failed: {:?}", e);
                                        let _ = tx.send(None);
                                    }
                                    _ => {
                                        let _ = tx.send(None);
                                    }
                                }
                            }
                        }
                        SwarmEvent::Behaviour(UtxoVmBehaviourEvent::Identify(
                            identify::Event::Received { peer_id, info, .. },
                        )) => {
                            info!("[P2P] Identify received from {}: listen_addrs={:?}", peer_id, info.listen_addrs);
                            for addr in &info.listen_addrs {
                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                            }
                            if let Ok(query_id) = swarm.behaviour_mut().kademlia.bootstrap() {
                                info!("[P2P] Kademlia re-bootstrap after identify: {:?}", query_id);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }
}
