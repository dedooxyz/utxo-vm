//! On-Chain Challenge Spend (Model B)
//!
//! Builds and broadcasts a challenge transaction that spends an operator's
//! bond UTXO via the committee challenge tapleaf when an EquivocationProof
//! is detected. The challenge tx creates a second-stage challenge UTXO
//! with claim (CSV) and rebut (immediate) paths.
//!
//! Flow:
//! 1. `build_challenge_transaction()` — constructs the raw tx hex from an
//!    EquivocationProof, the operator's vault config, and M-of-N watcher
//!    signatures. The tx spends the bond UTXO via the challenge leaf and
//!    creates a challenge UTXO (P2TR with claim/rebut tree) + an OP_RETURN
//!    evidence output.
//! 2. `broadcast_challenge_tx()` — submits the raw tx hex to electrs.
//! 3. `submit_challenge()` — end-to-end: verify proof → build tx → broadcast.
//!
//! Security: the watcher committee (M-of-N) must independently verify the
//! equivocation off-chain before co-signing. Bitcoin Script cannot verify
//! off-chain WASM execution. See docs/COURT.md and docs/TRUST-MODEL.tex.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::consensus::covenants;
use crate::consensus::l1_scripts::{
    self, ChallengeScriptTree, ChallengeUtxoConfig, VaultConfig,
};
use crate::scanner::electrs::ElectrsClient;
use crate::types::EquivocationProof;

/// Challenge transaction input (bond UTXO being spent)
#[derive(Debug, Clone)]
pub struct ChallengeInput {
    /// Bond UTXO txid (big-endian hex)
    pub bond_txid: String,
    /// Bond UTXO vout
    pub bond_vout: u32,
    /// Bond amount in satoshis
    pub bond_amount: u64,
}

/// Challenge transaction output (second-stage challenge UTXO)
#[derive(Debug, Clone)]
pub struct ChallengeOutput {
    /// P2TR output script for the challenge UTXO (claim/rebut tree)
    pub script_pubkey: Vec<u8>,
    /// Amount (bond_amount minus miner fee)
    pub amount: u64,
}

/// OP_RETURN evidence payload embedded in the challenge tx
#[derive(Debug, Clone)]
pub struct ChallengeEvidence {
    /// Operator's claimed (wrong) root — 32 bytes
    pub claimed_root: Vec<u8>,
    /// Correct root (from quorum) — 32 bytes
    pub correct_root: Vec<u8>,
    /// Operator pubkey (33 bytes compressed)
    pub operator_pubkey: Vec<u8>,
    /// Chain name
    pub chain: String,
    /// Block height
    pub block_height: u64,
}

/// Built challenge transaction (raw hex + metadata)
#[derive(Debug, Clone)]
pub struct ChallengeTransaction {
    /// Raw transaction hex (unsigned or partially signed — watcher sigs
    /// are embedded in the witness but the tx is not finalized until
    /// the wallet adds the Taproot control block)
    pub raw_hex: String,
    /// Challenge UTXO output (for tracking)
    pub challenge_output: ChallengeOutput,
    /// Evidence OP_RETURN output
    pub evidence_output: Vec<u8>,
    /// Total fee paid to miners (satoshis)
    pub fee: u64,
}

/// Build the OP_RETURN evidence output for a challenge tx.
///
/// Format:
/// ```text
/// OP_RETURN "utxovm:challenge" <claimed_root> <correct_root> <operator_pubkey> <chain> <height>
/// ```
fn build_evidence_output(evidence: &ChallengeEvidence) -> Result<Vec<u8>> {
    if evidence.claimed_root.len() != 32 {
        return Err(anyhow!("claimed_root must be 32 bytes"));
    }
    if evidence.correct_root.len() != 32 {
        return Err(anyhow!("correct_root must be 32 bytes"));
    }
    if evidence.operator_pubkey.len() != 33 {
        return Err(anyhow!("operator_pubkey must be 33 bytes (compressed)"));
    }

    let mut script = Vec::new();
    script.push(l1_scripts::OP_RETURN); // 0x6a

    // Protocol tag
    let tag = b"utxovm:challenge";
    script.push(tag.len() as u8);
    script.extend_from_slice(tag);

    // Claimed root (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&evidence.claimed_root);

    // Correct root (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&evidence.correct_root);

    // Operator pubkey (33 bytes)
    script.push(0x21);
    script.extend_from_slice(&evidence.operator_pubkey);

    // Chain name (variable, max 64 bytes)
    let chain_bytes = evidence.chain.as_bytes();
    if chain_bytes.len() > 64 {
        return Err(anyhow!("chain name too long (max 64 bytes)"));
    }
    script.push(chain_bytes.len() as u8);
    script.extend_from_slice(chain_bytes);

    // Block height (8 bytes LE)
    script.push(0x08);
    script.extend_from_slice(&evidence.block_height.to_le_bytes());

    Ok(script)
}

