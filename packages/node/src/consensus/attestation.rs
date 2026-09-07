use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use anyhow::Result;
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use crate::types::StateAttestation;

pub struct ConsensusManager {
    secp: Secp256k1<secp256k1::All>,
    pub known_validators: Arc<RwLock<HashSet<String>>>, // Set of hex pubkeys
    // (chain, height) -> Vec<StateAttestation>
    pub attestations: Arc<RwLock<HashMap<(String, u64), Vec<StateAttestation>>>>,
    pub quorum_threshold: usize,
}

impl ConsensusManager {
    pub fn new(quorum_threshold: usize) -> Self {
        Self {
            secp: Secp256k1::new(),
            known_validators: Arc::new(RwLock::new(HashSet::new())),
            attestations: Arc::new(RwLock::new(HashMap::new())),
            quorum_threshold,
        }
    }

    pub fn register_validator(&self, pubkey_hex: &str) {
        self.known_validators
            .write()
            .unwrap()
            .insert(pubkey_hex.to_string());
    }

    pub fn hash_attestation_payload(chain: &str, height: u64, block_hash: &str, state_root: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"UTXO_VM_ATTESTATION");
        hasher.update(chain.as_bytes());
        hasher.update(&height.to_be_bytes());
        hasher.update(block_hash.as_bytes());
        hasher.update(state_root.as_bytes());
        let res = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&res);
        out
    }

    pub fn sign_state_root(
        &self,
        secret_key: &SecretKey,
        chain: &str,
        height: u64,
        block_hash: &str,
        state_root: &str,
    ) -> Result<StateAttestation> {
        let digest = Self::hash_attestation_payload(chain, height, block_hash, state_root);
        let msg = Message::from_digest(digest);
        let sig = self.secp.sign_ecdsa(&msg, secret_key);
        let pubkey = PublicKey::from_secret_key(&self.secp, secret_key);

        Ok(StateAttestation {
            chain: chain.to_string(),
            block_height: height,
            block_hash: block_hash.to_string(),
            state_root: state_root.to_string(),
            validator_pubkey: hex::encode(pubkey.serialize()),
            signature_hex: hex::encode(sig.serialize_compact()),
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    pub fn verify_attestation(&self, attestation: &StateAttestation) -> bool {
        let pubkey_bytes = match hex::decode(&attestation.validator_pubkey) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let pubkey = match PublicKey::from_slice(&pubkey_bytes) {
            Ok(pk) => pk,
            Err(_) => return false,
        };

        let sig_bytes = match hex::decode(&attestation.signature_hex) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = match Signature::from_compact(&sig_bytes) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let digest = Self::hash_attestation_payload(
            &attestation.chain,
            attestation.block_height,
            &attestation.block_hash,
            &attestation.state_root,
        );
        let msg = Message::from_digest(digest);

        self.secp.verify_ecdsa(&msg, &sig, &pubkey).is_ok()
    }

    pub fn add_attestation(&self, attestation: StateAttestation) -> bool {
        if !self.verify_attestation(&attestation) {
            return false;
        }

        let key = (attestation.chain.clone(), attestation.block_height);
        let mut guard = self.attestations.write().unwrap();
        let list = guard.entry(key).or_default();

        // Check duplicate from same validator
        if list.iter().any(|a| a.validator_pubkey == attestation.validator_pubkey) {
            return false;
        }

        list.push(attestation);
        true
    }

    pub fn is_quorum_reached(&self, chain: &str, height: u64, state_root: &str) -> bool {
        let guard = self.attestations.read().unwrap();
        let key = (chain.to_string(), height);
        if let Some(list) = guard.get(&key) {
            let matching_count = list.iter().filter(|a| a.state_root == state_root).count();
            matching_count >= self.quorum_threshold
        } else {
            false
        }
    }

    pub fn get_attestations(&self, chain: &str, height: u64) -> Vec<StateAttestation> {
        let guard = self.attestations.read().unwrap();
        let key = (chain.to_string(), height);
        guard.get(&key).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify_attestation() {
        let mgr = ConsensusManager::new(1);
        let secp = Secp256k1::new();
        let (sk, _pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

        let attestation = mgr
            .sign_state_root(&sk, "JKC", 100, "hash_abc", "state_root_123")
            .expect("Signing should succeed");

        assert_eq!(mgr.verify_attestation(&attestation), true);
        assert_eq!(mgr.add_attestation(attestation), true);
        assert_eq!(mgr.is_quorum_reached("JKC", 100, "state_root_123"), true);
        assert_eq!(mgr.is_quorum_reached("JKC", 100, "wrong_root"), false);
    }
}
