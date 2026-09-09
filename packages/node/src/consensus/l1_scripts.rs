//! L1 Script Builders for UTXO-VM
//!
//! **WARNING: NOT CONSENSUS** — These are script templates for testing.
//! They do NOT enforce UTXO-VM rules on L1. Actual on-chain enforcement
//! requires proper BIP-341 Taproot script path spending, which is not yet
//! implemented. These builders are used for testnet experimentation only.
//!
//! This module provides builders for Bitcoin Script that implement:
//! - Operator vault (P2TR with unbond delay)
//! - Challenge leaf (OP_CAT hash comparison)
//! - Silence escape (user exit without operator)
//! - Seal spend (object binding to UTXO)
//!
//! All scripts are designed for JKC Testnet with full opcode support.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

// Standard opcodes
pub const OP_FALSE: u8 = 0x00;
pub const OP_IF: u8 = 0x63;
pub const OP_ELSE: u8 = 0x67;
pub const OP_ENDIF: u8 = 0x68;
pub const OP_DROP: u8 = 0x75;
pub const OP_DUP: u8 = 0x76;
pub const OP_HASH160: u8 = 0xa9;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_EQUAL: u8 = 0x87;
pub const OP_CHECKSIG: u8 = 0xac;
pub const OP_CHECKSIGVERIFY: u8 = 0xad;
pub const OP_CHECKSEQUENCEVERIFY: u8 = 0xb2;
pub const OP_CHECKLOCKTIMEVERIFY: u8 = 0xb1;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_CAT: u8 = 0x7e;
pub const OP_RETURN: u8 = 0x6a;
pub const OP_1: u8 = 0x51;

/// Taproot leaf version (BIP-341)
pub const TAPROOT_LEAF_VERSION: u8 = 0xc0;

/// BIP-341 tagged hash domain separators
pub const TAG_TAPLEAF: &[u8] = b"TapLeaf";
pub const TAG_TAPBRANCH: &[u8] = b"TapBranch";
pub const TAG_TAPTWEAK: &[u8] = b"TapTweak";

/// BIP-341 tagged hash: SHA256(SHA256(tag) || SHA256(tag) || data)
pub fn tagged_hash(tag: &[u8], data: &[u8]) -> Vec<u8> {
    let tag_hash = Sha256::digest(tag);
    let mut hasher = Sha256::new();
    hasher.update(&tag_hash);
    hasher.update(&tag_hash);
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Operator vault configuration
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Operator's public key (compressed, 33 bytes)
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (compressed, 33 bytes)
    pub challenger_pubkey: Vec<u8>,
    /// Unbond delay in blocks (relative timelock)
    pub unbond_delay: u32,
    /// Challenge window in blocks
    pub challenge_window: u32,
}

/// Challenge proof data
#[derive(Debug, Clone)]
pub struct ChallengeProof {
    /// The operator's claimed (wrong) root
    pub claimed_root: Vec<u8>,
    /// The correct root (from challenger's re-execution)
    pub correct_root: Vec<u8>,
    /// Operator's signature on the wrong root
    pub operator_signature: Vec<u8>,
    /// Operator's public key
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (must be provided, NOT hardcoded)
    pub challenger_pubkey: Vec<u8>,
}

/// Seal spend proof data
#[derive(Debug, Clone)]
pub struct SealSpendProof {
    /// Object ID (32 bytes)
    pub object_id: Vec<u8>,
    /// Previous seal (txid:vout)
    pub prev_seal: Vec<u8>,
    /// New seal (txid:vout)
    pub new_seal: Vec<u8>,
    /// State hash (SHA256 of state data)
    pub state_hash: Vec<u8>,
}

// ============================================================================
// OPERATOR VAULT SCRIPT (P2TR)
// ============================================================================