/// Build a challenge transaction from an EquivocationProof.
///
/// This constructs a raw Bitcoin transaction that:
/// 1. Spends the operator's bond UTXO via the committee challenge tapleaf
///    (witness: dummy + M watcher signatures + challenge leaf script +
///    control block — control block added by the signing wallet).
/// 2. Creates a challenge UTXO (P2TR with claim/rebut tree) for the bond
///    amount minus miner fee.
/// 3. Creates an OP_RETURN evidence output with the equivocation proof data.
///
/// The caller must provide:
/// - `proof`: the EquivocationProof from ConsensusManager
/// - `input`: the bond UTXO reference (txid, vout, amount)
/// - `vault_config`: the operator's vault configuration (pubkeys, delays)
/// - `watcher_signatures`: M-of-N ECDSA signatures from the watcher committee
///
/// Returns the raw tx hex + metadata. The tx is NOT fully signed — the
/// Taproot control block (internal key + Merkle path) must be appended by
/// a secp256k1-capable wallet before broadcast.
pub fn build_challenge_transaction(
    proof: &EquivocationProof,
    input: &ChallengeInput,
    vault_config: &VaultConfig,
    watcher_signatures: &[Vec<u8>],
    miner_fee: u64,
) -> Result<ChallengeTransaction> {
    // 1. Verify the equivocation proof cryptographically
    if !covenants::verify_equivocation_proof(proof) {
        return Err(anyhow!(
            "Equivocation proof failed cryptographic verification — cannot build challenge tx"
        ));
    }

    // 2. Check watcher signature threshold
    if watcher_signatures.len() < vault_config.watcher_threshold as usize {
        return Err(anyhow!(
            "Insufficient watcher signatures: got {}, need {} (M-of-N)",
            watcher_signatures.len(),
            vault_config.watcher_threshold
        ));
    }

    // 3. Build the vault script tree to get the challenge leaf
    let vault_tree = l1_scripts::build_vault_script_tree(vault_config)?;

    // 4. Build the challenge UTXO (second stage) script tree
    let challenge_config = ChallengeUtxoConfig {
        challenger_pubkey: vault_config.challenger_pubkey.clone(),
        operator_pubkey: vault_config.operator_pubkey.clone(),
        claim_delay: vault_config.claim_delay,
    };
    let challenge_tree = l1_scripts::build_challenge_claim_script_tree(&challenge_config)?;

    // 5. Build the challenge UTXO P2TR output script
    let challenge_script = challenge_tree_p2tr_script(&challenge_tree)?;

    // 6. Build the evidence OP_RETURN output
    let operator_pubkey = hex::decode(&proof.validator_pubkey)
        .map_err(|e| anyhow!("Invalid operator pubkey hex: {}", e))?;

    let claimed_root = decode_or_hash(&proof.first_attestation.state_root);
    let correct_root = decode_or_hash(&proof.second_attestation.state_root);

    // Roots could be hex strings that aren't 32 bytes; pad/truncate to 32
    let claimed_root_32 = pad_to_32(&claimed_root);
    let correct_root_32 = pad_to_32(&correct_root);

    let evidence = ChallengeEvidence {
        claimed_root: claimed_root_32,
        correct_root: correct_root_32,
        operator_pubkey,
        chain: proof.chain.clone(),
        block_height: proof.block_height,
    };
    let evidence_output = build_evidence_output(&evidence)?;

    // 7. Calculate challenge UTXO amount (bond - fee - dust for OP_RETURN)
    if input.bond_amount < miner_fee {
        return Err(anyhow!(
            "Bond amount ({}) < miner fee ({}) — nothing to challenge for",
            input.bond_amount,
            miner_fee
        ));
    }
    let challenge_amount = input.bond_amount - miner_fee;

    // 8. Build the witness for the challenge leaf (committee M-of-N)
    // Witness: [dummy, sig1, ..., sigM, challenge_leaf_script]
    // Control block (internal key + Merkle path) is appended by the wallet.
    let witness = vault_tree.committee_witness_template(watcher_signatures);

    // 9. Serialize the raw transaction (unsigned, with witness placeholder)
    let raw_hex = serialize_challenge_tx(
        input,
        &challenge_script,
        challenge_amount,
        &evidence_output,
        &witness,
    )?;

    Ok(ChallengeTransaction {
        raw_hex,
        challenge_output: ChallengeOutput {
            script_pubkey: challenge_script,
            amount: challenge_amount,
        },
        evidence_output,
        fee: miner_fee,
    })
}

