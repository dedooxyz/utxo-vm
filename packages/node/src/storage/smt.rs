use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use crate::types::{SmtInclusionProof, SmtProofNode};

pub const TREE_DEPTH: usize = 256;

fn hash_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"SMT_NODE");
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

/// Sorted-leaves Merkle tree used as the state commitment structure.
///
/// NOTE: Despite the name `SparseMerkleTree`, this is NOT a true 256-bit Sparse Merkle
/// Tree with fixed-position leaves. It is a sorted-leaves Merkle tree where leaves are
/// placed in sorted key order and hashed pairwise bottom-up. The root is a function of
/// the sorted key-value pairs. This provides deterministic state commitments but the
/// proof format is non-standard compared to true SMT implementations.
///
/// The root is cached and only recomputed when leaves change.
#[derive(Clone, Debug)]
pub struct SparseMerkleTree {
    pub leaves: BTreeMap<[u8; 32], [u8; 32]>,
    empty_hashes: Vec<[u8; 32]>,
    cached_root: [u8; 32],
    dirty: bool,
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
            empty_hashes: empty_hashes.clone(),
            cached_root: empty_hashes[TREE_DEPTH],
            dirty: false,
        }
    }

    /// Insert or update a key-value pair
    pub fn update(&mut self, key: [u8; 32], value: [u8; 32]) {
        if value == [0u8; 32] {
            self.leaves.remove(&key);
        } else {
            self.leaves.insert(key, value);
        }
        self.dirty = true;
    }

    /// Get value for a key
    pub fn get(&self, key: &[u8; 32]) -> Option<&[u8; 32]> {
        self.leaves.get(key)
    }

    fn compute_root(&self) -> [u8; 32] {
        if self.leaves.is_empty() {
            return self.empty_hashes[TREE_DEPTH];
        }

        let mut level_hashes: Vec<[u8; 32]> = self
            .leaves
            .iter()
            .map(|(k, v)| hash_leaf(k, v))
            .collect();

        while level_hashes.len() > 1 {
            let mut next_level = Vec::with_capacity((level_hashes.len() + 1) / 2);
            for chunk in level_hashes.chunks(2) {
                if chunk.len() == 2 {
                    next_level.push(hash_node(&chunk[0], &chunk[1]));
                } else {
                    // Odd leaf: hash with itself (standard Merkle behavior)
                    next_level.push(hash_node(&chunk[0], &chunk[0]));
                }
            }
            level_hashes = next_level;
        }

        level_hashes[0]
    }

    /// Compute the 32-byte Merkle root (cached, O(1) if no changes)
    pub fn root(&mut self) -> [u8; 32] {
        if self.dirty {
            self.cached_root = self.compute_root();
            self.dirty = false;
        }
        self.cached_root
    }

    /// Generate an inclusion proof for a key
    pub fn get_proof(&self, key: &[u8; 32]) -> Option<SmtInclusionProof> {
        let value = self.leaves.get(key)?;

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
        let verified = Self::verify_proof_static(&root_bytes, key, value, &proof_path);

        Some(SmtInclusionProof {
            key_hex: hex::encode(key),
            value_hex: hex::encode(value),
            root_hex: hex::encode(root_bytes),
            proof_path,
            verified,
        })
    }

    /// Verify an inclusion proof mathematically (static, no tree access needed)
    pub fn verify_proof_static(
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

    /// Verify an inclusion proof mathematically (compatible with existing API)
    pub fn verify_proof(
        root: &[u8; 32],
        key: &[u8; 32],
        value: &[u8; 32],
        proof_path: &[SmtProofNode],
    ) -> bool {
        Self::verify_proof_static(root, key, value, proof_path)
    }

    /// Get the number of leaves in the tree
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Check if the tree is empty
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
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

    #[test]
    fn test_smt_root_caching() {
        let mut smt = SparseMerkleTree::new();
        let key1 = [1u8; 32];
        let val1 = [10u8; 32];

        // Initial root should be empty
        let root1 = smt.root();
        assert_eq!(root1, smt.empty_hashes[TREE_DEPTH]);

        // Update and get root
        smt.update(key1, val1);
        let root2 = smt.root();
        assert_ne!(root2, root1);

        // Same root without changes (cached)
        let root3 = smt.root();
        assert_eq!(root2, root3);
        assert!(!smt.dirty);

        // Update again
        smt.update(key1, [99u8; 32]);
        let root4 = smt.root();
        assert_ne!(root4, root2);
    }

    #[test]
    fn test_smt_len() {
        let mut smt = SparseMerkleTree::new();
        assert_eq!(smt.len(), 0);
        assert!(smt.is_empty());

        smt.update([1u8; 32], [10u8; 32]);
        assert_eq!(smt.len(), 1);
        assert!(!smt.is_empty());
    }
}
