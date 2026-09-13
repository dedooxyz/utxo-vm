use serde::{Deserialize, Serialize};

use crate::consensus::ConsensusManager;
use crate::storage::SparseMerkleTree;
use crate::types::StateAttestation;

/// Recursively sort object keys in a `serde_json::Value` so that serialization
/// is canonical (independent of insertion order). This protects the Merkle
/// leaf hash from depending on whether `serde_json`'s `preserve_order` Cargo
/// feature is active anywhere in the dependency graph (Cargo unifies features
/// project-wide). Without this, the same logical object data could hash
/// differently depending on incidental construction order.
fn canonicalize_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            // Collect and sort keys, then recursively canonicalize each value.
            let mut sorted: Vec<(String, serde_json::Value)> = map
                .iter()
                .map(|(k, v)| (k.to_string(), canonicalize_json_value(v)))
                .collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let mut obj = serde_json::Map::new();
            for (k, v) in sorted {
                obj.insert(k, v);
            }
            serde_json::Value::Object(obj)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json_value).collect())
        }
        other => other.clone(),
    }
}

/// Serialize a `serde_json::Value` with canonically sorted object keys,
/// producing a deterministic byte encoding suitable for hashing.
fn canonical_json_to_vec(value: &serde_json::Value) -> Vec<u8> {
    let canonical = canonicalize_json_value(value);
    serde_json::to_vec(&canonical).unwrap_or_default()
}

/// Cross-chain proof from a source chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossChainProof {
    pub source_chain: String,
    pub source_height: u64,
    pub source_root: String,
    pub object_id: String,
    pub object_data: serde_json::Value,
    pub merkle_proof: crate::types::SmtInclusionProof,
    pub attestations: Vec<StateAttestation>,
}

/// Result of cross-chain proof verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub valid: bool,
    pub source_chain: String,
    pub source_height: u64,
    pub quorum_reached: bool,
    pub merkle_verified: bool,
    pub error: Option<String>,
}

/// Cross-chain proof verifier
pub struct CrossChainVerifier {
    consensus: std::sync::Arc<ConsensusManager>,
}

impl CrossChainVerifier {
    pub fn new(consensus: std::sync::Arc<ConsensusManager>) -> Self {
        Self { consensus }
    }

    /// Verify a cross-chain proof from another chain
    pub fn verify_proof(&self, proof: &CrossChainProof) -> VerificationResult {
        // Step 1: Verify quorum (≥2/3 validator attestations)
        let quorum_reached = self.verify_quorum(proof);

        if !quorum_reached {
            return VerificationResult {
                valid: false,
                source_chain: proof.source_chain.clone(),
                source_height: proof.source_height,
                quorum_reached: false,
                merkle_verified: false,
                error: Some("Quorum not reached: insufficient validator attestations".to_string()),
            };
        }

        // Step 2: Verify Merkle proof
        let merkle_verified = self.verify_merkle(proof);

        if !merkle_verified {
            return VerificationResult {
                valid: false,
                source_chain: proof.source_chain.clone(),
                source_height: proof.source_height,
                quorum_reached: true,
                merkle_verified: false,
                error: Some("Merkle proof verification failed".to_string()),
            };
        }

        // Step 3: Verify attestation signatures
        let signatures_valid = self.verify_attestation_signatures(proof);

        if !signatures_valid {
            return VerificationResult {
                valid: false,
                source_chain: proof.source_chain.clone(),
                source_height: proof.source_height,
                quorum_reached: true,
                merkle_verified: true,
                error: Some("Invalid attestation signatures".to_string()),
            };
        }

        // All checks passed
        VerificationResult {
            valid: true,
            source_chain: proof.source_chain.clone(),
            source_height: proof.source_height,
            quorum_reached: true,
            merkle_verified: true,
            error: None,
        }
    }

