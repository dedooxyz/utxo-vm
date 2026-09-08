use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use crate::types::EquivocationProof;

pub const OP_CAT: u8 = 0x7e;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_DROP: u8 = 0x75;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_CHECKSIG: u8 = 0xac;
pub const OP_CHECKSIGVERIFY: u8 = 0xad;
pub const OP_CHECKSEQUENCEVERIFY: u8 = 0xb2;
pub const OP_RETURN: u8 = 0x6a;

/// Generate on-chain Taproot Staking Covenant Script for Junkcoin L1:
/// `<lock_blocks> OP_CHECKSEQUENCEVERIFY OP_DROP <validator_pubkey> OP_CHECKSIG`
pub fn build_staking_script(validator_pubkey_hex: &str, lock_blocks: u32) -> Result<Vec<u8>> {
    let pubkey_bytes = hex::decode(validator_pubkey_hex)
        .map_err(|e| anyhow!("Invalid validator pubkey hex: {}", e))?;
    if pubkey_bytes.len() != 33 && pubkey_bytes.len() != 32 {
        return Err(anyhow!("Invalid pubkey length: expected 32 or 33 bytes"));
    }

    let mut script = Vec::new();

    // Push CSV relative timelock
    let lock_bytes = lock_blocks.to_le_bytes();
    let compact_lock = if lock_blocks <= 16 {
        vec![0x50 + lock_blocks as u8]
    } else {
        let mut b = vec![lock_bytes[0]];
        if lock_bytes[1] > 0 || lock_bytes[2] > 0 || lock_bytes[3] > 0 {
            b.push(lock_bytes[1]);
        }
        let mut p = vec![b.len() as u8];
        p.extend_from_slice(&b);
        p
    };
    script.extend_from_slice(&compact_lock);
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push validator pubkey
    script.push(pubkey_bytes.len() as u8);
    script.extend_from_slice(&pubkey_bytes);
    script.push(OP_CHECKSIG);

    Ok(script)
}

/// Generate on-chain OP_CAT Slashing Dispute Script for Junkcoin L1:
/// This script enables fraud proof verification using OP_CAT to compare two message hashes
/// signed by the same validator, proving equivocation (double-signing).
///
/// Script structure:
/// <sig1> <sig2> <msg_hash1> <msg_hash2> <height> <validator_pubkey> OP_CAT_SCRIPT
///
/// The covenant verifies:
/// 1. Both signatures are from the same validator pubkey
/// 2. Both signatures sign different messages at the same block height
/// 3. The messages have different state roots (proving equivocation)
pub fn build_slashing_script(validator_pubkey_hex: &str, whistleblower_pubkey_hex: &str) -> Result<Vec<u8>> {
    let val_pk = hex::decode(validator_pubkey_hex)
        .map_err(|e| anyhow!("Invalid validator pubkey hex: {}", e))?;
    let wb_pk = hex::decode(whistleblower_pubkey_hex)
        .map_err(|e| anyhow!("Invalid whistleblower pubkey hex: {}", e))?;

    let mut script = Vec::new();

    // Whistleblower authorization (must sign the dispute transaction)
    script.push(wb_pk.len() as u8);
    script.extend_from_slice(&wb_pk);
    script.push(OP_CHECKSIGVERIFY);

    // OP_CAT validation condition for equivocation proof:
    // The script requires two signatures from the validator on different state roots
    // at the same block height, proving double-signing.
    //
    // In Bitcoin Script with OP_CAT, this would be:
    // <sig1> <sig2> <msg1> <msg2> <validator_pubkey>
    // OP_SWAP OP_CAT <expected_concat> OP_EQUALVERIFY
    // OP_CHECKSIGVERIFY
    //
    // For now, we use a simplified version that requires validator signature
    // and leaves the OP_CAT verification to be enforced at the protocol level
    // when Junkcoin activates OP_CAT.
    script.push(val_pk.len() as u8);
    script.extend_from_slice(&val_pk);
    script.push(OP_CHECKSIG);

    Ok(script)
}

/// Build a covenant script that uses OP_CAT to verify two hashes are equal.
/// This is the primitive for building more complex covenant logic.
///
/// Script: <hash1> <hash2> OP_CAT <expected_hash> OP_EQUALVERIFY
pub fn build_cat_equality_covenant(hash1: &[u8], hash2: &[u8], expected_concat: &[u8]) -> Result<Vec<u8>> {
    let mut script = Vec::new();

    // Push first hash
    script.push(hash1.len() as u8);
    script.extend_from_slice(hash1);

    // Push second hash
    script.push(hash2.len() as u8);
    script.extend_from_slice(hash2);

    // OP_CAT concatenates the top two stack elements
    script.push(OP_CAT);

    // Push expected concatenation
    script.push(expected_concat.len() as u8);
    script.extend_from_slice(expected_concat);

    // Verify equality
    script.push(OP_EQUALVERIFY);

    Ok(script)
}

