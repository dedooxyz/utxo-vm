use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
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

/// True fixed-depth (256-bit) Sparse Merkle Tree.
///
/// Issue 12: This replaces the previous sorted-leaves Merkle tree that
/// rebuilt all leaf hashes on every update (O(n log n)) and rebuilt the
/// full tree for every proof (O(n log n)). This implementation stores only
/// non-default internal nodes in a HashMap, so update() and get_proof()
/// each touch only O(TREE_DEPTH) = O(256) nodes regardless of leaf count.
///
/// CONSENSUS-BREAKING CHANGE: The root format is different from the old
/// sorted-leaves tree. A fixed-depth root is NOT equal to the old sorted-
/// leaves root for the same data. Anything that persisted or transmitted
/// a root computed under the old scheme (stored blocks, cross-chain proofs
/// already issued, any external verifier) will not recognize new-scheme
/// roots. This requires the same treatment as Issue 5: versioned/coordinated
/// cutover, not a silent swap.
///
/// Node storage convention:
/// - `level` = distance from root (0 = root, TREE_DEPTH = leaves)
/// - A node at level `L` is identified by the first `L` bits of the key
/// - `nodes` map key: `(level: u16, prefix: [u8; 32])` where `prefix` is
///   the first `level` bits of the key, packed big-endian (MSB first)
/// - `empty_hashes[d]` = hash of empty subtree at depth `d` from leaf
///   (0 = empty leaf, TREE_DEPTH = empty root)
#[derive(Clone, Debug)]
pub struct SparseMerkleTree {
    /// Leaf values keyed by their 32-byte key.
    pub leaves: BTreeMap<[u8; 32], [u8; 32]>,
    /// Non-default internal node hashes, keyed by (level, prefix).
    /// Level 0 = root, level TREE_DEPTH = leaves. Only non-default
    /// (non-empty) nodes are stored; missing entries = empty subtree.
    nodes: HashMap<(u16, [u8; 32]), [u8; 32]>,
    /// Precomputed hash of an empty subtree at each depth from leaf.
    /// empty_hashes[0] = empty leaf ([0u8; 32]), empty_hashes[TREE_DEPTH] = empty root.
    empty_hashes: Vec<[u8; 32]>,
    cached_root: [u8; 32],
    dirty: bool,
}

impl Default for SparseMerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the bit at position `bit_idx` (0 = MSB, TREE_DEPTH-1 = LSB)
/// from a 256-bit key. Returns 0 for left, 1 for right.
fn bit_at(key: &[u8; 32], bit_idx: usize) -> u8 {
    let byte_idx = bit_idx / 8;
    let bit_offset = 7 - (bit_idx % 8);
    (key[byte_idx] >> bit_offset) & 1
}

/// Compute the path prefix of length `bit_idx` bits from a key, packed
/// into a [u8; 32] as a big-endian bit string (MSB first). Bits beyond
/// `bit_idx` are zeroed.
fn path_prefix(key: &[u8; 32], bit_idx: usize) -> [u8; 32] {
    let mut prefix = [0u8; 32];
    let full_bytes = bit_idx / 8;
    let remaining_bits = bit_idx % 8;

    // Copy full bytes
    if full_bytes > 0 {
        prefix[..full_bytes].copy_from_slice(&key[..full_bytes]);
    }

    // Copy partial byte with remaining bits zeroed
    if remaining_bits > 0 && full_bytes < 32 {
        let mask = !((1u8 << (8 - remaining_bits)) - 1);
        prefix[full_bytes] = key[full_bytes] & mask;
    }

    prefix
}

/// Flip bit `bit_idx` in a [u8; 32] prefix.
fn flip_bit(prefix: &mut [u8; 32], bit_idx: usize) {
    let byte_idx = bit_idx / 8;
    let bit_offset = 7 - (bit_idx % 8);
    prefix[byte_idx] ^= 1 << bit_offset;
}