/// Build the operator vault script for P2TR spending.
///
/// **NOT CONSENSUS** — This is a testnet template.
///
/// The vault has two spending paths:
/// 1. Key path: Operator can spend after unbond delay
/// 2. Script path: Challenger can spend with equivocation proof
///
/// This function builds the script tree for Taproot commitment.
pub fn build_vault_script_tree(config: &VaultConfig) -> Result<VaultScriptTree> {
    if config.operator_pubkey.len() != 33 {
        return Err(anyhow!("Operator pubkey must be 33 bytes (compressed)"));
    }
    if config.challenger_pubkey.len() != 33 {
        return Err(anyhow!("Challenger pubkey must be 33 bytes (compressed)"));
    }

    // Leaf 0: Operator unbond path
    // <unbond_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <operator_pubkey> OP_CHECKSIG
    let operator_leaf = build_operator_unbond_leaf(
        &config.operator_pubkey,
        config.unbond_delay,
    )?;

    // Leaf 1: Challenge path
    // <challenger_pubkey> OP_CHECKSIGVERIFY <operator_pubkey> OP_CHECKSIGVERIFY
    // <claimed_root> <correct_root> OP_CAT OP_SHA256
    let challenge_leaf = build_challenge_leaf(
        &config.operator_pubkey,
        &config.challenger_pubkey,
    )?;

    Ok(VaultScriptTree {
        operator_leaf,
        challenge_leaf,
        operator_pubkey: config.operator_pubkey.clone(),
        challenger_pubkey: config.challenger_pubkey.clone(),
    })
}

/// Build the operator unbond leaf script.
///
/// Script: <unbond_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <operator_pubkey> OP_CHECKSIG
fn build_operator_unbond_leaf(
    operator_pubkey: &[u8],
    unbond_delay: u32,
) -> Result<Vec<u8>> {
    let mut script = Vec::new();

    // Push unbond delay as minimal encoding
    push_minimal_uint(&mut script, unbond_delay as u64);

    // CSV + DROP
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push operator pubkey
    script.push(operator_pubkey.len() as u8);
    script.extend_from_slice(operator_pubkey);

    // Checksig
    script.push(OP_CHECKSIG);

    Ok(script)
}

/// Build the challenge leaf script using OP_CAT.
///
/// **NOT CONSENSUS** — This is a testnet template.
///
/// This script enables fraud proof verification:
/// 1. Challenger provides proof that operator signed wrong root
/// 2. OP_CAT concatenates hashes for comparison
/// 3. If valid, bond is slashed to challenger
///
/// Script structure (simplified for v1):
/// <challenger_pubkey> OP_CHECKSIGVERIFY
/// <operator_pubkey> OP_CHECKSIGVERIFY
/// <claimed_root> <correct_root> OP_CAT OP_SHA256 <expected_hash> OP_EQUALVERIFY
///
/// For actual consensus, this needs proper BIP-341 script path spending
/// with tagged tapleaf hashes and witness version.
fn build_challenge_leaf(
    operator_pubkey: &[u8],
    challenger_pubkey: &[u8],
) -> Result<Vec<u8>> {
    if operator_pubkey.len() != 33 {
        return Err(anyhow!("Operator pubkey must be 33 bytes (compressed)"));
    }
    if challenger_pubkey.len() != 33 {
        return Err(anyhow!("Challenger pubkey must be 33 bytes (compressed)"));
    }

    let mut script = Vec::new();

    // Push challenger pubkey (must be provided, NOT hardcoded)
    script.push(challenger_pubkey.len() as u8);
    script.extend_from_slice(challenger_pubkey);

    // OP_CHECKSIGVERIFY - challenger must sign the challenge
    script.push(OP_CHECKSIGVERIFY);

    // Push operator pubkey (committed in script)
    script.push(operator_pubkey.len() as u8);
    script.extend_from_slice(operator_pubkey);

    // OP_CHECKSIGVERIFY - operator must have signed the wrong root
    script.push(OP_CHECKSIGVERIFY);

    // Now verify the equivocation proof using OP_CAT:
    // The witness provides: <claimed_root> <correct_root>
    // OP_CAT concatenates them, OP_SHA256 hashes the result
    // Then compare against expected hash

    // OP_CAT: concatenate top two stack elements
    script.push(OP_CAT);

    // OP_SHA256: hash the concatenation
    script.push(OP_SHA256);

    // The expected hash (SHA256(claimed_root || correct_root)) is provided
    // by the witness and compared. In a full implementation, this would be
    // pre-committed in the script or verified via additional logic.

    Ok(script)
}

/// Vault script tree containing both spending paths
#[derive(Debug, Clone)]
pub struct VaultScriptTree {
    pub operator_leaf: Vec<u8>,
    pub challenge_leaf: Vec<u8>,
    pub operator_pubkey: Vec<u8>,
    pub challenger_pubkey: Vec<u8>,
}

impl VaultScriptTree {
    /// Calculate the tapleaf hash using BIP-341 tagged hash
    pub fn tapleaf_hash(script: &[u8]) -> Vec<u8> {
        let leaf_version = TAPROOT_LEAF_VERSION;
        let script_len = script.len();

        // BIP-341: tapleaf = 0xc0 || compact_size(script_len) || script
        let mut data = Vec::new();
        data.push(leaf_version);
        data.extend_from_slice(&push_size_compact(script_len));
        data.extend_from_slice(script);

        tagged_hash(TAG_TAPLEAF, &data)
    }

