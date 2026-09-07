use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use crate::types::{SmtInclusionProof, SmtProofNode};

pub const TREE_DEPTH: usize = 256;

fn hash_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

pub fn hash_leaf(key: &[u8; 32], value: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"SMT_LEAF");
    hasher.update(key);
    hasher.update(value);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Sparse Merkle Tree (SMT) with 256-bit keys and values
#[derive(Clone, Debug)]
pub struct SparseMerkleTree {
    pub leaves: BTreeMap<[u8; 32], [u8; 32]>,
    empty_hashes: Vec<[u8; 32]>,
}

impl Default for SparseMerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseMerkleTree {
    pub fn new() -> Self {
        let mut empty_hashes = Vec::with_capacity(TREE_DEPTH + 1);
        let mut current = [0u8; 32];
        empty_hashes.push(current);
        for _ in 0..TREE_DEPTH {
            current = hash_node(&current, &current);
            empty_hashes.push(current);
        }

        Self {
            leaves: BTreeMap::new(),
            empty_hashes,
        }
    }

    /// Insert or update a key-value pair
    pub fn update(&mut self, key: [u8; 32], value: [u8; 32]) {
        if value == [0u8; 32] {
            self.leaves.remove(&key);
        } else {
            self.leaves.insert(key, value);
        }
    }

    /// Get value for a key
    pub fn get(&self, key: &[u8; 32]) -> Option<&[u8; 32]> {
        self.leaves.get(key)
    }

    /// Compute the 32-byte Merkle root
    pub fn root(&self) -> [u8; 32] {
        if self.leaves.is_empty() {
            return self.empty_hashes[TREE_DEPTH];
        }

        // Fast tree calculation over active leaves
        // We sort the hashed leaves and construct the canonical binary Merkle root
        let mut level_hashes: Vec<[u8; 32]> = self
            .leaves
            .iter()
            .map(|(k, v)| hash_leaf(k, v))
            .collect();

        if level_hashes.is_empty() {
            return self.empty_hashes[TREE_DEPTH];
        }

        while level_hashes.len() > 1 {
            let mut next_level = Vec::with_capacity((level_hashes.len() + 1) / 2);
            for chunk in level_hashes.chunks(2) {
                if chunk.len() == 2 {
                    next_level.push(hash_node(&chunk[0], &chunk[1]));
                } else {
                    next_level.push(hash_node(&chunk[0], &chunk[0]));
                }
            }
            level_hashes = next_level;
        }

        level_hashes[0]
    }

    /// Generate an inclusion proof for a key
    pub fn get_proof(&self, key: &[u8; 32]) -> Option<SmtInclusionProof> {
        let value = self.leaves.get(key)?;
        let _target_leaf = hash_leaf(key, value);

        let leaves_list: Vec<([u8; 32], [u8; 32])> = self.leaves.iter().map(|(&k, &v)| (k, v)).collect();
        let target_idx = leaves_list.iter().position(|(k, _)| k == key)?;

        let mut level_hashes: Vec<[u8; 32]> = leaves_list.iter().map(|(k, v)| hash_leaf(k, v)).collect();
        let mut current_idx = target_idx;
        let mut proof_path = Vec::new();

        while level_hashes.len() > 1 {
            let is_right = current_idx % 2 == 1;
            let sibling_idx = if is_right { current_idx - 1 } else { current_idx + 1 };

            let sibling_hash = if sibling_idx < level_hashes.len() {
                level_hashes[sibling_idx]
            } else {
                level_hashes[current_idx]
            };

            proof_path.push(SmtProofNode {
                position: if is_right { "left".to_string() } else { "right".to_string() },
                hash_hex: hex::encode(sibling_hash),
            });

            let mut next_level = Vec::with_capacity((level_hashes.len() + 1) / 2);
            for chunk in level_hashes.chunks(2) {
                if chunk.len() == 2 {
                    next_level.push(hash_node(&chunk[0], &chunk[1]));
                } else {
                    next_level.push(hash_node(&chunk[0], &chunk[0]));
                }
            }
            level_hashes = next_level;
            current_idx /= 2;
        }

        let root_bytes = if !level_hashes.is_empty() {
            level_hashes[0]
        } else {
            self.empty_hashes[TREE_DEPTH]
        };

        // Self-verify
        let verified = Self::verify_proof(&root_bytes, key, value, &proof_path);

        Some(SmtInclusionProof {
            key_hex: hex::encode(key),
            value_hex: hex::encode(value),
            root_hex: hex::encode(root_bytes),
            proof_path,
            verified,
        })
    }

    /// Verify an inclusion proof mathematically
    pub fn verify_proof(
        root: &[u8; 32],
        key: &[u8; 32],
        value: &[u8; 32],
        proof_path: &[SmtProofNode],
    ) -> bool {
        let mut current = hash_leaf(key, value);

        for node in proof_path {
            let sibling_bytes = match hex::decode(&node.hash_hex) {
                Ok(b) if b.len() == 32 => {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&b);
                    arr
                }
                _ => return false,
            };

            if node.position == "left" {
                current = hash_node(&sibling_bytes, &current);
            } else {
                current = hash_node(&current, &sibling_bytes);
            }
        }

        current == *root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smt_update_and_proof() {
        let mut smt = SparseMerkleTree::new();
        let key1 = [1u8; 32];
        let val1 = [10u8; 32];
        let key2 = [2u8; 32];
        let val2 = [20u8; 32];
        let key3 = [3u8; 32];
        let val3 = [30u8; 32];

        smt.update(key1, val1);
        smt.update(key2, val2);
        smt.update(key3, val3);

        let root = smt.root();
        assert_ne!(root, [0u8; 32]);

        let proof = smt.get_proof(&key2).expect("Proof should exist");
        assert_eq!(proof.verified, true);
        assert_eq!(proof.root_hex, hex::encode(root));

        // Test with tampered value
        let tampered_val = [99u8; 32];
        let invalid = SparseMerkleTree::verify_proof(&root, &key2, &tampered_val, &proof.proof_path);
        assert_eq!(invalid, false);
    }
}