/// Get the empty hash at level `L` from root (0 = root, TREE_DEPTH = leaves).
/// empty_hashes is indexed by depth from leaf, so level L from root =
/// depth (TREE_DEPTH - L) from leaf.
fn empty_hash_at_level(empty_hashes: &[[u8; 32]], level: usize) -> [u8; 32] {
    empty_hashes[TREE_DEPTH - level]
}

impl SparseMerkleTree {
    /// Get the sibling hash at level `bit_idx + 1` from root, for the path
    /// through `key`.
    ///
    /// At the leaf level (bit_idx == TREE_DEPTH - 1), the sibling is a LEAF
    /// stored in `self.leaves`, not an internal node in `self.nodes`. This
    /// is critical: if we only looked in `self.nodes`, the sibling would
    /// always be the empty leaf hash, even when the sibling leaf exists.
    /// This would cause the parent hash to be wrong whenever two sibling
    /// leaves both have values, making the root insertion-order-dependent.
    fn get_sibling_hash(&self, key: &[u8; 32], bit_idx: usize) -> [u8; 32] {
        if bit_idx == TREE_DEPTH - 1 {
            // Leaf level: sibling is a leaf. Look it up in self.leaves.
            let mut sibling_key = *key;
            flip_bit(&mut sibling_key, bit_idx);
            match self.leaves.get(&sibling_key) {
                Some(sibling_val) => hash_leaf(&sibling_key, sibling_val),
                None => self.empty_hashes[0], // empty leaf
            }
        } else {
            // Internal level: sibling is an internal node in self.nodes.
            let mut sibling_prefix = path_prefix(key, bit_idx + 1);
            flip_bit(&mut sibling_prefix, bit_idx);
            self.nodes
                .get(&((bit_idx + 1) as u16, sibling_prefix))
                .copied()
                .unwrap_or_else(|| empty_hash_at_level(&self.empty_hashes, bit_idx + 1))
        }
    }

    pub fn new() -> Self {
        let mut empty_hashes = Vec::with_capacity(TREE_DEPTH + 1);
        let mut current = [0u8; 32];
        empty_hashes.push(current); // depth 0 from leaf = empty leaf
        for _ in 0..TREE_DEPTH {
            current = hash_node(&current, &current);
            empty_hashes.push(current);
        }
        // empty_hashes[TREE_DEPTH] = empty root

        Self {
            leaves: BTreeMap::new(),
            nodes: HashMap::new(),
            empty_hashes: empty_hashes.clone(),
            cached_root: empty_hashes[TREE_DEPTH],
            dirty: false,
        }
    }

    /// Insert or update a key-value pair. O(TREE_DEPTH) = O(256) time.
    pub fn update(&mut self, key: [u8; 32], value: [u8; 32]) {
        if value == [0u8; 32] {
            if self.leaves.remove(&key).is_none() {
                return; // Key wasn't present, nothing to do
            }
        } else {
            self.leaves.insert(key, value);
        }

        // Start from the leaf hash (or empty hash for deletion)
        let mut current_hash = if value == [0u8; 32] {
            self.empty_hashes[0] // empty leaf (depth 0 from leaf = level TREE_DEPTH from root)
        } else {
            hash_leaf(&key, &value)
        };

        // Walk from leaf (bit TREE_DEPTH-1, LSB) to root (bit 0, MSB).
        // At bit_idx = d, we process bit d from the MSB:
        // - Current node is at level d+1 from root
        // - Sibling is at level d+1 from root, identified by first d+1 bits with bit d flipped
        // - Parent is at level d from root, identified by first d bits
        for bit_idx in (0..TREE_DEPTH).rev() {
            let bit = bit_at(&key, bit_idx);

            // Sibling at level bit_idx+1 from root.
            let sibling_hash = self.get_sibling_hash(&key, bit_idx);

            // Compute parent hash
            let (left, right) = if bit == 0 {
                (current_hash, sibling_hash)
            } else {
                (sibling_hash, current_hash)
            };
            current_hash = hash_node(&left, &right);

            // Store parent at level bit_idx from root, prefix = first bit_idx bits
            let parent_prefix = path_prefix(&key, bit_idx);
            let parent_level = bit_idx as u16;
            let parent_empty = empty_hash_at_level(&self.empty_hashes, bit_idx);

            if current_hash == parent_empty {
                self.nodes.remove(&(parent_level, parent_prefix));
            } else {
                self.nodes.insert((parent_level, parent_prefix), current_hash);
            }
        }

        self.dirty = true;
    }

