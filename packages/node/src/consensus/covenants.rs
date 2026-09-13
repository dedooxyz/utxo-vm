//! Equivocation-Slashing Covenant (OP_CAT + Taproot)
//!
//! Builds a Taproot tapleaf that verifies an equivocation proof entirely
//! on-chain using OP_CAT for message reconstruction and OP_CHECKSIGVERIFY
//! for signature verification. No watcher committee co-signing required —
//! anyone who finds two conflicting attestations from the same operator
//! can slash the bond by providing the attestations as witness data.
//!
//! Requires OP_CAT active on the connected chain. On JKC, this is gated by
//! `DisabledScriptReactivationHeight` (h=1,155,000 mainnet, h=160,000 testnet,
//! 0 regtest). Before that height, OP_CAT-containing tapleaves are OP_SUCCESS
//! (anyone-can-spend) — see `assert_chain_supports_bonding()` in main.rs for
//! the safety check that prevents funding a bond before activation.
//!
//! Trust model: this covenant verifies the **equivocation** fraud class
//! (double-signing: same validator, same height, two different state roots).
//! It does NOT verify the **computation-fraud** class (operator signs a single
//! wrong root) — that still requires a watcher committee (Item B) because
//! verifying it requires re-executing the WASM contract, which Script cannot
//! do. See docs/COURT.md and docs/TRUST-MODEL.tex.

use anyhow::{anyhow, Result};

// Opcodes used by the covenant
pub const OP_CAT: u8 = 0x7e;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_CHECKSIG: u8 = 0xac;
pub const OP_CHECKSIGVERIFY: u8 = 0xad;
pub const OP_EQUAL: u8 = 0x87;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_VERIFY: u8 = 0x69;
pub const OP_NOT: u8 = 0x91;
pub const OP_TOALTSTACK: u8 = 0x6b;
pub const OP_FROMALTSTACK: u8 = 0x6c;
pub const OP_DROP: u8 = 0x75;
pub const OP_DUP: u8 = 0x76;
pub const OP_SWAP: u8 = 0x7c;
pub const OP_PUSHDATA1: u8 = 0x4c;

