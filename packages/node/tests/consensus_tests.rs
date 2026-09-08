use secp256k1::Secp256k1;
use utxo_vmd::consensus::ConsensusManager;

#[test]
fn test_consensus_quorum_threshold() {
    let secp = Secp256k1::new();
    let (sk1, pk1) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let (sk2, pk2) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let (sk3, pk3) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    // Require 2-of-3 quorum
    let consensus = ConsensusManager::new(2);
    consensus.register_validator(&hex::encode(pk1.serialize()));
    consensus.register_validator(&hex::encode(pk2.serialize()));
    consensus.register_validator(&hex::encode(pk3.serialize()));

    let chain = "JKC";
    let height = 1000;
    let block_hash = "00000000abc123";
    let state_root = "root_hash_999";

    // 1st validator signs
    let att1 = consensus
        .sign_state_root(&sk1, chain, height, block_hash, state_root)
        .expect("Signing failed");
    assert!(consensus.add_attestation(att1));
    // Quorum not reached yet (1 < 2)
    assert_eq!(consensus.is_quorum_reached(chain, height, state_root), false);

    // 2nd validator signs
    let att2 = consensus
        .sign_state_root(&sk2, chain, height, block_hash, state_root)
        .expect("Signing failed");
    assert!(consensus.add_attestation(att2));
    // Quorum reached! (2 >= 2)
    assert_eq!(consensus.is_quorum_reached(chain, height, state_root), true);

    // Conflicting root from validator 3
    let att3_bad = consensus
        .sign_state_root(&sk3, chain, height, block_hash, "evil_root_666")
        .expect("Signing failed");
    assert!(consensus.add_attestation(att3_bad));
    assert_eq!(consensus.is_quorum_reached(chain, height, "evil_root_666"), false);
}

#[test]
fn test_equivocation_slashing_proof_capture() {
    let secp = Secp256k1::new();
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    let consensus = ConsensusManager::new(1);
    consensus.register_validator(&hex::encode(pk.serialize()));

    let chain = "DOGE";
    let height = 250;
    let block_hash = "00000000dogehash";

    // Legitimate signature
    let att_good = consensus
        .sign_state_root(&sk, chain, height, block_hash, "canonical_state_root")
        .unwrap();
    assert!(consensus.add_attestation(att_good));

    // Equivocating signature (same height, different state root)
    let att_equivocating = consensus
        .sign_state_root(&sk, chain, height, block_hash, "fork_state_root_evil")
        .unwrap();
    assert_eq!(consensus.add_attestation(att_equivocating), false);

    // Verify slashing proof was automatically registered
    let proofs = consensus.get_slashing_proofs();
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0].validator_pubkey, hex::encode(pk.serialize()));
    assert_eq!(proofs[0].first_attestation.state_root, "canonical_state_root");
    assert_eq!(proofs[0].second_attestation.state_root, "fork_state_root_evil");

    // Cryptographically verify the equivocation proof
    assert!(utxo_vmd::consensus::covenants::verify_equivocation_proof(&proofs[0]));
}