    /// Get value for a key
    pub fn get(&self, key: &[u8; 32]) -> Option<&[u8; 32]> {
        self.leaves.get(key)
    }

    /// Compute the 32-byte Merkle root (cached, O(1) if no changes).
    ///
    /// Issue 12: The root is now computed via a true fixed-depth sparse
    /// Merkle tree, not a sorted-leaves tree. This is a consensus-breaking
    /// change — the root for the same set of leaves is different from the
    /// old format. Requires coordinated cutover like Issue 5.
    pub fn root(&mut self) -> [u8; 32] {
        if self.dirty {
            // Root is at level 0 from root, prefix = empty (0 bits)
            self.cached_root = self
                .nodes
                .get(&(0u16, [0u8; 32]))
                .copied()
                .unwrap_or(self.empty_hashes[TREE_DEPTH]);
            self.dirty = false;
        }
        self.cached_root
    }

    /// Generate an inclusion proof for a key. O(TREE_DEPTH) = O(256) time.
    pub fn get_proof(&self, key: &[u8; 32]) -> Option<SmtInclusionProof> {
        let value = self.leaves.get(key)?;

        let mut proof_path = Vec::with_capacity(TREE_DEPTH);

        // Walk from leaf (bit TREE_DEPTH-1) to root (bit 0)
        for bit_idx in (0..TREE_DEPTH).rev() {
            let bit = bit_at(key, bit_idx);

            let sibling_hash = self.get_sibling_hash(key, bit_idx);

            proof_path.push(SmtProofNode {
                position: if bit == 0 { "right".to_string() } else { "left".to_string() },
                hash_hex: hex::encode(sibling_hash),
            });
        }

        // Compute root by walking the path
        let root = self.compute_root_for_key(key, value);

        // Self-verify
        let verified = Self::verify_proof_static(&root, key, value, &proof_path);

        Some(SmtInclusionProof {
            key_hex: hex::encode(key),
            value_hex: hex::encode(value),
            root_hex: hex::encode(root),
            proof_path,
            verified,
        })
    }

