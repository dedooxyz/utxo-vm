use serde::{Deserialize, Serialize};

use crate::consensus::ConsensusManager;
use crate::storage::SparseMerkleTree;
use crate::types::StateAttestation;

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

    /// Verify quorum (≥2/3 validator attestations for same state root)
    fn verify_quorum(&self, proof: &CrossChainProof) -> bool {
        if proof.attestations.is_empty() {
            return false;
        }

        // Group attestations by state_root
        let mut root_counts = std::collections::HashMap::new();
        for att in &proof.attestations {
            *root_counts.entry(&att.state_root).or_insert(0) += 1;
        }

        // Find the state_root with most attestations
        let max_count = root_counts.values().max().unwrap_or(&0);
        let total_validators = self.consensus.get_validator_count();

        // Check if ≥2/3 quorum
        let required = (total_validators * 2 + 2) / 3; // Ceiling division
        *max_count >= required
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

        // Parse the object data as value
        let value = {
            use sha2::{Digest, Sha256};
            let data_str = proof.object_data.to_string();
            let hash = Sha256::digest(data_str.as_bytes());
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
}