    /// Verify quorum (≥2/3 validator attestations for same state root).
    /// F1.3: Fail-closed — if no validators are registered, quorum is never reached.
    /// Security: dedup by validator_pubkey to prevent counting duplicates.
    /// Security: only count attestations from known validators.
    fn verify_quorum(&self, proof: &CrossChainProof) -> bool {
        if proof.attestations.is_empty() {
            return false;
        }

        let total_validators = self.consensus.get_validator_count();

        // F1.3: Fail-closed — zero validators means no trust anchor, reject always
        if total_validators == 0 {
            tracing::warn!(
                "[CrossChain] Quorum check failed-closed: no validators registered"
            );
            return false;
        }

        // Get the set of known validators for membership check
        let known_validators = match self.consensus.known_validators.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                tracing::error!("[CrossChain] Failed to read validator set: {}", e);
                return false;
            }
        };

        // Filter: only count attestations from known validators, dedup by pubkey
        let mut seen_pubkeys = std::collections::HashSet::new();
        let mut root_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

        for att in &proof.attestations {
            // Must be from a known validator
            if !known_validators.contains(&att.validator_pubkey) {
                tracing::warn!(
                    "[CrossChain] Skipping attestation from unknown validator: {}",
                    att.validator_pubkey
                );
                continue;
            }
            // Dedup: one attestation per validator per (chain, height)
            if !seen_pubkeys.insert(att.validator_pubkey.clone()) {
                tracing::warn!(
                    "[CrossChain] Skipping duplicate attestation from validator: {}",
                    att.validator_pubkey
                );
                continue;
            }
            *root_counts.entry(att.state_root.as_str()).or_insert(0) += 1;
        }

        if root_counts.is_empty() {
            return false;
        }

        // Find the state_root with most attestations
        let (winning_root, max_count) = root_counts
            .into_iter()
            .max_by(|(root_a, count_a), (root_b, count_b)| {
                count_a.cmp(count_b).then_with(|| root_a.cmp(root_b))
            })
            .unwrap();

        // The winning root must match the proof's source_root
        if winning_root != proof.source_root {
            tracing::warn!(
                "[CrossChain] Quorum root mismatch: winning={} but proof.source_root={}",
                winning_root, proof.source_root
            );
            return false;
        }

        // Check if ≥2/3 quorum
        let required = (total_validators * 2 + 2) / 3; // Ceiling division
        max_count >= required
    }

    /// Verify Merkle inclusion proof
    fn verify_merkle(&self, proof: &CrossChainProof) -> bool {
        // Parse the root
        let root_bytes = match hex::decode(&proof.source_root) {
            Ok(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                arr
            }
            _ => return false,
        };

        // Parse the object_id as key
        let key = {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(proof.object_id.as_bytes());
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&hash);
            arr
        };

        // Parse the object data as value (canonical serialization with sorted keys)
        let value = {
            use sha2::{Digest, Sha256};
            let data_bytes = canonical_json_to_vec(&proof.object_data);
            let hash = Sha256::digest(&data_bytes);
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&hash);
            arr
        };

        // Verify the proof
        SparseMerkleTree::verify_proof(&root_bytes, &key, &value, &proof.merkle_proof.proof_path)
    }

    /// Verify attestation signatures are valid via ECDSA (secp256k1).
    /// Each attestation must:
    ///   1. Match source_chain, source_height, source_root from the proof
    ///   2. Have a cryptographically valid ECDSA signature (via ConsensusManager)
    fn verify_attestation_signatures(&self, proof: &CrossChainProof) -> bool {
        if proof.attestations.is_empty() {
            return false;
        }
        proof.attestations.iter().all(|att| {
            // Field consistency: attestation must reference the correct chain/height/root
            if att.state_root != proof.source_root
                || att.block_height != proof.source_height
                || att.chain != proof.source_chain
            {
                tracing::warn!(
                    "[CrossChain] Attestation field mismatch: chain={} height={} root={}",
                    att.chain, att.block_height, att.state_root
                );
                return false;
            }
            // Cryptographic ECDSA verification via ConsensusManager
            let ok = self.consensus.verify_attestation(att);
            if !ok {
                tracing::warn!(
                    "[CrossChain] Invalid ECDSA signature from validator {}",
                    att.validator_pubkey
                );
            }
            ok
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_proof_fails() {
        let consensus = std::sync::Arc::new(ConsensusManager::new(3));
        let verifier = CrossChainVerifier::new(consensus);

        let proof = CrossChainProof {
            source_chain: "JKC".to_string(),
            source_height: 100,
            source_root: "abc".to_string(),
            object_id: "obj_1".to_string(),
            object_data: serde_json::json!({}),
            merkle_proof: crate::types::SmtInclusionProof {
                key_hex: "aa".to_string(),
                value_hex: "bb".to_string(),
                root_hex: "cc".to_string(),
                proof_path: vec![],
                verified: false,
            },
            attestations: vec![],
        };

        let result = verifier.verify_proof(&proof);
        assert!(!result.valid);
        assert!(!result.quorum_reached);
    }

    #[test]
    fn test_merkle_leaf_hash_independent_of_key_insertion_order() {
        // Two serde_json::Value::Objects with the same key/value pairs but
        // inserted in different orders must produce identical Merkle leaf
        // hashes. This locks in the canonicalization invariant: the hash
        // must not depend on whether serde_json's `preserve_order` Cargo
        // feature is active anywhere in the dependency graph.
        use sha2::{Digest, Sha256};

        // Build object A: keys inserted in alphabetical order
        let mut map_a = serde_json::Map::new();
        map_a.insert("z".to_string(), serde_json::json!(1));
        map_a.insert("a".to_string(), serde_json::json!(2));
        map_a.insert("m".to_string(), serde_json::json!(3));
        let obj_a = serde_json::Value::Object(map_a);

        // Build object B: same pairs, reverse insertion order
        let mut map_b = serde_json::Map::new();
        map_b.insert("m".to_string(), serde_json::json!(3));
        map_b.insert("a".to_string(), serde_json::json!(2));
        map_b.insert("z".to_string(), serde_json::json!(1));
        let obj_b = serde_json::Value::Object(map_b);

        let hash_a = Sha256::digest(&canonical_json_to_vec(&obj_a));
        let hash_b = Sha256::digest(&canonical_json_to_vec(&obj_b));
        assert_eq!(
            hash_a, hash_b,
            "Merkle leaf hash must be identical regardless of key insertion order"
        );

        // Also verify nested objects are canonicalized recursively
        let nested_a = serde_json::json!({"outer": {"b": 1, "a": 2}, "x": 0});
        let nested_b = serde_json::json!({"x": 0, "outer": {"a": 2, "b": 1}});
        let hash_na = Sha256::digest(&canonical_json_to_vec(&nested_a));
        let hash_nb = Sha256::digest(&canonical_json_to_vec(&nested_b));
        assert_eq!(
            hash_na, hash_nb,
            "Nested object key order must not affect Merkle leaf hash"
        );

        // Different key/value content must still produce different hashes
        let different = serde_json::json!({"a": 2, "z": 99, "m": 3});
        let hash_diff = Sha256::digest(&canonical_json_to_vec(&different));
        assert_ne!(
            hash_a, hash_diff,
            "Different content must produce different Merkle leaf hashes"
        );
    }
}