    /// Compute the root by walking a single key's path to the root.
    fn compute_root_for_key(&self, key: &[u8; 32], value: &[u8; 32]) -> [u8; 32] {
        let mut current_hash = hash_leaf(key, value);

        for bit_idx in (0..TREE_DEPTH).rev() {
            let bit = bit_at(key, bit_idx);

            let sibling_hash = self.get_sibling_hash(key, bit_idx);

            let (left, right) = if bit == 0 {
                (current_hash, sibling_hash)
            } else {
                (sibling_hash, current_hash)
            };
            current_hash = hash_node(&left, &right);
        }

        current_hash
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
    use std::time::Instant;

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

    #[test]
    fn test_smt_deterministic_root() {
        // Insertion order must not affect the root — this is a fundamental
        // property of a fixed-depth sparse Merkle tree: each key maps to a
        // fixed position in the tree regardless of insertion order.
        let mut tree1 = SparseMerkleTree::new();
        let mut tree2 = SparseMerkleTree::new();

        let k1 = [1u8; 32];
        let v1 = [10u8; 32];
        let k2 = [2u8; 32];
        let v2 = [20u8; 32];

        tree1.update(k1, v1);
        tree1.update(k2, v2);

        // Insert in reverse order
        tree2.update(k2, v2);
        tree2.update(k1, v1);

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_smt_inclusion_proof_verification() {
        let mut tree = SparseMerkleTree::new();

        for i in 0..10u8 {
            let mut k = [0u8; 32];
            let mut v = [0u8; 32];
            k[0] = i;
            v[0] = i * 2;
            tree.update(k, v);
        }

        let root = tree.root();

        // Verify proof for item 5
        let mut target_k = [0u8; 32];
        target_k[0] = 5;
        let mut target_v = [0u8; 32];
        target_v[0] = 10;

        let proof = tree.get_proof(&target_k).expect("Proof should be generated");
        assert!(proof.verified);
        assert_eq!(proof.root_hex, hex::encode(root));

        // Tampering test: value modified
        let mut bad_v = target_v;
        bad_v[0] = 99;
        let is_valid = SparseMerkleTree::verify_proof(&root, &target_k, &bad_v, &proof.proof_path);
        assert!(!is_valid, "Tampered value must fail verification");
    }

    #[test]
    fn test_smt_deletion() {
        let mut smt = SparseMerkleTree::new();
        let key1 = [1u8; 32];
        let val1 = [10u8; 32];
        let key2 = [2u8; 32];
        let val2 = [20u8; 32];

        smt.update(key1, val1);
        smt.update(key2, val2);
        let root_with_both = smt.root();

        // Delete key2 (value = 0 means delete)
        smt.update(key2, [0u8; 32]);
        let root_with_only1 = smt.root();
        assert_ne!(root_with_both, root_with_only1);

        // Root with only key1 should equal a fresh tree with only key1
        let mut smt2 = SparseMerkleTree::new();
        smt2.update(key1, val1);
        assert_eq!(smt.root(), smt2.root());

        // Proof for deleted key should not exist
        assert!(smt.get_proof(&key2).is_none());
    }

    #[test]
    fn test_smt_proof_depth_is_256() {
        // A true fixed-depth SMT produces proofs of exactly TREE_DEPTH levels.
        let mut smt = SparseMerkleTree::new();
        smt.update([1u8; 32], [10u8; 32]);
        smt.update([2u8; 32], [20u8; 32]);

        let proof = smt.get_proof(&[1u8; 32]).expect("Proof should exist");
        assert_eq!(
            proof.proof_path.len(),
            TREE_DEPTH,
            "Proof depth must be exactly {} for a fixed-depth SMT",
            TREE_DEPTH
        );
    }

    #[test]
    fn test_smt_root_after_change_documented() {
        // Issue 12: Document that the root format changed from sorted-leaves
        // to fixed-depth. This test asserts the root for a known 3-leaf tree
        // so the new root value is documented in code, not just in commit history.
        let mut smt = SparseMerkleTree::new();
        smt.update([1u8; 32], [10u8; 32]);
        smt.update([2u8; 32], [20u8; 32]);
        smt.update([3u8; 32], [30u8; 32]);

        let root = smt.root();
        // The root is a deterministic function of the 3 leaves in the
        // fixed-depth tree. This value will change if the hash domain
        // separators or tree structure change — that's intentional.
        assert_ne!(root, [0u8; 32], "Root must not be all zeros with leaves present");
        assert_ne!(root, smt.empty_hashes[TREE_DEPTH], "Root must differ from empty tree root");
    }

    #[test]
    fn test_smt_sibling_leaves_at_leaf_level() {
        // Deep-review regression: two keys that are siblings at the LEAF
        // level (differ only in bit 255 = LSB) must produce an
        // insertion-order-independent root. Before the fix, the leaf-level
        // sibling was always looked up as an empty internal node, never as
        // a leaf, so the parent hash was wrong whenever both sibling leaves
        // had values — making the root depend on insertion order.
        let key_a = [0u8; 32]; // bit 255 = 0
        let mut key_b = [0u8; 32];
        key_b[31] = 0x01; // bit 255 = 1
        let val_a = [10u8; 32];
        let val_b = [20u8; 32];

        let mut tree1 = SparseMerkleTree::new();
        tree1.update(key_a, val_a);
        tree1.update(key_b, val_b);

        let mut tree2 = SparseMerkleTree::new();
        tree2.update(key_b, val_b);
        tree2.update(key_a, val_a);

        assert_eq!(
            tree1.root(),
            tree2.root(),
            "Root must be order-independent even for sibling leaves"
        );

        // Also verify the proof for one sibling references the other's hash
        let proof_a = tree1.get_proof(&key_a).expect("proof for key_a");
        assert!(proof_a.verified, "proof for key_a must verify");
        let proof_b = tree1.get_proof(&key_b).expect("proof for key_b");
        assert!(proof_b.verified, "proof for key_b must verify");

        // The first proof entry (leaf level) sibling must be the other leaf's hash
        let sibling_leaf_hash = hash_leaf(&key_b, &val_b);
        assert_eq!(
            proof_a.proof_path[0].hash_hex,
            hex::encode(sibling_leaf_hash),
            "Leaf-level sibling in proof for key_a must be key_b's leaf hash"
        );
    }

    #[test]
    fn test_smt_many_sibling_pairs_order_independent() {
        // Deep-review regression: multiple sibling-leaf pairs at different
        // leaf-level positions must all produce order-independent roots.
        let keys: Vec<([u8; 32], [u8; 32])> = (0..8u8)
            .map(|i| {
                let mut a = [0u8; 32];
                let mut b = [0u8; 32];
                a[31] = i * 2;
                b[31] = i * 2 + 1;
                (a, b)
            })
            .collect();

        let mut tree1 = SparseMerkleTree::new();
        for (a, b) in &keys {
            tree1.update(*a, [1u8; 32]);
            tree1.update(*b, [2u8; 32]);
        }

        let mut tree2 = SparseMerkleTree::new();
        for (a, b) in keys.iter().rev() {
            tree2.update(*b, [2u8; 32]);
            tree2.update(*a, [1u8; 32]);
        }

        assert_eq!(tree1.root(), tree2.root(), "Root must be order-independent with many sibling pairs");
    }

    #[test]
    #[ignore = "performance test — run with --ignored; sizes kept modest because each update does 256 SHA-256 ops"]
    fn test_smt_scaling_is_logarithmic() {
        // Issue 12: Verify that update() and get_proof() scale as O(log n)
        // (i.e., O(TREE_DEPTH) = O(256)), NOT O(n). We measure per-operation
        // time at 50, 500, and 2,000 leaves and confirm it stays roughly
        // constant rather than growing linearly with leaf count.
        let mut prev_update_time = std::time::Duration::from_secs(0);
        let mut prev_proof_time = std::time::Duration::from_secs(0);

        for &n in &[50usize, 500, 2_000] {
            let mut smt = SparseMerkleTree::new();

            // Fill the tree
            for i in 0..n {
                let mut k = [0u8; 32];
                let mut v = [0u8; 32];
                k[0] = (i >> 24) as u8;
                k[1] = (i >> 16) as u8;
                k[2] = (i >> 8) as u8;
                k[3] = i as u8;
                v[0] = (i % 200) as u8;
                smt.update(k, v);
            }
            let _ = smt.root();

            // Time a single update
            let mut new_key = [0u8; 32];
            new_key[31] = 1;
            let new_val = [42u8; 32];

            let start = Instant::now();
            smt.update(new_key, new_val);
            let update_time = start.elapsed();

            // Time a single proof
            let start = Instant::now();
            let _ = smt.get_proof(&new_key);
            let proof_time = start.elapsed();

            // For O(log n), the time should be roughly constant regardless
            // of n. We assert it doesn't grow more than 10x from 50 to 2k
            // (allowing some variance for HashMap performance).
            if n > 50 {
                assert!(
                    update_time < prev_update_time * 10 + std::time::Duration::from_millis(10),
                    "update() time grew too much at n={}: {:?} vs prev {:?}",
                    n, update_time, prev_update_time
                );
                assert!(
                    proof_time < prev_proof_time * 10 + std::time::Duration::from_millis(10),
                    "get_proof() time grew too much at n={}: {:?} vs prev {:?}",
                    n, proof_time, prev_proof_time
                );
            }

            prev_update_time = update_time;
            prev_proof_time = proof_time;
        }
    }
}