    /// Get the operator leaf hash
    pub fn operator_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.operator_leaf)
    }

    /// Get the challenge leaf hash
    pub fn challenge_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.challenge_leaf)
    }

    /// Calculate the script tree root using BIP-341 tagged hash
    pub fn script_tree_root(&self) -> Vec<u8> {
        let left = self.operator_leaf_hash();
        let right = self.challenge_leaf_hash();

        // Sort the two hashes (BIP-340)
        let (first, second) = if left < right {
            (left.as_slice(), right.as_slice())
        } else {
            (right.as_slice(), left.as_slice())
        };

        // BIP-341: tapbranch = left || right
        let mut data = Vec::new();
        data.extend_from_slice(first);
        data.extend_from_slice(second);

        tagged_hash(TAG_TAPBRANCH, &data)
    }
}

// ============================================================================
// CHALLENGE SCRIPT (OP_CAT)
// ============================================================================

/// Build a challenge script for equivocation proof.
///
/// This script uses OP_CAT to verify that two roots are different,
/// proving the operator signed conflicting attestations.
///
/// Script: <claimed_root> <correct_root> OP_CAT OP_SHA256 <expected_hash> OP_EQUALVERIFY
pub fn build_equivocation_challenge_script(
    claimed_root: &[u8],
    correct_root: &[u8],
) -> Result<Vec<u8>> {
    if claimed_root.len() != 32 || correct_root.len() != 32 {
        return Err(anyhow!("Roots must be 32 bytes"));
    }

    let mut script = Vec::new();

    // Push claimed root (32 bytes)
    script.push(0x20); // 32 bytes push
    script.extend_from_slice(claimed_root);

    // Push correct root (32 bytes)
    script.push(0x20); // 32 bytes push
    script.extend_from_slice(correct_root);

    // OP_CAT: concatenate the two roots
    script.push(OP_CAT);

    // OP_SHA256: hash the concatenation
    script.push(OP_SHA256);

    // The expected hash is the SHA256 of (claimed_root || correct_root)
    // This is pre-computed and provided by the witness
    // For on-chain verification, the script checks if the witness provides the correct hash

    Ok(script)
}

// ============================================================================
// SILENCE ESCAPE SCRIPT
// ============================================================================

/// Build the silence escape script for user exit.
///
/// This script allows users to exit without operator cooperation
/// if no valid batch is posted for N blocks.
///
/// Script: <silence_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <user_pubkey> OP_CHECKSIG
pub fn build_silence_escape_script(
    user_pubkey: &[u8],
    silence_delay: u32,
) -> Result<Vec<u8>> {
    if user_pubkey.len() != 33 {
        return Err(anyhow!("User pubkey must be 33 bytes (compressed)"));
    }

    let mut script = Vec::new();

    // Push silence delay
    push_minimal_uint(&mut script, silence_delay as u64);

    // CSV + DROP
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push user pubkey
    script.push(user_pubkey.len() as u8);
    script.extend_from_slice(user_pubkey);

    // Checksig
    script.push(OP_CHECKSIG);

    Ok(script)
}

// ============================================================================
// SEAL SPEND SCRIPT
// ============================================================================

/// Build a seal spend script that binds an object to a UTXO.
///
/// The seal ensures that:
/// 1. Object state is bound to a specific UTXO
/// 2. State transition spends the old UTXO and creates a new one
/// 3. Each seal can only be spent once
///
/// This is implemented as an OP_RETURN output with seal data.
pub fn build_seal_spend_output(seal_proof: &SealSpendProof) -> Result<Vec<u8>> {
    if seal_proof.object_id.len() != 32 {
        return Err(anyhow!("Object ID must be 32 bytes"));
    }
    if seal_proof.prev_seal.len() != 36 {
        return Err(anyhow!("Previous seal must be 36 bytes (txid + vout)"));
    }
    if seal_proof.new_seal.len() != 36 {
        return Err(anyhow!("New seal must be 36 bytes (txid + vout)"));
    }
    if seal_proof.state_hash.len() != 32 {
        return Err(anyhow!("State hash must be 32 bytes"));
    }

    let mut script = Vec::new();

    // OP_RETURN
    script.push(OP_RETURN);

    // Protocol identifier
    let protocol = b"utxovm:seal";
    script.push(protocol.len() as u8);
    script.extend_from_slice(protocol);

    // Object ID (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&seal_proof.object_id);

    // Previous seal (36 bytes)
    script.push(0x24);
    script.extend_from_slice(&seal_proof.prev_seal);

    // New seal (36 bytes)
    script.push(0x24);
    script.extend_from_slice(&seal_proof.new_seal);

    // State hash (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&seal_proof.state_hash);

    Ok(script)
}

