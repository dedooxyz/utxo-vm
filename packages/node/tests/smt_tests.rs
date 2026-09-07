use utxo_vmd::storage::SparseMerkleTree;

#[test]
fn test_smt_deterministic_root() {
    let mut tree1 = SparseMerkleTree::new();
    let mut tree2 = SparseMerkleTree::new();

    let k1 = [1u8; 32];
    let v1 = [10u8; 32];
    let k2 = [2u8; 32];
    let v2 = [20u8; 32];

    tree1.update(k1, v1);
    tree1.update(k2, v2);

    // Insert in reverse order to ensure order-independent deterministic root
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
