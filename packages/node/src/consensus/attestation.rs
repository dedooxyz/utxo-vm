use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use anyhow::Result;
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use crate::types::{EquivocationProof, StateAttestation};

pub struct ConsensusManager {
    secp: Secp256k1<secp256k1::All>,
    pub known_validators: Arc<RwLock<HashSet<String>>>, // Set of hex pubkeys
    // (chain, height) -> Vec<StateAttestation>
    pub attestations: Arc<RwLock<HashMap<(String, u64), Vec<StateAttestation>>>>,
    pub slashing_proofs: Arc<RwLock<Vec<EquivocationProof>>>,
    pub quorum_threshold: usize,
}

impl ConsensusManager {
    pub fn new(quorum_threshold: usize) -> Self {
        Self {
            secp: Secp256k1::new(),
            known_validators: Arc::new(RwLock::new(HashSet::new())),
            attestations: Arc::new(RwLock::new(HashMap::new())),
            slashing_proofs: Arc::new(RwLock::new(Vec::new())),
            quorum_threshold,
        }
    }

    pub fn register_validator(&self, pubkey_hex: &str) {
        match self.known_validators.write() {
            Ok(mut validators) => {
                validators.insert(pubkey_hex.to_string());
            }
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire write lock on known_validators: {}", e);
            }
        }
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

        // F1.3: Fail-closed — if no validators are registered, reject all attestations
        match self.known_validators.read() {
            Ok(validators) => {
                if validators.is_empty() {
                    tracing::warn!(
                        "[Consensus] Rejected attestation: no validators registered (fail-closed)"
                    );
                    return false;
                }
                if !validators.contains(&attestation.validator_pubkey) {
                    tracing::warn!(
                        "[Consensus] Rejected attestation from unregistered validator: {}",
                        attestation.validator_pubkey
                    );
                    return false;
                }
            }
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire read lock on known_validators: {}", e);
                return false;
            }
        }

        let key = (attestation.chain.clone(), attestation.block_height);
        
        match self.attestations.write() {
            Ok(mut guard) => {
                let list = guard.entry(key).or_default();

                // Check duplicate from same validator
                if let Some(existing) = list.iter().find(|a| a.validator_pubkey == attestation.validator_pubkey) {
                    if existing.state_root != attestation.state_root {
                        let proof = EquivocationProof {
                            chain: attestation.chain.clone(),
                            block_height: attestation.block_height,
                            validator_pubkey: attestation.validator_pubkey.clone(),
                            first_attestation: existing.clone(),
                            second_attestation: attestation.clone(),
                            detected_at: chrono::Utc::now().timestamp(),
                        };
                        tracing::error!(
                            "[Slashing] CRITICAL: Equivocation detected for validator {} on chain {} at height #{}! Conflicting roots: {} vs {}",
                            proof.validator_pubkey,
                            proof.chain,
                            proof.block_height,
                            proof.first_attestation.state_root,
                            proof.second_attestation.state_root
                        );
                        if let Ok(mut proofs) = self.slashing_proofs.write() {
                            proofs.push(proof);
                        }
                    }
                    return false;
                }

                list.push(attestation);
                true
            }
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire write lock on attestations: {}", e);
                false
            }
        }
    }

    pub fn is_quorum_reached(&self, chain: &str, height: u64, state_root: &str) -> bool {
        match self.attestations.read() {
            Ok(guard) => {
                let key = (chain.to_string(), height);
                if let Some(list) = guard.get(&key) {
                    let matching_count = list.iter().filter(|a| a.state_root == state_root).count();
                    matching_count >= self.quorum_threshold
                } else {
                    false
                }
            }
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire read lock on attestations: {}", e);
                false
            }
        }
    }

    pub fn get_attestations(&self, chain: &str, height: u64) -> Vec<StateAttestation> {
        match self.attestations.read() {
            Ok(guard) => {
                let key = (chain.to_string(), height);
                guard.get(&key).cloned().unwrap_or_default()
            }
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire read lock on attestations: {}", e);
                Vec::new()
            }
        }
    }

    pub fn get_validator_count(&self) -> usize {
        match self.known_validators.read() {
            Ok(validators) => validators.len(),
            Err(_) => 0,
        }
    }

    pub fn get_slashing_proofs(&self) -> Vec<EquivocationProof> {
        match self.slashing_proofs.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                tracing::error!("[Consensus] Failed to acquire read lock on slashing_proofs: {}", e);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify_attestation() {
        let mgr = ConsensusManager::new(1);
        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

        // F1.3: Register the validator before adding attestation (fail-closed requires it)
        mgr.register_validator(&hex::encode(pk.serialize()));

        let attestation = mgr
            .sign_state_root(&sk, "JKC", 100, "hash_abc", "state_root_123")
            .expect("Signing should succeed");

        assert_eq!(mgr.verify_attestation(&attestation), true);
        assert_eq!(mgr.add_attestation(attestation), true);
        assert_eq!(mgr.is_quorum_reached("JKC", 100, "state_root_123"), true);
        assert_eq!(mgr.is_quorum_reached("JKC", 100, "wrong_root"), false);
    }

    #[test]
    fn test_equivocation_detection_and_slashing_proof() {
        let mgr = ConsensusManager::new(1);
        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

        // F1.3: Register the validator (fail-closed requires it)
        mgr.register_validator(&hex::encode(pk.serialize()));

        // Sign first valid root
        let att1 = mgr
            .sign_state_root(&sk, "DOGE", 500, "hash_500", "root_canonical")
            .unwrap();
        assert_eq!(mgr.add_attestation(att1), true);

        // Sign conflicting root on the same height (double-signing / equivocation)
        let att2 = mgr
            .sign_state_root(&sk, "DOGE", 500, "hash_500", "root_malicious_fork")
            .unwrap();
        assert_eq!(mgr.add_attestation(att2), false);

        // Verify slashing proof was captured
        let proofs = mgr.get_slashing_proofs();
        assert_eq!(proofs.len(), 1);
        let proof = &proofs[0];
        assert_eq!(proof.chain, "DOGE");
        assert_eq!(proof.block_height, 500);
        assert_eq!(proof.first_attestation.state_root, "root_canonical");
        assert_eq!(proof.second_attestation.state_root, "root_malicious_fork");

        // Verify cryptographic validity of the proof
        assert!(crate::consensus::covenants::verify_equivocation_proof(proof));
    }

    #[test]
    fn test_fail_closed_no_validators_rejects_attestation() {
        // F1.3: With zero registered validators, ALL attestations must be rejected
        let mgr = ConsensusManager::new(3);
        let secp = Secp256k1::new();
        let (sk, _pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

        // Do NOT register any validator — empty set
        let attestation = mgr
            .sign_state_root(&sk, "JKC", 100, "hash_abc", "root_xyz")
            .expect("Signing should succeed");

        // Despite valid signature, must be rejected because no validators registered
        let added = mgr.add_attestation(attestation);
        assert!(!added, "add_attestation must fail when no validators are registered (fail-closed)");
    }
}