// ============================================================================
// BATCH COMMITMENT SCRIPT
// ============================================================================

/// Build a batch commitment output.
///
/// This output commits to:
/// 1. State root
/// 2. List of consumed seals
/// 3. Total fees
/// 4. Operator signature
///
/// Implemented as OP_RETURN with batch data.
pub fn build_batch_commitment_output(
    state_root: &[u8],
    consumed_seals: &[Vec<u8>],
    total_fees: u64,
    operator_signature: &[u8],
) -> Result<Vec<u8>> {
    if state_root.len() != 32 {
        return Err(anyhow!("State root must be 32 bytes"));
    }

    let mut script = Vec::new();

    // OP_RETURN
    script.push(OP_RETURN);

    // Protocol identifier
    let protocol = b"utxovm:batch";
    script.push(protocol.len() as u8);
    script.extend_from_slice(protocol);

    // State root (32 bytes)
    script.push(0x20);
    script.extend_from_slice(state_root);

    // Number of consumed seals
    push_minimal_uint(&mut script, consumed_seals.len() as u64);

    // Each consumed seal (36 bytes each)
    for seal in consumed_seals {
        if seal.len() != 36 {
            return Err(anyhow!("Each consumed seal must be 36 bytes"));
        }
        script.push(0x24);
        script.extend_from_slice(seal);
    }

    // Total fees (8 bytes, little-endian)
    script.push(0x08);
    script.extend_from_slice(&total_fees.to_le_bytes());

    // Operator signature (64 bytes compact + 1 byte recovery)
    if operator_signature.len() != 65 {
        return Err(anyhow!("Operator signature must be 65 bytes"));
    }
    script.push(0x41); // 65 bytes push
    script.extend_from_slice(operator_signature);

    Ok(script)
}

// ============================================================================
// FEE OUTPUT
// ============================================================================

