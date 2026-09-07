use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use futures::StreamExt;
use libp2p::gossipsub::{
    self, IdentTopic as Topic, MessageAuthenticity, ValidationMode,
};
use libp2p::kad::store::RecordStore;
use libp2p::kad::{self, Record, RecordKey};
use libp2p::noise;
use libp2p::swarm::SwarmEvent;
use libp2p::tcp;
use libp2p::yamux;
use libp2p::{identify, ping, Multiaddr, PeerId};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{error, info, warn};

use crate::p2p::behaviour::{
    UtxoVmBehaviour, UtxoVmBehaviourEvent, TOPIC_ATTESTATION, TOPIC_MEMPOOL,
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

impl P2pService {
    pub fn new(port: u16, bootstrap_peers: Vec<String>) -> (Self, P2pHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(100);
        let id_keys = libp2p::identity::Keypair::generate_ed25519();
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

    pub async fn run(mut self, on_attestation: Option<mpsc::Sender<StateAttestation>>) -> Result<()> {
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(self.id_keys)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                // Gossipsub configuration
                let message_id_fn = |message: &gossipsub::Message| {
                    let mut s = DefaultHasher::new();
                    message.data.hash(&mut s);
                    gossipsub::MessageId::from(s.finish().to_string())
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

                // Kademlia DHT configuration
                let peer_id = key.public().to_peer_id();
                let store = kad::store::MemoryStore::new(peer_id);
                #[allow(deprecated)]
                let kad_config = kad::Config::default();
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
                            let key = RecordKey::new(&code_hash);
                            let record = Record {
                                key,
                                value: wasm_bytes,
                                publisher: Some(local_peer_id),
                                expires: None,
                            };
                            let res = swarm.behaviour_mut().kademlia.put_record(record, kad::Quorum::One)
                                .map(|_| ())
                                .map_err(|e| anyhow::anyhow!("Kademlia put error: {:?}", e));
                            let _ = resp.send(res);
                        }
                        Some(P2pCommand::GetContract { code_hash, resp }) => {
                            // In memory lookup from Kademlia store
                            let key = RecordKey::new(&code_hash);
                            if let Some(record) = swarm.behaviour_mut().kademlia.store_mut().get(&key) {
                                let _ = resp.send(Some(record.value.to_vec()));
                            } else {
                                let _ = resp.send(None);
                            }
                        }
                        Some(P2pCommand::GetPeers(resp)) => {
                            let peers = connected_peers.read().await.iter().map(|p| p.to_string()).collect();
                            let _ = resp.send(peers);
                        }
                        None => break,
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!("[P2P] Swarm listening on {}", address);
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            info!("[P2P] Connection established with peer {}", peer_id);
                            connected_peers.write().await.push(peer_id);
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, "/ip4/127.0.0.1/tcp/2232".parse().unwrap());
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
                                info!("[P2P] Received mempool transaction envelope ({} bytes)", message.data.len());
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
