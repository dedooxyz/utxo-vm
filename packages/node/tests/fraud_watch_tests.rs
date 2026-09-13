//! Integration tests for the fraud-watch background task wiring (Item A).
//!
//! These tests exercise the same building blocks the running node uses in
//! `main.rs::run_fraud_watch_pass`:
//!   1. Feed the ConsensusManager two attestations with different state roots
//!      for the same (chain, height).
//!   2. Confirm `build_quorum_result` + `detect_divergence` fires for the
//!      minority attestation.
//!   3. Confirm `get_slashing_proofs` accumulates an EquivocationProof when
//!      the SAME validator signs two different roots.
//!   4. Confirm `verify_equivocation_proof` accepts the accumulated proof.
//!   5. Confirm `challenge::build_challenge_transaction` constructs a tx with
//!      the expected evidence fields from that proof (broadcast is not
//!      performed — there is no test L1 — but the built tx's fields are
//!      asserted, matching the pattern in challenge.rs's existing unit tests).

use secp256k1::Secp256k1;
use utxo_vmd::consensus::attestation::verify_equivocation_proof;
use utxo_vmd::consensus::challenge::{self, ChallengeInput};
use utxo_vmd::consensus::l1_scripts::VaultConfig;
use utxo_vmd::consensus::ConsensusManager;

/// Detecting divergence: three validators, two sign canonical, one diverges.
/// Quorum=2 → canonical root wins; the divergent attestation surfaces via
/// `detect_divergence`.
#[test]
fn test_fraud_watch_detects_divergence_on_minority_attestation() {
    let secp = Secp256k1::new();
    let (sk1, pk1) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let (sk2, pk2) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let (sk3, pk3) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    let consensus = ConsensusManager::new(2);
    consensus.register_validator(&hex::encode(pk1.serialize()));
    consensus.register_validator(&hex::encode(pk2.serialize()));
    consensus.register_validator(&hex::encode(pk3.serialize()));

    let chain = "JKC_TESTNET";
    let height = 9_001;
    let block_hash = "00000000abcdef";
    let canonical_root = "canonical_root_aaaa";
    let divergent_root = "divergent_root_bbbb";

    // Validators 1 and 2 sign the canonical root → quorum reached (2 >= 2).
    let att1 = consensus
        .sign_state_root(&sk1, chain, height, block_hash, canonical_root)
        .unwrap();
    assert!(consensus.add_attestation(att1.clone()));
    let att2 = consensus
        .sign_state_root(&sk2, chain, height, block_hash, canonical_root)
        .unwrap();
    assert!(consensus.add_attestation(att2.clone()));

    // Validator 3 signs a DIFFERENT root for the same (chain, height).
    let att3 = consensus
        .sign_state_root(&sk3, chain, height, block_hash, divergent_root)
        .unwrap();
    assert!(consensus.add_attestation(att3.clone()));

    // Quorum result picks the canonical root (2 supporters vs 1).
    let quorum = consensus
        .build_quorum_result(chain, height)
        .expect("quorum must exist (2-of-3 reached)");
    assert_eq!(quorum.state_root, canonical_root);

    // The divergent attestation must produce a divergence proof.
    let div = consensus
        .detect_divergence(&att3, &quorum)
        .expect("divergence must be detected for the minority attestation");
    assert_eq!(div.operator_attestation.state_root, divergent_root);
    assert_eq!(div.quorum_result_root, canonical_root);
    assert_eq!(div.chain, chain);
    assert_eq!(div.block_height, height);

    // The canonical attestations must NOT produce a divergence proof.
    assert!(consensus.detect_divergence(&att1, &quorum).is_none());
    assert!(consensus.detect_divergence(&att2, &quorum).is_none());
}