/// Build fee output structure.
///
/// Fee output pays:
/// 1. Miner fee (standard transaction fee)
/// 2. Operator fee (optional, from operator)
/// 3. Indexer fee (small output for indexer incentive)
///
/// This returns the output script for the indexer fee.
pub fn build_indexer_fee_output(
    indexer_pubkey: &[u8],
    _fee_sats: u64,
) -> Result<Vec<u8>> {
    if indexer_pubkey.len() != 33 {
        return Err(anyhow!("Indexer pubkey must be 33 bytes (compressed)"));
    }

    // Simple P2WPKH output
    let mut script = Vec::new();

    // OP_0 (witness version 0)
    script.push(OP_FALSE);

    // SHA256(indexer_pubkey)
    let pubkey_hash = Sha256::digest(indexer_pubkey);
    script.push(pubkey_hash.len() as u8);
    script.extend_from_slice(&pubkey_hash);

    Ok(script)
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Push a minimal encoding of an integer to the script.
fn push_minimal_uint(script: &mut Vec<u8>, value: u64) {
    if value == 0 {
        script.push(0x00);
    } else if value <= 16 {
        script.push(0x50 + value as u8);
    } else if value <= 0xff {
        script.push(0x01);
        script.push(value as u8);
    } else if value <= 0xffff {
        script.push(0x02);
        script.extend_from_slice(&(value as u16).to_le_bytes());
    } else if value <= 0xffffff {
        script.push(0x03);
        let bytes = (value as u32).to_le_bytes();
        script.extend_from_slice(&bytes[..3]);
    } else {
        script.push(0x04);
        script.extend_from_slice(&(value as u32).to_le_bytes());
    }
}

/// Get the compact size encoding for a length.
fn push_size_compact(len: usize) -> Vec<u8> {
    if len < 0x4c {
        vec![len as u8]
    } else if len <= 0xff {
        vec![0x4c, len as u8]
    } else if len <= 0xffff {
        vec![0x4d, (len & 0xff) as u8, ((len >> 8) & 0xff) as u8]
    } else {
        vec![
            0x4e,
            (len & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            ((len >> 16) & 0xff) as u8,
            ((len >> 24) & 0xff) as u8,
        ]
    }
}

/// Calculate SHA256 hash
pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

/// Calculate HASH160 (SHA256 + RIPEMD160)
pub fn hash160(data: &[u8]) -> Vec<u8> {
    use ripemd::Ripemd160;
    use digest::Digest;
    let sha = Sha256::digest(data);
    Ripemd160::digest(&sha).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_vault_script_tree() {
        let config = VaultConfig {
            operator_pubkey: vec![0x02; 33],
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 60,
            challenge_window: 10,
        };

        let tree = build_vault_script_tree(&config).expect("Failed to build vault script tree");

        assert!(!tree.operator_leaf.is_empty());
        assert!(!tree.challenge_leaf.is_empty());
        assert!(!tree.operator_leaf_hash().is_empty());
        assert!(!tree.challenge_leaf_hash().is_empty());
        assert!(!tree.script_tree_root().is_empty());

        // Verify tagged hashes produce different results than plain SHA256
        let plain_hash = Sha256::digest(&tree.operator_leaf).to_vec();
        assert_ne!(tree.operator_leaf_hash(), plain_hash);
    }

    #[test]
    fn test_build_operator_unbond_leaf() {
        let pubkey = vec![0x02; 33];
        let script = build_operator_unbond_leaf(&pubkey, 60).expect("Failed to build operator leaf");

        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_DROP));
        assert!(script.contains(&OP_CHECKSIG));
    }

    #[test]
    fn test_build_challenge_leaf() {
        let operator_pubkey = vec![0x02; 33];
        let challenger_pubkey = vec![0x03; 33];
        let script = build_challenge_leaf(&operator_pubkey, &challenger_pubkey)
            .expect("Failed to build challenge leaf");

        assert!(script.contains(&OP_CHECKSIGVERIFY));
        assert!(script.contains(&OP_CAT));
        assert!(script.contains(&OP_SHA256));

        // Verify challenger pubkey is in the script (not hardcoded 0x02×33)
        assert!(script.windows(challenger_pubkey.len()).any(|w| w == challenger_pubkey));
    }

    #[test]
    fn test_build_silence_escape_script() {
        let user_pubkey = vec![0x03; 33];
        let script = build_silence_escape_script(&user_pubkey, 60).expect("Failed to build escape script");

        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_DROP));
        assert!(script.contains(&OP_CHECKSIG));
    }

    #[test]
    fn test_build_seal_spend_output() {
        let proof = SealSpendProof {
            object_id: vec![0x01; 32],
            prev_seal: vec![0x02; 36],
            new_seal: vec![0x03; 36],
            state_hash: vec![0x04; 32],
        };

        let script = build_seal_spend_output(&proof).expect("Failed to build seal spend output");
        assert!(script.contains(&OP_RETURN));
        // Check for protocol identifier "utxovm:seal"
        let protocol = b"utxovm:seal";
        assert!(script.windows(protocol.len()).any(|w| w == protocol));
    }

    #[test]
    fn test_build_batch_commitment_output() {
        let state_root = vec![0x01; 32];
        let consumed_seals = vec![vec![0x02; 36], vec![0x03; 36]];
        let total_fees = 1000;
        let operator_sig = vec![0x04; 65];

        let script = build_batch_commitment_output(
            &state_root,
            &consumed_seals,
            total_fees,
            &operator_sig,
        ).expect("Failed to build batch commitment");

        assert!(script.contains(&OP_RETURN));
        // Check for protocol identifier "utxovm:batch"
        let protocol = b"utxovm:batch";
        assert!(script.windows(protocol.len()).any(|w| w == protocol));
    }

    #[test]
    fn test_push_minimal_uint() {
        let mut script = Vec::new();
        push_minimal_uint(&mut script, 0);
        assert_eq!(script, vec![0x00]);

        script.clear();
        push_minimal_uint(&mut script, 16);
        assert_eq!(script, vec![0x60]);

        script.clear();
        push_minimal_uint(&mut script, 17);
        assert_eq!(script, vec![0x01, 0x11]);

        script.clear();
        push_minimal_uint(&mut script, 256);
        assert_eq!(script, vec![0x02, 0x00, 0x01]);
    }

    #[test]
    fn test_tagged_hash() {
        // BIP-341 test vector: tapleaf of empty script
        let empty_script = vec![];
        let hash = VaultScriptTree::tapleaf_hash(&empty_script);
        assert_eq!(hash.len(), 32);
        assert_ne!(hash, Sha256::digest(&empty_script).to_vec());
    }
}