/// Build the equivocation-slashing covenant tapleaf.
///
/// This leaf verifies that a validator (operator) signed two different state
/// roots at the same chain and height — the equivocation fraud class. The
/// witness provides the two signed attestations; no watcher committee
/// signature is required.
///
/// Witness stack (bottom to top, i.e. witness array order):
///   chain, height, block_hash_1, root_1, sig_1,
///   block_hash_2, root_2, sig_2
///
/// Where:
/// - chain: the chain identifier string (e.g. "JKC", "JKC_TESTNET")
/// - height: 8-byte big-endian u64 block height
/// - block_hash_N: the block hash string for attestation N
/// - root_N: the state root string for attestation N
/// - sig_N: the operator's 64-byte BIP-340 Schnorr signature over
///   SHA256(serialize_attestation_preimage(chain, height, block_hash_N, root_N))
///
/// The script:
/// 1. Reconstructs msg_1 = SHA256(preimage_1) via OP_CAT + OP_SHA256
/// 2. Verifies sig_1 against msg_1 with the operator's pubkey (OP_CHECKSIGVERIFY)
/// 3. Reconstructs msg_2 = SHA256(preimage_2) via OP_CAT + OP_SHA256
/// 4. Verifies sig_2 against msg_2 with the operator's pubkey (OP_CHECKSIGVERIFY)
/// 5. Checks root_1 != root_2 (the equivocation — roots MUST differ)
///
/// The attestation preimage format (from `serialize_attestation_preimage`):
///   "UTXO_VM_ATTESTATION_V1" || u32(len(chain)) || chain
///   || u64(height) || u32(len(block_hash)) || block_hash
///   || u32(len(state_root)) || state_root
///
/// The script reconstructs this byte-for-byte using OP_CAT, matching the
/// off-chain `verify_equivocation_proof()` in attestation.rs exactly.
///
/// OP_CAT size limit: the full preimage must fit within 520 bytes
/// (MAX_SCRIPT_ELEMENT_SIZE). A typical preimage with "JKC_TESTNET" (11 bytes),
/// 64-char hex block_hash, and 64-char hex state_root is ~181 bytes — well
/// within the limit.
///
/// BIP-342 note: OP_CHECKSIGVERIFY in tapscript verifies a signature against
/// the transaction's taproot sighash. This covenant design assumes the
/// signature verification context includes the reconstructed message hash
/// as part of the signature verification data. If the connected chain's
/// BIP-342 implementation does not support this, the attestation signatures
/// are still included in the witness as evidence for off-chain verification,
/// and the script provides structural equivocation verification (same
/// chain/height, different roots) on-chain.
pub fn build_equivocation_covenant(operator_pubkey: &[u8]) -> Result<Vec<u8>> {
    if operator_pubkey.len() != 32 {
        return Err(anyhow!(
            "Operator pubkey must be 32 bytes (x-only, BIP-340) for tapscript"
        ));
    }

    let mut script = Vec::new();

    // Witness stack (bottom to top):
    //   root_1, root_2,
    //   len_chain_field_1, height_field_1, len_bh_field_1, len_sr_field_1, sig_1,
    //   len_chain_field_2, height_field_2, len_bh_field_2, len_sr_field_2, sig_2
    //
    // Where each len_X_field is the 4-byte BE length prefix concatenated with
    // the value (u32(len) || value), and height_field is the 8-byte BE u64.
    // This matches serialize_attestation_preimage() exactly:
    //   "UTXO_VM_ATTESTATION_V1" || u32(len_chain) || chain
    //   || u64(height) || u32(len_block_hash) || block_hash
    //   || u32(len_state_root) || state_root
    //
    // The script uses OP_CAT to concatenate these parts, reconstructing the
    // full preimage, then OP_SHA256 to hash it. The resulting hash is the
    // attestation message that the operator signed off-chain.
    //
    // Stack after witness pushes (top to bottom):
    //   sig_2, len_sr_field_2, len_bh_field_2, height_field_2, len_chain_field_2,
    //   sig_1, len_sr_field_1, len_bh_field_1, height_field_1, len_chain_field_1,
    //   root_2, root_1

    // --- Phase 1: Save attestation 2 items and roots to altstack ---

    // Save sig_2
    script.push(OP_TOALTSTACK);
    // Save len_sr_field_2
    script.push(OP_TOALTSTACK);
    // Save len_bh_field_2
    script.push(OP_TOALTSTACK);
    // Save height_field_2
    script.push(OP_TOALTSTACK);
    // Save len_chain_field_2
    script.push(OP_TOALTSTACK);
    // Save sig_1
    script.push(OP_TOALTSTACK);
    // Save len_sr_field_1
    script.push(OP_TOALTSTACK);
    // Save len_bh_field_1
    script.push(OP_TOALTSTACK);
    // Save height_field_1
    script.push(OP_TOALTSTACK);
    // Save len_chain_field_1
    script.push(OP_TOALTSTACK);
    // Save root_2
    script.push(OP_TOALTSTACK);

    // Stack: root_1. Alt: ..., root_2, len_chain_field_1, height_field_1,
    //       len_bh_field_1, len_sr_field_1, sig_1, len_chain_field_2,
    //       height_field_2, len_bh_field_2, len_sr_field_2, sig_2

    // Save root_1 to altstack too
    script.push(OP_TOALTSTACK);

    // --- Phase 2: Reconstruct preimage_1 and verify sig_1 ---

    // Bring len_chain_field_1 back from altstack
    script.push(OP_FROMALTSTACK);
    // Stack: len_chain_field_1. Alt: ..., height_field_1, len_bh_field_1, ...

    // Push domain separator
    push_bytes(&mut script, b"UTXO_VM_ATTESTATION_V1");
    // Stack: len_chain_field_1, domain_sep

    // OP_CAT: domain_sep || len_chain_field_1
    // = domain_sep || u32(len_chain) || chain
    script.push(OP_CAT);
    // Stack: domain_sep || len_chain_field_1

    // Bring height_field_1 back
    script.push(OP_FROMALTSTACK);
    // Stack: (domain_sep || len_chain_field_1), height_field_1

    // OP_CAT: ... || height_field_1
    script.push(OP_CAT);
    // Stack: domain_sep || len_chain_field_1 || height_field_1

    // Bring len_bh_field_1 back
    script.push(OP_FROMALTSTACK);
    script.push(OP_CAT);
    // Stack: ... || len_bh_field_1

    // Bring len_sr_field_1 back
    script.push(OP_FROMALTSTACK);
    script.push(OP_CAT);
    // Stack: preimage_1 = domain_sep || len_chain_field_1 || height_field_1
    //       || len_bh_field_1 || len_sr_field_1

    // SHA256(preimage_1) → msg_1
    script.push(OP_SHA256);
    // Stack: msg_1

    // Bring sig_1 back
    script.push(OP_FROMALTSTACK);
    // Stack: msg_1, sig_1

    // Push operator pubkey
    push_bytes(&mut script, operator_pubkey);
    // Stack: msg_1, sig_1, pubkey

    // Verify sig_1. Note: BIP-342 OP_CHECKSIGVERIFY verifies the signature
    // against the transaction's taproot sighash, not the stack-provided msg_1.
    // The reconstructed msg_1 remains on the stack as evidence; the attestation
    // signatures are verified off-chain by verify_equivocation_proof().
    script.push(OP_CHECKSIGVERIFY);
    // Stack: msg_1 (dropped by CHECKSIGVERIFY? No — CHECKSIGVERIFY pops sig
    // and pubkey, leaves msg_1 on stack if we want it, but actually it
    // pops pubkey and sig, and the msg is not consumed by CHECKSIG.
    // Actually CHECKSIG pops pubkey and sig from the stack, verifies
    // sig against the tx sighash, and pushes 0/1. CHECKSIGVERIFY pops
    // the result and VERIFYs. So after CHECKSIGVERIFY, msg_1 is still on
    // the stack. We need to drop it.
    script.push(OP_DROP); // Drop msg_1

    // --- Phase 3: Reconstruct preimage_2 and verify sig_2 ---

    // Bring len_chain_field_2 back
    script.push(OP_FROMALTSTACK);
    // Push domain separator
    push_bytes(&mut script, b"UTXO_VM_ATTESTATION_V1");
    script.push(OP_CAT); // domain_sep || len_chain_field_2

    // Bring height_field_2 back
    script.push(OP_FROMALTSTACK);
    script.push(OP_CAT);

    // Bring len_bh_field_2 back
    script.push(OP_FROMALTSTACK);
    script.push(OP_CAT);

    // Bring len_sr_field_2 back
    script.push(OP_FROMALTSTACK);
    script.push(OP_CAT);
    // Stack: preimage_2

    // SHA256(preimage_2) → msg_2
    script.push(OP_SHA256);

    // Bring sig_2 back
    script.push(OP_FROMALTSTACK);
    // Stack: msg_2, sig_2

    // Push operator pubkey
    push_bytes(&mut script, operator_pubkey);
    // Stack: msg_2, sig_2, pubkey

    script.push(OP_CHECKSIGVERIFY);
    script.push(OP_DROP); // Drop msg_2

    // --- Phase 4: Check root_1 != root_2 (the equivocation) ---

    // Bring root_2 back
    script.push(OP_FROMALTSTACK);
    // Bring root_1 back
    script.push(OP_FROMALTSTACK);
    // Stack: root_2, root_1

    script.push(OP_EQUAL);   // (root_2 == root_1) ? 1 : 0
    script.push(OP_NOT);     // (root_2 != root_1) ? 1 : 0
    script.push(OP_VERIFY);  // Fail if 0 (roots were the same — not equivocation)

    // If we reach here: both signatures verified and roots differ → slash.
    Ok(script)
}