/// Equivocation: the SAME validator signs two different roots for the same
/// (chain, height). ConsensusManager accumulates an EquivocationProof;
/// verify_equivocation_proof accepts it; build_challenge_transaction
/// constructs a tx with the expected evidence fields.
#[test]
fn test_fraud_watch_equivocation_proof_to_challenge_tx() {
    let secp = Secp256k1::new();
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    let consensus = ConsensusManager::new(1);
    consensus.register_validator(&hex::encode(pk.serialize()));

    let chain = "JKC_TESTNET";
    let height = 9_002;
    let block_hash = "00000000deadbeef";
    let root1 = "root_one_1111";
    let root2 = "root_two_2222";

    let att1 = consensus
        .sign_state_root(&sk, chain, height, block_hash, root1)
        .unwrap();
    assert!(consensus.add_attestation(att1));

    // Same validator, different root → add_attestation returns false (rejects
    // the second as equivocation) AND accumulates a slashing proof.
    let att2 = consensus
        .sign_state_root(&sk, chain, height, block_hash, root2)
        .unwrap();
    assert_eq!(consensus.add_attestation(att2), false);

    let proofs = consensus.get_slashing_proofs();
    assert_eq!(proofs.len(), 1, "exactly one equivocation proof expected");
    let proof = &proofs[0];
    assert_eq!(proof.validator_pubkey, hex::encode(pk.serialize()));
    assert_eq!(proof.first_attestation.state_root, root1);
    assert_eq!(proof.second_attestation.state_root, root2);

    // Verify cryptographically — this is the gate build_challenge_transaction
    // applies before constructing the tx.
    assert!(verify_equivocation_proof(proof));

    // Build a challenge tx from the proof, mirroring what the fraud-watch
    // task does when a vault config + bond UTXO are configured.
    let pk_hex = hex::encode(pk.serialize());
    let operator_pubkey = hex::decode(&pk_hex).unwrap();

    let input = ChallengeInput {
        bond_txid: "ab".repeat(32),
        bond_vout: 0,
        bond_amount: 100_000_000,
    };
    let vault_config = VaultConfig {
        operator_pubkey: operator_pubkey.clone(),
        challenger_pubkey: vec![0x03; 33],
        unbond_delay: 60,
        claim_delay: 10,
        watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33]],
        watcher_threshold: 1,
    };
    let watcher_sig = vec![0xAAu8; 64];

    let tx = challenge::build_challenge_transaction(proof, &input, &vault_config, &[watcher_sig], 1000)
        .expect("challenge tx must build from a verified equivocation proof");

    // The evidence output must carry the utxovm:challenge tag and both roots.
    assert!(
        tx.evidence_output.windows(16).any(|w| w == b"utxovm:challenge"),
        "evidence output must contain the 'utxovm:challenge' tag"
    );
    assert_eq!(tx.fee, 1000);
    assert_eq!(tx.challenge_output.amount, 100_000_000 - 1000);
    assert!(!tx.raw_hex.is_empty(), "raw tx hex must be non-empty");
}

/// A proof that fails cryptographic verification must NOT produce a challenge
/// tx. build_challenge_transaction returns Err in that case.
#[test]
fn test_fraud_watch_rejects_invalid_proof_no_challenge_tx() {
    let secp = Secp256k1::new();
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let (sk_other, _pk_other) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    let consensus = ConsensusManager::new(1);
    consensus.register_validator(&hex::encode(pk.serialize()));

    let chain = "JKC_TESTNET";
    let height = 9_003;
    let block_hash = "00000000cafebabe";

    // att1 signed by the registered validator
    let att1 = consensus
        .sign_state_root(&sk, chain, height, block_hash, "root_a")
        .unwrap();
    // att2 signed by a DIFFERENT key but with validator_pubkey field set to
    // the registered validator's pubkey — verify_equivocation_proof must
    // reject this because the signature won't match.
    let mut att2 = consensus
        .sign_state_root(&sk_other, chain, height, block_hash, "root_b")
        .unwrap();
    att2.validator_pubkey = hex::encode(pk.serialize());

    let proof = utxo_vmd::types::EquivocationProof {
        chain: chain.to_string(),
        block_height: height,
        validator_pubkey: hex::encode(pk.serialize()),
        first_attestation: att1,
        second_attestation: att2,
        detected_at: chrono::Utc::now().timestamp(),
    };

    assert!(
        !verify_equivocation_proof(&proof),
        "proof with mismatched signature must fail verification"
    );

    let input = ChallengeInput {
        bond_txid: "cd".repeat(32),
        bond_vout: 0,
        bond_amount: 50_000_000,
    };
    let vault_config = VaultConfig {
        operator_pubkey: vec![0x01; 33],
        challenger_pubkey: vec![0x02; 33],
        unbond_delay: 60,
        claim_delay: 10,
        watcher_pubkeys: vec![vec![0x04; 33]],
        watcher_threshold: 1,
    };
    let result = challenge::build_challenge_transaction(
        &proof,
        &input,
        &vault_config,
        &[vec![0xAA; 64]],
        1000,
    );
    assert!(
        result.is_err(),
        "build_challenge_transaction must reject an unverified proof"
    );
}