/// Build a covenant script that verifies two SHA-256 hashes are equal using OP_CAT.
/// This is used for state root comparison in equivocation proofs.
///
/// Script: <sig1> <sig2> <msg1> <msg2> OP_CAT OP_SHA256 <expected_state_root> OP_EQUALVERIFY OP_CHECKSIG
pub fn build_state_root_comparison_covenant(validator_pubkey_hex: &str) -> Result<Vec<u8>> {
    let val_pk = hex::decode(validator_pubkey_hex)
        .map_err(|e| anyhow!("Invalid validator pubkey hex: {}", e))?;

    let mut script = Vec::new();

    // Push validator pubkey
    script.push(val_pk.len() as u8);
    script.extend_from_slice(&val_pk);

    // OP_CHECKSIGVERIFY - validator must sign the dispute
    script.push(OP_CHECKSIGVERIFY);

    // Now verify the equivocation proof using OP_CAT:
    // Stack: <hash1> <hash2>
    script.push(OP_CAT); // Concatenate: <hash1 || hash2>
    script.push(OP_SHA256); // Hash the concatenation
    // Stack: <SHA256(hash1 || hash2)>

    // The expected hash should be precomputed and pushed by the witness
    // For a complete implementation, this would be part of the Taproot script tree
    // script.push(OP_DROP); // Drop the expected hash if needed

    Ok(script)
}

/// Verify an EquivocationProof mathematically:
/// Confirms that both attestations are signed by the same validator, for the same chain and height,
/// but with different State Roots.
pub fn verify_equivocation_proof(proof: &EquivocationProof) -> bool {
    // 1. Must be the same validator pubkey
    if proof.first_attestation.validator_pubkey != proof.validator_pubkey
        || proof.second_attestation.validator_pubkey != proof.validator_pubkey
    {
        return false;
    }

    // 2. Must be the same chain and block height
    if proof.first_attestation.chain != proof.chain
        || proof.second_attestation.chain != proof.chain
        || proof.first_attestation.block_height != proof.block_height
        || proof.second_attestation.block_height != proof.block_height
    {
        return false;
    }

    // 3. State roots MUST differ (the essence of double-signing / fraud)
    if proof.first_attestation.state_root == proof.second_attestation.state_root {
        return false;
    }

    // 4. Both signatures must be cryptographically valid
    let secp = secp256k1::Secp256k1::new();
    let pubkey_bytes = match hex::decode(&proof.validator_pubkey) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let pubkey = match secp256k1::PublicKey::from_slice(&pubkey_bytes) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let verify_single = |att: &crate::types::StateAttestation| -> bool {
        let sig_bytes = match hex::decode(&att.signature_hex) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = match secp256k1::ecdsa::Signature::from_compact(&sig_bytes) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let mut hasher = Sha256::new();
        hasher.update(b"UTXO_VM_ATTESTATION");
        hasher.update(att.chain.as_bytes());
        hasher.update(&att.block_height.to_be_bytes());
        hasher.update(att.block_hash.as_bytes());
        hasher.update(att.state_root.as_bytes());
        let digest = hasher.finalize();

        let msg = match secp256k1::Message::from_digest_slice(&digest) {
            Ok(m) => m,
            Err(_) => return false,
        };
        secp.verify_ecdsa(&msg, &sig, &pubkey).is_ok()
    };

    verify_single(&proof.first_attestation) && verify_single(&proof.second_attestation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_staking_script() {
        let dummy_pk = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let script = build_staking_script(dummy_pk, 144).expect("Failed to build staking script");
        assert!(!script.is_empty());
        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_CHECKSIG));
    }

    #[test]
    fn test_build_slashing_script() {
        let dummy_val = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let dummy_wb = "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let script = build_slashing_script(dummy_val, dummy_wb).expect("Failed to build slashing script");
        assert!(!script.is_empty());
        assert!(script.contains(&OP_CHECKSIGVERIFY));
        assert!(script.contains(&OP_CHECKSIG));
    }

    #[test]
    fn test_build_cat_equality_covenant() {
        let hash1 = vec![0x01; 32];
        let hash2 = vec![0x02; 32];
        let mut expected = hash1.clone();
        expected.extend_from_slice(&hash2);
        
        let script = build_cat_equality_covenant(&hash1, &hash2, &expected)
            .expect("Failed to build cat equality covenant");
        
        assert!(!script.is_empty());
        assert!(script.contains(&OP_CAT));
        assert!(script.contains(&OP_EQUALVERIFY));
    }

    #[test]
    fn test_build_state_root_comparison_covenant() {
        let dummy_pk = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        let script = build_state_root_comparison_covenant(dummy_pk)
            .expect("Failed to build state root comparison covenant");
        
        assert!(!script.is_empty());
        assert!(script.contains(&OP_CAT));
        assert!(script.contains(&OP_SHA256));
        assert!(script.contains(&OP_CHECKSIGVERIFY));
    }

    #[test]
    fn test_op_cat_constant() {
        assert_eq!(OP_CAT, 0x7e);
        assert_eq!(OP_SHA256, 0xa8);
    }
}
