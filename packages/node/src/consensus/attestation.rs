use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use anyhow::Result;
use secp256k1::ecdsa::Signature;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use crate::types::{EquivocationProof, QuorumDivergenceProof, QuorumResult, StateAttestation};

/// Compute a Merkle root over a list of attestations.
/// Reuses the same SHA-256 pairwise hashing pattern as the SMT in storage/smt.rs.
/// For an empty list, returns the zero hash. For a single attestation, returns
/// the hash of that attestation's canonical serialization. For odd-length lists,
/// the last element is hashed with itself (standard Merkle behavior).
fn compute_attestation_merkle_root(attestations: &[StateAttestation]) -> String {
    if attestations.is_empty() {
        return hex::encode([0u8; 32]);
    }

    // Sort attestations by validator_pubkey for deterministic ordering.
    // Insertion order is not guaranteed to be the same across nodes.
    let mut sorted: Vec<&StateAttestation> = attestations.iter().collect();
    sorted.sort_by(|a, b| a.validator_pubkey.cmp(&b.validator_pubkey));

    // Leaf hash: SHA256("ATTESTATION_LEAF" || canonical_attestation_bytes)
    let leaf_hashes: Vec<[u8; 32]> = sorted
        .iter()
        .map(|a| {
            let mut hasher = Sha256::new();
            hasher.update(b"ATTESTATION_LEAF");
            hasher.update(a.chain.as_bytes());
            hasher.update(&a.block_height.to_be_bytes());
            hasher.update(a.block_hash.as_bytes());
            hasher.update(a.state_root.as_bytes());
            hasher.update(a.validator_pubkey.as_bytes());
            hasher.update(a.signature_hex.as_bytes());
            // timestamp is metadata — excluded from the Merkle leaf hash for determinism
            let result = hasher.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&result);
            out
        })
        .collect();

    let mut level = leaf_hashes;
    while level.len() > 1 {
        let mut next_level = Vec::with_capacity((level.len() + 1) / 2);
        for chunk in level.chunks(2) {
            if chunk.len() == 2 {
                let mut hasher = Sha256::new();
                hasher.update(&chunk[0]);
                hasher.update(&chunk[1]);
                let result = hasher.finalize();
                let mut out = [0u8; 32];
                out.copy_from_slice(&result);
                next_level.push(out);
            } else {
                // Odd leaf: hash with itself
                let mut hasher = Sha256::new();
                hasher.update(&chunk[0]);
                hasher.update(&chunk[0]);
                let result = hasher.finalize();
                let mut out = [0u8; 32];
                out.copy_from_slice(&result);
                next_level.push(out);
            }
        }
        level = next_level;
    }

    hex::encode(level[0])
}

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

    /// Build the quorum-winning result for a (chain, height) pair.
    ///
    /// Extracts the state_root with the most attestations; if that count meets
    /// the quorum threshold, returns a QuorumResult containing the winning root,
    /// the supporting attestations, and a Merkle root over them.
    ///
    /// Returns None if no attestation list exists for the key, or if no
    /// state_root reaches the quorum threshold.
    pub fn build_quorum_result(&self, chain: &str, height: u64) -> Option<QuorumResult> {
        let attestations = self.get_attestations(chain, height);
        if attestations.is_empty() {
            return None;
        }

        // Group attestations by state_root, pick the one with the most support.
        // Ties are broken by lexicographic order of state_root for determinism.
        let mut root_groups: HashMap<String, Vec<StateAttestation>> = HashMap::new();
        for att in &attestations {
            root_groups
                .entry(att.state_root.clone())
                .or_default()
                .push(att.clone());
        }

        // Find the root with the most support (ties broken by lexicographic order)
        let winning = root_groups
            .into_iter()
            .max_by(|(root_a, group_a), (root_b, group_b)| {
                group_a
                    .len()
                    .cmp(&group_b.len())
                    .then_with(|| root_a.cmp(root_b))
            })
            .expect("non-empty map");

        let (winning_root, supporting) = winning;

        if supporting.len() < self.quorum_threshold {
            return None;
        }

        // Use the block_hash from the first supporting attestation (all supporting
        // attestations for the same state_root should agree on block_hash).
        let block_hash = supporting
            .first()
            .map(|a| a.block_hash.clone())
            .unwrap_or_default();

        let merkle_root = compute_attestation_merkle_root(&supporting);

        Some(QuorumResult {
            chain: chain.to_string(),
            block_height: height,
            state_root: winning_root,
            block_hash,
            supporting_attestations: supporting,
            merkle_root,
            support_count: attestations.len(), // total attestations for this key
            quorum_threshold: self.quorum_threshold,
        })
    }

    /// Detect quorum divergence: given a node's own attestation and the quorum
    /// result, produce a QuorumDivergenceProof if the operator's state_root
    /// differs from the quorum-winning root.
    ///
    /// Returns None if the operator's root matches the quorum root, or if the
    /// quorum result is None (no quorum reached).
    pub fn detect_divergence(
        &self,
        operator_attestation: &StateAttestation,
        quorum_result: &QuorumResult,
    ) -> Option<QuorumDivergenceProof> {
        if operator_attestation.state_root == quorum_result.state_root {
            return None;
        }

        Some(QuorumDivergenceProof {
            chain: operator_attestation.chain.clone(),
            block_height: operator_attestation.block_height,
            operator_attestation: operator_attestation.clone(),
            quorum_result_root: quorum_result.state_root.clone(),
            supporting_attestations: quorum_result.supporting_attestations.clone(),
            merkle_root: quorum_result.merkle_root.clone(),
            detected_at: chrono::Utc::now().timestamp(),
        })
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

    #[test]
    fn test_build_quorum_result_and_divergence_detection() {
        // Simulate 5 validators: 4 agree on root A, 1 signs root B.
        // Quorum threshold = 4 (so root A reaches quorum, root B does not).
        let mgr = ConsensusManager::new(4);
        let secp = Secp256k1::new();

        // Generate 5 keypairs
        let mut keys = Vec::new();
        for _ in 0..5 {
            let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
            mgr.register_validator(&hex::encode(pk.serialize()));
            keys.push((sk, pk));
        }

        let chain = "JKC";
        let height = 1000u64;
        let block_hash = "block_hash_1000";
        let root_a = "root_a_canonical";
        let root_b = "root_b_divergent";

        // Validators 1-4 sign root A (the quorum-winning root)
        for (sk, _) in &keys[..4] {
            let att = mgr
                .sign_state_root(sk, chain, height, block_hash, root_a)
                .unwrap();
            assert!(mgr.add_attestation(att), "Valid attestation for root A must be accepted");
        }

        // Validator 5 signs root B (divergent)
        let divergent_att = mgr
            .sign_state_root(&keys[4].0, chain, height, block_hash, root_b)
            .unwrap();
        assert!(mgr.add_attestation(divergent_att.clone()), "Valid attestation for root B must be accepted");

        // Build quorum result — should return root A with 4 supporting attestations
        let quorum = mgr.build_quorum_result(chain, height);
        assert!(quorum.is_some(), "Quorum result must exist (4 attestations for root A >= threshold 4)");
        let quorum = quorum.unwrap();

        assert_eq!(quorum.state_root, root_a, "Quorum-winning root must be root A");
        assert_eq!(quorum.supporting_attestations.len(), 4,
            "Must have 4 supporting attestations for root A");
        assert_eq!(quorum.quorum_threshold, 4);
        assert!(!quorum.merkle_root.is_empty(), "Merkle root must be non-empty");

        // Verify all supporting attestations are for root A
        for att in &quorum.supporting_attestations {
            assert_eq!(att.state_root, root_a,
                "All supporting attestations must be for root A");
        }

        // Detect divergence: validator 5's attestation (root B) vs quorum (root A)
        let divergence = mgr.detect_divergence(&divergent_att, &quorum);
        assert!(divergence.is_some(), "Divergence must be detected for validator 5 (root B != root A)");
        let proof = divergence.unwrap();

        assert_eq!(proof.operator_attestation.state_root, root_b,
            "Proof must contain the divergent operator's attestation (root B)");
        assert_eq!(proof.quorum_result_root, root_a,
            "Proof must contain the quorum-winning root (root A)");
        assert_eq!(proof.supporting_attestations.len(), 4,
            "Proof must contain 4 supporting attestations");
        assert_eq!(proof.merkle_root, quorum.merkle_root,
            "Proof merkle root must match quorum merkle root");

        // Verify no false positive: a validator that signed root A should NOT be flagged
        let honest_att = mgr
            .sign_state_root(&keys[0].0, chain, height, block_hash, root_a)
            .unwrap();
        // This attestation is a duplicate (same validator, same root) — add_attestation
        // will reject it, but we can still test detect_divergence directly.
        let no_divergence = mgr.detect_divergence(&honest_att, &quorum);
        assert!(no_divergence.is_none(),
            "No divergence should be detected for a validator that signed the quorum root");
    }

    #[test]
    fn test_build_quorum_result_no_quorum_returns_none() {
        // 3 validators, threshold 4 — no quorum possible
        let mgr = ConsensusManager::new(4);
        let secp = Secp256k1::new();

        for _ in 0..3 {
            let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
            mgr.register_validator(&hex::encode(pk.serialize()));
            let att = mgr.sign_state_root(&sk, "JKC", 200, "hash", "root").unwrap();
            mgr.add_attestation(att);
        }

        // Only 3 attestations, threshold 4 — no quorum
        let result = mgr.build_quorum_result("JKC", 200);
        assert!(result.is_none(), "build_quorum_result must return None when quorum is not reached");
    }

    #[test]
    fn test_merkle_root_deterministic() {
        // The Merkle root over the same set of attestations must be deterministic
        // regardless of insertion order (since we sort by state_root, not insertion order).
        let mgr1 = ConsensusManager::new(2);
        let mgr2 = ConsensusManager::new(2);
        let secp = Secp256k1::new();

        let (sk1, pk1) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        let (sk2, pk2) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        mgr1.register_validator(&hex::encode(pk1.serialize()));
        mgr1.register_validator(&hex::encode(pk2.serialize()));
        mgr2.register_validator(&hex::encode(pk1.serialize()));
        mgr2.register_validator(&hex::encode(pk2.serialize()));

        // Both validators sign the same root in both managers
        let att1 = mgr1.sign_state_root(&sk1, "JKC", 300, "hash", "root_x").unwrap();
        let att2 = mgr1.sign_state_root(&sk2, "JKC", 300, "hash", "root_x").unwrap();
        mgr1.add_attestation(att1.clone());
        mgr1.add_attestation(att2.clone());

        // Add in reverse order to mgr2
        mgr2.add_attestation(att2);
        mgr2.add_attestation(att1);

        let q1 = mgr1.build_quorum_result("JKC", 300).unwrap();
        let q2 = mgr2.build_quorum_result("JKC", 300).unwrap();

        assert_eq!(q1.merkle_root, q2.merkle_root,
            "Merkle root must be deterministic regardless of attestation insertion order");
    }
}