/// Compute the P2TR output script for a challenge UTXO script tree.
///
/// This is a simplified version that uses the script tree root as the
/// tweaked key placeholder. A full implementation would use secp256k1
/// to compute Q = internal_key + lift_x(tweak) * G.
fn challenge_tree_p2tr_script(tree: &ChallengeScriptTree) -> Result<Vec<u8>> {
    let internal_key = [0u8; 32]; // NUMS placeholder
    let merkle_root = tree.script_tree_root();

    let mut tweak_data = Vec::with_capacity(64);
    tweak_data.extend_from_slice(&internal_key);
    tweak_data.extend_from_slice(&merkle_root);
    let tweak = l1_scripts::tagged_hash(l1_scripts::TAG_TAPTWEAK, &tweak_data);

    let mut script = Vec::with_capacity(34);
    script.push(0x51); // OP_1 (witness version 1)
    script.push(32);
    script.extend_from_slice(&tweak[..32]);

    Ok(script)
}

/// Decode a hex string to bytes, or hash it to 32 bytes if it's not valid hex.
fn decode_or_hash(s: &str) -> Vec<u8> {
    if let Ok(bytes) = hex::decode(s) {
        pad_to_32(&bytes)
    } else {
        // Not valid hex — hash the string to get a deterministic 32-byte root
        let hash = Sha256::digest(s.as_bytes());
        hash.to_vec()
    }
}

/// Pad or truncate a byte vector to 32 bytes (for root hashing).
fn pad_to_32(data: &[u8]) -> Vec<u8> {
    if data.len() == 32 {
        data.to_vec()
    } else if data.len() > 32 {
        data[..32].to_vec()
    } else {
        let mut out = vec![0u8; 32];
        out[..data.len()].copy_from_slice(data);
        out
    }
}

/// Serialize a challenge transaction to raw hex.
///
/// This builds a simplified Bitcoin tx structure:
/// - Version: 2
/// - 1 input (bond UTXO)
/// - 2 outputs (challenge UTXO + evidence OP_RETURN)
/// - Witness (committee signatures + challenge leaf)
///
/// NOTE: This is a simplified serialization for protocol-level testing.
/// A production implementation would use `bitcoin::Transaction` from the
/// `rust-bitcoin` crate for exact wire-format compliance. The witness here
/// is a placeholder — the actual Taproot control block must be appended
/// by a secp256k1-capable wallet.
fn serialize_challenge_tx(
    input: &ChallengeInput,
    challenge_script: &[u8],
    challenge_amount: u64,
    evidence_script: &[u8],
    witness: &[Vec<u8>],
) -> Result<String> {
    // Parse txid (big-endian hex → little-endian bytes)
    let txid_bytes = hex::decode(&input.bond_txid)
        .map_err(|e| anyhow!("Invalid bond txid hex: {}", e))?;
    if txid_bytes.len() != 32 {
        return Err(anyhow!("Bond txid must be 32 bytes, got {}", txid_bytes.len()));
    }
    let mut txid_le = vec![0u8; 32];
    for i in 0..32 {
        txid_le[i] = txid_bytes[31 - i]; // reverse to LE
    }

    let mut tx = Vec::new();

    // Version (4 bytes LE) — version 2
    tx.extend_from_slice(&2u32.to_le_bytes());

    // Input count (1 byte)
    tx.push(1);

    // Input: txid (32 bytes LE) + vout (4 bytes LE)
    tx.extend_from_slice(&txid_le);
    tx.extend_from_slice(&input.bond_vout.to_le_bytes());

    // ScriptSig length (1 byte = 0, empty for Taproot)
    tx.push(0);

    // Sequence (4 bytes LE) — 0xFFFFFFFD (RBF + non-locked)
    tx.extend_from_slice(&0xFFFFFFFDu32.to_le_bytes());

    // Output count (1 byte = 2)
    tx.push(2);

    // Output 1: Challenge UTXO
    tx.extend_from_slice(&challenge_amount.to_le_bytes());
    tx.push(challenge_script.len() as u8);
    tx.extend_from_slice(challenge_script);

    // Output 2: Evidence OP_RETURN (0 value)
    tx.extend_from_slice(&0u64.to_le_bytes());
    tx.push(evidence_script.len() as u8);
    tx.extend_from_slice(evidence_script);

    // Witness (for the 1 input)
    // Witness stack count
    tx.push(witness.len() as u8);
    for item in witness {
        tx.push(item.len() as u8);
        tx.extend_from_slice(item);
    }

    // Locktime (4 bytes LE) — 0
    tx.extend_from_slice(&0u32.to_le_bytes());

    Ok(hex::encode(&tx))
}