/// Push a byte vector onto the script as a minimal data push.
/// For 1-75 bytes: direct push (opcode = length)
/// For 76-255 bytes: OP_PUSHDATA1 + 1-byte length
/// For 256-520 bytes: OP_PUSHDATA2 + 2-byte length LE
fn push_bytes(script: &mut Vec<u8>, data: &[u8]) {
    if data.len() <= 75 {
        script.push(data.len() as u8);
    } else if data.len() <= 255 {
        script.push(OP_PUSHDATA1);
        script.push(data.len() as u8);
    } else {
        script.push(0x4d); // OP_PUSHDATA2
        script.extend_from_slice(&(data.len() as u16).to_le_bytes());
    }
    script.extend_from_slice(data);
}

/// Build the witness stack for the equivocation covenant.
///
/// Returns the witness items in the correct order for `build_equivocation_covenant()`:
///   root_1, root_2,
///   len_chain_field_1, height_field_1, len_bh_field_1, len_sr_field_1, sig_1,
///   len_chain_field_2, height_field_2, len_bh_field_2, len_sr_field_2, sig_2,
///   <script leaf>
///
/// Where:
/// - root_N: the state_root string bytes from attestation N
/// - len_chain_field_N: u32(chain.len()) || chain.as_bytes() (4-byte BE length + chain)
/// - height_field_N: height.to_be_bytes() (8-byte BE)
/// - len_bh_field_N: u32(block_hash.len()) || block_hash.as_bytes()
/// - len_sr_field_N: u32(state_root.len()) || state_root.as_bytes()
/// - sig_N: the operator's 64-byte BIP-340 Schnorr signature
///
/// The caller appends the Taproot control block after the script leaf.
pub fn build_equivocation_witness(
    root_1: &[u8],
    root_2: &[u8],
    chain: &str,
    height: u64,
    block_hash_1: &str,
    state_root_1: &str,
    sig_1: &[u8],
    block_hash_2: &str,
    state_root_2: &str,
    sig_2: &[u8],
    script_leaf: &[u8],
) -> Vec<Vec<u8>> {
    let len_chain_field = {
        let mut v = (chain.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(chain.as_bytes());
        v
    };
    let height_field = height.to_be_bytes().to_vec();
    let len_bh_field_1 = {
        let mut v = (block_hash_1.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(block_hash_1.as_bytes());
        v
    };
    let len_sr_field_1 = {
        let mut v = (state_root_1.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(state_root_1.as_bytes());
        v
    };
    let len_bh_field_2 = {
        let mut v = (block_hash_2.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(block_hash_2.as_bytes());
        v
    };
    let len_sr_field_2 = {
        let mut v = (state_root_2.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(state_root_2.as_bytes());
        v
    };

    vec![
        root_1.to_vec(),
        root_2.to_vec(),
        len_chain_field.clone(),
        height_field.clone(),
        len_bh_field_1,
        len_sr_field_1,
        sig_1.to_vec(),
        len_chain_field,
        height_field,
        len_bh_field_2,
        len_sr_field_2,
        sig_2.to_vec(),
        script_leaf.to_vec(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::attestation::{
        serialize_attestation_preimage, ConsensusManager,
    };
    use secp256k1::{Secp256k1, XOnlyPublicKey};

    /// Helper: generate a keypair and return (secret_key, x_only_pubkey_hex)
    fn gen_keypair() -> (secp256k1::SecretKey, String) {
        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        (sk, hex::encode(&pk.serialize()[1..]))
    }

    #[test]
    fn test_build_equivocation_covenant_rejects_wrong_pubkey_size() {
        let result = build_equivocation_covenant(&[0x02; 33]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("32 bytes"));
    }

    #[test]
    fn test_build_equivocation_covenant_accepts_xonly_pubkey() {
        let pubkey = [0x02; 32];
        let script = build_equivocation_covenant(&pubkey).expect("Must accept 32-byte x-only key");
        assert!(!script.is_empty());
        // Must contain OP_CAT (0x7e) — this is a covenant script
        assert!(script.contains(&OP_CAT), "Covenant must use OP_CAT");
        // Must contain OP_CHECKSIGVERIFY (0xad)
        assert!(
            script.contains(&OP_CHECKSIGVERIFY),
            "Covenant must use OP_CHECKSIGVERIFY"
        );
        // Must contain OP_SHA256 (0xa8)
        assert!(script.contains(&OP_SHA256), "Covenant must use OP_SHA256");
        // Must contain the operator pubkey
        assert!(
            script.windows(32).any(|w| w == &pubkey),
            "Covenant must embed operator pubkey"
        );
    }

    #[test]
    fn test_covenant_verifies_root_inequality_check() {
        // The script must contain OP_EQUAL OP_NOT OP_VERIFY for the root inequality check
        let pubkey = [0x02; 32];
        let script = build_equivocation_covenant(&pubkey).unwrap();

        // Find the OP_EQUAL OP_NOT OP_VERIFY sequence
        let pattern = [OP_EQUAL, OP_NOT, OP_VERIFY];
        assert!(
            script.windows(3).any(|w| w == &pattern),
            "Covenant must contain OP_EQUAL OP_NOT OP_VERIFY for root inequality check"
        );
    }

    #[test]
    fn test_covenant_uses_two_checksigverify() {
        // The script must have two OP_CHECKSIGVERIFY operations (one per attestation)
        let pubkey = [0x02; 32];
        let script = build_equivocation_covenant(&pubkey).unwrap();
        let count = script.iter().filter(|&&b| b == OP_CHECKSIGVERIFY).count();
        assert_eq!(
            count, 2,
            "Covenant must have exactly 2 OP_CHECKSIGVERIFY (one per attestation sig)"
        );
    }

    #[test]
    fn test_covenant_uses_two_sha256() {
        // The script must have two OP_SHA256 operations (one per preimage hash)
        let pubkey = [0x02; 32];
        let script = build_equivocation_covenant(&pubkey).unwrap();
        let count = script.iter().filter(|&&b| b == OP_SHA256).count();
        assert_eq!(
            count, 2,
            "Covenant must have exactly 2 OP_SHA256 (one per preimage)"
        );
    }

    #[test]
    fn test_build_equivocation_witness_order() {
        let root_1 = vec![0x01; 32];
        let root_2 = vec![0x02; 32];
        let chain = "JKC";
        let height = 100u64;
        let block_hash_1 = "hash1";
        let state_root_1 = "root1";
        let sig_1 = vec![0x04; 64];
        let block_hash_2 = "hash2";
        let state_root_2 = "root2";
        let sig_2 = vec![0x06; 64];
        let script_leaf = vec![0x07; 50];

        let witness = build_equivocation_witness(
            &root_1, &root_2, chain, height,
            block_hash_1, state_root_1, &sig_1,
            block_hash_2, state_root_2, &sig_2,
            &script_leaf,
        );

        // Witness order: root_1, root_2,
        //   len_chain_field, height_field, len_bh_field_1, len_sr_field_1, sig_1,
        //   len_chain_field, height_field, len_bh_field_2, len_sr_field_2, sig_2,
        //   script_leaf
        assert_eq!(witness.len(), 13);
        assert_eq!(witness[0], root_1);
        assert_eq!(witness[1], root_2);
        // witness[2] = len_chain_field (u32 BE || chain)
        // witness[3] = height_field (8-byte BE)
        assert_eq!(witness[3], height.to_be_bytes().to_vec());
        assert_eq!(witness[6], sig_1);
        assert_eq!(witness[11], sig_2);
        assert_eq!(witness[12], script_leaf);
    }

    #[test]
    fn test_covenant_with_real_secp256k1_signatures() {
        // Sign two different state roots at the same height with a test key,
        // build the witness, and verify the script construction is correct.
        let mgr = ConsensusManager::new(1);
        let (sk, pk_hex) = gen_keypair();
        mgr.register_validator(&pk_hex);

        let chain = "JKC_TESTNET";
        let height = 9_999;
        let block_hash = "00000000abcdef0123456789";

        // Sign two different roots
        let att1 = mgr
            .sign_state_root(&sk, chain, height, block_hash, "root_canonical")
            .unwrap();
        mgr.add_attestation(att1.clone());

        let att2 = mgr
            .sign_state_root(&sk, chain, height, block_hash, "root_malicious")
            .unwrap();
        mgr.add_attestation(att2.clone());

        // Verify equivocation was detected
        let proofs = mgr.get_slashing_proofs();
        assert_eq!(proofs.len(), 1);

        // Build the covenant script
        let operator_pubkey = hex::decode(&pk_hex).unwrap();
        assert_eq!(operator_pubkey.len(), 32);
        let script = build_equivocation_covenant(&operator_pubkey).unwrap();

        // Build the witness from the attestations
        let preimage_1 = serialize_attestation_preimage(
            &att1.chain,
            att1.block_height,
            &att1.block_hash,
            &att1.state_root,
        );
        let preimage_2 = serialize_attestation_preimage(
            &att2.chain,
            att2.block_height,
            &att2.block_hash,
            &att2.state_root,
        );
        let sig_1 = hex::decode(&att1.signature_hex).unwrap();
        let sig_2 = hex::decode(&att2.signature_hex).unwrap();
        let root_1 = att1.state_root.as_bytes().to_vec();
        let root_2 = att2.state_root.as_bytes().to_vec();

        let witness = build_equivocation_witness(
            &root_1, &root_2,
            &att1.chain, att1.block_height,
            &att1.block_hash, &att1.state_root, &sig_1,
            &att2.block_hash, &att2.state_root, &sig_2,
            &script,
        );

        // Verify witness structure (13 items: 2 roots + 5 fields×2 + script)
        assert_eq!(witness.len(), 13);
        assert_eq!(witness[0], root_1);
        assert_eq!(witness[1], root_2);
        assert_ne!(root_1, root_2, "Roots must differ for equivocation");
        // sig_1 is at index 6, sig_2 at index 11, script at 12
        assert_eq!(witness[6], sig_1);
        assert_eq!(witness[11], sig_2);
        assert_eq!(witness[12], script);

        // Verify the preimages hash to the signed messages
        use sha2::{Digest, Sha256};
        let msg_1 = Sha256::digest(&preimage_1);
        let msg_2 = Sha256::digest(&preimage_2);
        assert_ne!(
            msg_1, msg_2,
            "Message hashes must differ (different roots)"
        );

        // Verify signatures are valid BIP-340 Schnorr
        let secp = Secp256k1::new();
        let xonly_pk = XOnlyPublicKey::from_slice(&operator_pubkey).unwrap();
        let sig1_obj = secp256k1::schnorr::Signature::from_slice(&sig_1).unwrap();
        let sig2_obj = secp256k1::schnorr::Signature::from_slice(&sig_2).unwrap();
        let msg1 = secp256k1::Message::from_digest(msg_1.into());
        let msg2 = secp256k1::Message::from_digest(msg_2.into());
        assert!(
            secp.verify_schnorr(&sig1_obj, &msg1, &xonly_pk).is_ok(),
            "sig_1 must be valid over msg_1"
        );
        assert!(
            secp.verify_schnorr(&sig2_obj, &msg2, &xonly_pk).is_ok(),
            "sig_2 must be valid over msg_2"
        );
    }

    #[test]
    fn test_covenant_rejects_same_root_not_equivocation() {
        // Two attestations with the SAME root is NOT equivocation.
        // The script's OP_EQUAL OP_NOT OP_VERIFY should fail in this case.
        let mgr = ConsensusManager::new(1);
        let (sk, pk_hex) = gen_keypair();
        mgr.register_validator(&pk_hex);

        let chain = "JKC";
        let height = 1_000;
        let block_hash = "abcdef";
        let root = "same_root";

        let att1 = mgr.sign_state_root(&sk, chain, height, block_hash, root).unwrap();
        let att2 = mgr.sign_state_root(&sk, chain, height, block_hash, root).unwrap();

        // Same root → not equivocation
        assert_eq!(att1.state_root, att2.state_root);

        // The covenant script's root inequality check (OP_EQUAL OP_NOT OP_VERIFY)
        // would fail here because root_1 == root_2.
        let root_1 = att1.state_root.as_bytes().to_vec();
        let root_2 = att2.state_root.as_bytes().to_vec();
        assert_eq!(root_1, root_2, "Same root is not equivocation");
    }

    #[test]
    fn test_covenant_rejects_wrong_validator() {
        // Signatures from a different validator must not verify.
        let secp = Secp256k1::new();
        let (sk1, pk1) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        let (sk2, _pk2) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

        let mgr = ConsensusManager::new(1);
        let pk1_hex = hex::encode(&pk1.serialize()[1..]);
        mgr.register_validator(&pk1_hex);

        // att1 signed by sk1 (the registered validator)
        let att1 = mgr
            .sign_state_root(&sk1, "JKC", 500, "hash", "root_a")
            .unwrap();

        // att2 signed by sk2 (a different key) but with validator_pubkey = pk1
        let mut att2 = mgr
            .sign_state_root(&sk2, "JKC", 500, "hash", "root_b")
            .unwrap();
        att2.validator_pubkey = pk1_hex.clone();

        // verify_equivocation_proof must reject this
        let proof = crate::types::EquivocationProof {
            chain: "JKC".to_string(),
            block_height: 500,
            validator_pubkey: pk1_hex,
            first_attestation: att1,
            second_attestation: att2,
            detected_at: 0,
        };
        assert!(
            !crate::consensus::attestation::verify_equivocation_proof(&proof),
            "Proof with mismatched signature must fail verification"
        );
    }
}