/// Broadcast a challenge transaction to L1 via electrs.
///
/// Submits the raw tx hex to the electrs `/tx` endpoint. Returns the
/// txid of the broadcast transaction on success.
pub async fn broadcast_challenge_tx(
    client: &ElectrsClient,
    raw_hex: &str,
) -> Result<String> {
    client.broadcast_tx(raw_hex).await
}

/// End-to-end challenge submission:
/// 1. Verify the equivocation proof
/// 2. Build the challenge transaction
/// 3. Broadcast to L1
///
/// Returns the broadcast txid.
pub async fn submit_challenge(
    client: &ElectrsClient,
    proof: &EquivocationProof,
    input: &ChallengeInput,
    vault_config: &VaultConfig,
    watcher_signatures: &[Vec<u8>],
    miner_fee: u64,
) -> Result<String> {
    let tx = build_challenge_transaction(proof, input, vault_config, watcher_signatures, miner_fee)?;
    broadcast_challenge_tx(client, &tx.raw_hex).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::attestation::ConsensusManager;
    use secp256k1::{Secp256k1, SecretKey};

    /// Helper: generate a keypair and return (secret_key, pubkey_hex)
    fn gen_keypair() -> (SecretKey, String) {
        let secp = Secp256k1::new();
        let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        (sk, hex::encode(pk.serialize()))
    }

    #[test]
    fn test_build_evidence_output() {
        let evidence = ChallengeEvidence {
            claimed_root: vec![0x01; 32],
            correct_root: vec![0x02; 32],
            operator_pubkey: vec![0x03; 33],
            chain: "JKC_TESTNET".to_string(),
            block_height: 12345,
        };
        let script = build_evidence_output(&evidence).expect("Failed to build evidence output");
        assert!(!script.is_empty());
        assert_eq!(script[0], l1_scripts::OP_RETURN);
        // Must contain the protocol tag
        assert!(script.windows(16).any(|w| w == b"utxovm:challenge"));
    }

    #[test]
    fn test_build_challenge_transaction_rejects_invalid_proof() {
        let proof = EquivocationProof {
            chain: "JKC".to_string(),
            block_height: 100,
            validator_pubkey: "02".repeat(33), // invalid pubkey
            first_attestation: crate::types::StateAttestation {
                chain: "JKC".to_string(),
                block_height: 100,
                block_hash: "hash".to_string(),
                state_root: "root1".to_string(),
                validator_pubkey: "02".repeat(33),
                signature_hex: "00".repeat(64),
                timestamp: 0,
            },
            second_attestation: crate::types::StateAttestation {
                chain: "JKC".to_string(),
                block_height: 100,
                block_hash: "hash".to_string(),
                state_root: "root2".to_string(),
                validator_pubkey: "02".repeat(33),
                signature_hex: "00".repeat(64),
                timestamp: 0,
            },
            detected_at: 0,
        };

        let input = ChallengeInput {
            bond_txid: "00".repeat(32),
            bond_vout: 0,
            bond_amount: 100_000,
        };

        let vault_config = VaultConfig {
            operator_pubkey: vec![0x02; 33],
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 60,
            claim_delay: 10,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33]],
            watcher_threshold: 2,
        };

        let sig = vec![0u8; 64];
        let result = build_challenge_transaction(&proof, &input, &vault_config, &[sig], 1000);
        assert!(result.is_err(), "Must reject invalid equivocation proof");
    }

    #[test]
    fn test_build_challenge_transaction_rejects_insufficient_watchers() {
        // Generate a real equivocation proof
        let mgr = ConsensusManager::new(1);
        let (sk, pk_hex) = gen_keypair();
        mgr.register_validator(&pk_hex);

        let att1 = mgr
            .sign_state_root(&sk, "JKC", 100, "hash", "root_a")
            .unwrap();
        mgr.add_attestation(att1);

        let att2 = mgr
            .sign_state_root(&sk, "JKC", 100, "hash", "root_b")
            .unwrap();
        mgr.add_attestation(att2);

        let proofs = mgr.get_slashing_proofs();
        assert_eq!(proofs.len(), 1);
        let proof = &proofs[0];

        let input = ChallengeInput {
            bond_txid: "ab".repeat(32),
            bond_vout: 0,
            bond_amount: 100_000,
        };

        let vault_config = VaultConfig {
            operator_pubkey: hex::decode(&pk_hex).unwrap(),
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 60,
            claim_delay: 10,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]],
            watcher_threshold: 2,
        };

        // Only 1 signature, but threshold is 2
        let sig = vec![0u8; 64];
        let result = build_challenge_transaction(proof, &input, &vault_config, &[sig], 1000);
        assert!(result.is_err(), "Must reject insufficient watcher signatures");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Insufficient watcher signatures"), "Got: {}", err);
    }

    #[test]
    fn test_build_challenge_transaction_success() {
        // Generate a real equivocation proof
        let mgr = ConsensusManager::new(1);
        let (sk, pk_hex) = gen_keypair();
        mgr.register_validator(&pk_hex);

        let att1 = mgr
            .sign_state_root(&sk, "JKC", 200, "block_hash", "root_canonical")
            .unwrap();
        mgr.add_attestation(att1);

        let att2 = mgr
            .sign_state_root(&sk, "JKC", 200, "block_hash", "root_malicious")
            .unwrap();
        mgr.add_attestation(att2);

        let proofs = mgr.get_slashing_proofs();
        assert_eq!(proofs.len(), 1);
        let proof = &proofs[0];

        let input = ChallengeInput {
            bond_txid: "cd".repeat(32),
            bond_vout: 0,
            bond_amount: 1_000_000, // 0.01 JKC
        };

        let vault_config = VaultConfig {
            operator_pubkey: hex::decode(&pk_hex).unwrap(),
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 60,
            claim_delay: 10,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]],
            watcher_threshold: 2,
        };

        // 2 watcher signatures (meets threshold of 2)
        let watcher_sigs = vec![vec![0xAA; 64], vec![0xBB; 64]];

        let result = build_challenge_transaction(proof, &input, &vault_config, &watcher_sigs, 1000);
        assert!(result.is_ok(), "Failed to build challenge tx: {:?}", result);

        let tx = result.unwrap();
        assert!(!tx.raw_hex.is_empty(), "Raw hex must not be empty");
        assert_eq!(tx.fee, 1000);
        assert_eq!(tx.challenge_output.amount, 1_000_000 - 1000);
        assert!(!tx.evidence_output.is_empty());
        assert_eq!(tx.evidence_output[0], l1_scripts::OP_RETURN);

        // Raw hex must be valid hex
        assert!(hex::decode(&tx.raw_hex).is_ok(), "Raw hex must be valid");

        // Raw hex must contain the tx version (2 LE = 02000000)
        assert!(tx.raw_hex.starts_with("02000000"), "Tx must start with version 2 LE");
    }

    #[test]
    fn test_build_challenge_transaction_rejects_bond_below_fee() {
        let mgr = ConsensusManager::new(1);
        let (sk, pk_hex) = gen_keypair();
        mgr.register_validator(&pk_hex);

        let att1 = mgr.sign_state_root(&sk, "JKC", 300, "h", "r1").unwrap();
        mgr.add_attestation(att1);
        let att2 = mgr.sign_state_root(&sk, "JKC", 300, "h", "r2").unwrap();
        mgr.add_attestation(att2);

        let proof = &mgr.get_slashing_proofs()[0];

        let input = ChallengeInput {
            bond_txid: "ef".repeat(32),
            bond_vout: 0,
            bond_amount: 500, // less than fee
        };

        let vault_config = VaultConfig {
            operator_pubkey: hex::decode(&pk_hex).unwrap(),
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 60,
            claim_delay: 10,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33]],
            watcher_threshold: 1,
        };

        let sig = vec![0u8; 64];
        let result = build_challenge_transaction(proof, &input, &vault_config, &[sig], 1000);
        assert!(result.is_err(), "Must reject when bond < fee");
        assert!(result.unwrap_err().to_string().contains("Bond amount"));
    }

    #[test]
    fn test_pad_to_32() {
        assert_eq!(pad_to_32(&[0x01; 32]).len(), 32);
        assert_eq!(pad_to_32(&[0x01; 16]).len(), 32);
        assert_eq!(pad_to_32(&[0x01; 64]).len(), 32);
        assert_eq!(pad_to_32(&[]).len(), 32);
    }
}
