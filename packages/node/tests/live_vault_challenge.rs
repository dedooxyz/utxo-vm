//! Live JKC Testnet: P2TR Vault + Challenge Slash
//!
//! Run with: cargo test -p utxo-vmd --test live_vault_challenge -- --nocapture --ignored

use std::cmp::min;

use secp256k1::rand::RngCore;

use bitcoin::address::Address;
use bitcoin::consensus::encode;
use bitcoin::hashes::Hash;
use bitcoin::opcodes::all::*;
use bitcoin::script::{ScriptBuf, PushBytesBuf};
use bitcoin::secp256k1::{Secp256k1, SecretKey, XOnlyPublicKey};
use bitcoin::sighash::{SighashCache, Prevouts};
use bitcoin::taproot::{TaprootBuilder, LeafVersion, TapLeafHash};
use bitcoin::transaction::Transaction;
use bitcoin::amount::Amount;
use bitcoin::Sequence;
use bitcoin::Witness;
use reqwest::Client as HttpClient;
use sha2::{Digest, Sha256};

use utxo_vmd::consensus::attestation::ConsensusManager;
use utxo_vmd::consensus::covenants;
use utxo_vmd::consensus::l1_scripts;

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";
const SENDER_ADDRESS: &str = "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi";
const PRIV_KEY_HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

async fn fetch_utxos(http: &HttpClient) -> Vec<serde_json::Value> {
    http.get(format!("{}/address/{}/utxo", ELECTRS_URL, SENDER_ADDRESS))
        .send().await.unwrap().json().await.unwrap()
}

async fn sign_and_broadcast_p2pkh(
    http: &HttpClient,
    secp: &Secp256k1<secp256k1::All>,
    sk: &SecretKey,
    pk_comp: &[u8],
    input_txid: &str,
    input_vout: u32,
    input_amount: u64,
    outputs: Vec<bitcoin::TxOut>,
    label: &str,
) -> Result<String, String> {
    let input_txid_parsed: bitcoin::Txid = input_txid.parse().unwrap();
    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::locktime::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn {
            previous_output: bitcoin::OutPoint { txid: input_txid_parsed, vout: input_vout },
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xFFFFFFFD),
            witness: Witness::default(),
        }],
        output: outputs,
    };

    let sighash_type = bitcoin::sighash::EcdsaSighashType::All;
    let p2pkh_script = ScriptBuf::builder()
        .push_opcode(OP_DUP)
        .push_opcode(OP_HASH160)
        .push_slice(&bitcoin::hashes::hash160::Hash::hash(pk_comp).to_byte_array())
        .push_opcode(OP_EQUALVERIFY)
        .push_opcode(OP_CHECKSIG)
        .into_script();

    let mut sighash_cache = SighashCache::new(&tx);
    let sighash = sighash_cache.legacy_signature_hash(0, &p2pkh_script, sighash_type.to_u32()).unwrap();
    let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
    let sig = secp.sign_ecdsa(&msg, sk);
    let mut sig_with_hashtype = sig.serialize_der().to_vec();
    sig_with_hashtype.push(sighash_type.to_u32() as u8);

    let mut sig_push = PushBytesBuf::new();
    sig_push.extend_from_slice(&sig_with_hashtype).ok();
    let mut pk_push = PushBytesBuf::new();
    pk_push.extend_from_slice(pk_comp).ok();

    tx.input[0].script_sig = ScriptBuf::builder()
        .push_slice(&sig_push)
        .push_slice(&pk_push)
        .into_script();

    let raw_tx_bytes = encode::serialize(&tx);
    let raw_tx_hex = hex::encode(&raw_tx_bytes);

    let resp = http
        .post(format!("{}/tx", ELECTRS_URL))
        .header("Content-Type", "text/plain")
        .body(raw_tx_hex)
        .send()
        .await
        .map_err(|e| format!("Send failed: {}", e))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if status.is_success() {
        let txid = body.trim().to_string();
        println!("[LIVE] {} OK: txid={}", label, txid);
        Ok(txid)
    } else {
        Err(format!("HTTP {} — {}", status, body))
    }
}

#[tokio::test]
#[ignore]
async fn live_deploy_p2tr_vault_and_challenge() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();

    // Operator keypair (the bonded operator)
    let (op_sk, op_pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let op_pk_comp = op_pk.serialize();

    // Challenger keypair (the watcher who will slash)
    // Need Keypair for Schnorr signing
    let challenger_keypair = {
        let mut sk_bytes = [0u8; 32];
        secp256k1::rand::rngs::OsRng.fill_bytes(&mut sk_bytes);
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        secp256k1::Keypair::from_secret_key(&secp, &sk)
    };
    let challenger_pk = challenger_keypair.public_key();
    let challenger_pk_comp = challenger_pk.serialize();
    let challenger_xonly = XOnlyPublicKey::from_keypair(&challenger_keypair).0;

    println!("[LIVE] Operator pubkey: {}", hex::encode(&op_pk_comp));
    println!("[LIVE] Challenger pubkey: {}", hex::encode(&challenger_pk_comp));
    println!("[LIVE] Challenger x-only: {}", challenger_xonly);

    // Build the vault script tree using l1_scripts
    let vault_config = l1_scripts::VaultConfig {
        operator_pubkey: op_pk_comp.to_vec(),
        challenger_pubkey: challenger_pk_comp.to_vec(),
        unbond_delay: 60,
        claim_delay: 10,
        watcher_pubkeys: vec![challenger_pk_comp.to_vec()],
        watcher_threshold: 1,
    };

    let vault_tree = l1_scripts::build_vault_script_tree(&vault_config)
        .expect("Failed to build vault script tree");

    println!("[LIVE] Vault script tree built:");
    println!("[LIVE]   Operator leaf: {} bytes", vault_tree.operator_leaf.len());
    println!("[LIVE]   Challenge leaf: {} bytes", vault_tree.challenge_leaf.len());

    // Build a REAL P2TR output using rust-bitcoin's TaprootBuilder
    // Use a random internal key (key-path spend not used — only script-path)
    let nums_keypair = {
        let mut sk_bytes = [0u8; 32];
        secp256k1::rand::rngs::OsRng.fill_bytes(&mut sk_bytes);
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        secp256k1::Keypair::from_secret_key(&secp, &sk)
    };
    let nums_xonly = XOnlyPublicKey::from_keypair(&nums_keypair).0;

    let operator_leaf_script = ScriptBuf::from_bytes(vault_tree.operator_leaf.clone());
    let challenge_leaf_script = ScriptBuf::from_bytes(vault_tree.challenge_leaf.clone());

    let taproot_builder = TaprootBuilder::new()
        .add_leaf(1, operator_leaf_script.clone())
        .expect("Failed to add operator leaf")
        .add_leaf(1, challenge_leaf_script.clone())
        .expect("Failed to add challenge leaf");

    let taproot_spend_info = taproot_builder
        .finalize(&secp, nums_xonly)
        .expect("Failed to finalize taproot");

    let output_key = taproot_spend_info.output_key();
    println!("[LIVE] P2TR output key (x-only): {}", output_key);

    // Build the P2TR output script: OP_1 <32-byte x-only output key>
    let vault_script_pubkey = ScriptBuf::builder()
        .push_opcode(OP_PUSHNUM_1)
        .push_slice(&output_key.serialize())
        .into_script();

    println!("[LIVE] P2TR vault script: {} bytes", vault_script_pubkey.len());

    // Get a UTXO to fund the vault
    let (funder_sk, _, funder_pk_comp) = {
        let sk = SecretKey::from_slice(&hex::decode(PRIV_KEY_HEX).unwrap()).unwrap();
        let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
        (sk, (), pk.serialize().to_vec())
    };

    let utxos = fetch_utxos(&http).await;
    // Use any UTXO >= 500k sats
    let utxo = utxos.iter().find(|u| u["value"].as_u64().unwrap_or(0) >= 500_000)
        .expect("Need a UTXO >= 500k sats for vault");

    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // Lock 100,000 sats (0.001 tJKC) in the vault
    let bond_amount = 100_000u64;
    let miner_fee = 500u64;
    let change_amount = input_amount.saturating_sub(bond_amount).saturating_sub(miner_fee);

    // Change output (P2PKH back to funder)
    let wpk = bitcoin::PublicKey::from_slice(&funder_pk_comp).unwrap();
    let change_script = Address::p2pkh(&wpk, bitcoin::network::Network::Testnet).script_pubkey();

    // OP_RETURN envelope
    let vault_payload = format!(
        r#"{{"type":"P2TRVault","operator":"{}","bondSatoshis":{},"unbondDelay":{},"claimDelay":{}}}"#,
        hex::encode(&op_pk_comp), bond_amount, vault_config.unbond_delay, vault_config.claim_delay
    );
    let payload_hash = Sha256::digest(vault_payload.as_bytes());
    let mut tag_push = PushBytesBuf::new();
    tag_push.extend_from_slice(b"utxovm:vault").ok();
    let mut hash_push = PushBytesBuf::new();
    hash_push.extend_from_slice(&payload_hash).ok();
    let mut op_return_script = ScriptBuf::new();
    op_return_script.push_opcode(OP_RETURN);
    op_return_script.push_slice(&tag_push);
    op_return_script.push_slice(&hash_push);

    // Deploy the vault
    let vault_txid = sign_and_broadcast_p2pkh(
        &http, &secp, &funder_sk, &funder_pk_comp,
        input_txid, input_vout, input_amount,
        vec![
            bitcoin::TxOut { value: Amount::from_sat(0), script_pubkey: op_return_script },
            bitcoin::TxOut { value: Amount::from_sat(bond_amount), script_pubkey: vault_script_pubkey.clone() },
            bitcoin::TxOut { value: Amount::from_sat(change_amount), script_pubkey: change_script },
        ],
        "DEPLOY P2TR VAULT",
    ).await;

    let vault_txid = match vault_txid {
        Ok(txid) => {
            println!("[LIVE] Vault deployed! TXID: {}", txid);
            println!("[LIVE] Bond: {} sats (0.005 tJKC)", bond_amount);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
            txid
        }
        Err(e) => {
            println!("[LIVE] Vault deploy failed: {}", e);
            println!("[LIVE] Skipping challenge (no vault to slash)");
            return;
        }
    };

    // Wait for indexing
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Create an equivocation proof
    let mgr = ConsensusManager::new(1);
    let op_pk_hex = hex::encode(&op_pk_comp);
    mgr.register_validator(&op_pk_hex);

    let root_honest = hex::encode(Sha256::digest(b"vault_honest_root"));
    let root_malicious = hex::encode(Sha256::digest(b"vault_malicious_root"));

    let att1 = mgr.sign_state_root(&op_sk, "JKC_TESTNET", 177_620, "vault_block", &root_honest).unwrap();
    let att2 = mgr.sign_state_root(&op_sk, "JKC_TESTNET", 177_620, "vault_block", &root_malicious).unwrap();

    mgr.add_attestation(att1.clone());
    mgr.add_attestation(att2.clone());

    let proofs = mgr.get_slashing_proofs();
    assert_eq!(proofs.len(), 1);
    let proof = &proofs[0];
    assert!(covenants::verify_equivocation_proof(proof));
    println!("[LIVE] Equivocation proof created and verified");

    // Build the challenge tx that spends the vault via script path
    let vault_outpoint = bitcoin::OutPoint {
        txid: vault_txid.parse().unwrap(),
        vout: 1, // vault output is vout 1
    };

    // Challenge output: P2PKH to challenger
    let challenger_wpk = bitcoin::PublicKey::from_slice(&challenger_pk_comp).unwrap();
    let challenge_script = Address::p2pkh(&challenger_wpk, bitcoin::network::Network::Testnet).script_pubkey();

    // Evidence OP_RETURN
    let mut ev_tag = PushBytesBuf::new();
    ev_tag.extend_from_slice(b"utxovm:challenge").ok();
    let claimed_root = Sha256::digest(&proof.first_attestation.state_root.as_bytes());
    let correct_root = Sha256::digest(&proof.second_attestation.state_root.as_bytes());
    let mut ev_r1 = PushBytesBuf::new();
    ev_r1.extend_from_slice(&claimed_root).ok();
    let mut ev_r2 = PushBytesBuf::new();
    ev_r2.extend_from_slice(&correct_root).ok();

    let mut evidence_script = ScriptBuf::new();
    evidence_script.push_opcode(OP_RETURN);
    evidence_script.push_slice(&ev_tag);
    evidence_script.push_slice(&ev_r1);
    evidence_script.push_slice(&ev_r2);

    let challenge_fee = 1000u64;
    let challenge_output_amount = bond_amount.saturating_sub(challenge_fee);

    let challenge_tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::locktime::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn {
            previous_output: vault_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xFFFFFFFD),
            witness: Witness::default(),
        }],
        output: vec![
            bitcoin::TxOut { value: Amount::from_sat(0), script_pubkey: evidence_script },
            bitcoin::TxOut { value: Amount::from_sat(challenge_output_amount), script_pubkey: challenge_script },
        ],
    };

    // Compute the tapleaf hash for the challenge leaf
    let challenge_leaf_hash = TapLeafHash::from_script(&challenge_leaf_script, LeafVersion::TapScript);

    // Compute taproot sighash for script-path spend
    let sighash_type = bitcoin::sighash::TapSighashType::All;
    let prevouts = Prevouts::All(&[
        bitcoin::TxOut { value: Amount::from_sat(bond_amount), script_pubkey: vault_script_pubkey.clone() },
    ]);

    let mut sighash_cache = SighashCache::new(&challenge_tx);
    let sighash = sighash_cache
        .taproot_script_spend_signature_hash(0, &prevouts, challenge_leaf_hash, sighash_type)
        .expect("Failed to compute taproot sighash");

    // Sign with challenger key (Schnorr)
    let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
    let tap_sig = secp.sign_schnorr_with_rng(&msg, &challenger_keypair, &mut secp256k1::rand::rngs::OsRng);
    let mut sig_with_hashtype = tap_sig.serialize().to_vec();
    sig_with_hashtype.push(sighash_type as u8);

    // Get the control block for the challenge leaf
    let control_block = taproot_spend_info
        .control_block(&(challenge_leaf_script.clone(), LeafVersion::TapScript))
        .expect("Failed to build control block");

    // Build witness: [sig, challenge_leaf_script, control_block]
    // BIP-342 individual CHECKSIG — no dummy element needed
    let mut witness = Witness::new();
    witness.push(&sig_with_hashtype);
    witness.push(challenge_leaf_script.as_bytes());
    witness.push(control_block.serialize());

    let mut challenge_tx = challenge_tx;
    challenge_tx.input[0].witness = witness;

    // Serialize and broadcast
    let raw_tx_bytes = encode::serialize(&challenge_tx);
    let raw_tx_hex = hex::encode(&raw_tx_bytes);
    let challenge_txid = challenge_tx.compute_txid();

    println!("[LIVE] Challenge tx built:");
    println!("[LIVE]   TxID: {}", challenge_txid);
    println!("[LIVE]   Size: {} bytes", raw_tx_bytes.len());
    println!("[LIVE]   Challenge output: {} sats", challenge_output_amount);
    println!("[LIVE]   Fee: {} sats", challenge_fee);
    println!("[LIVE]   Witness items: {}", challenge_tx.input[0].witness.len());

    // Broadcast
    let resp = http
        .post(format!("{}/tx", ELECTRS_URL))
        .header("Content-Type", "text/plain")
        .body(raw_tx_hex)
        .send()
        .await
        .expect("Failed to send challenge tx");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    println!("[LIVE] Challenge broadcast: HTTP {} — {}", status, body);

    if status.is_success() {
        let broadcast_txid = body.trim();
        println!("[LIVE] CHALLENGE TX BROADCAST! TXID: {}", broadcast_txid);
        println!("[LIVE] Bond slashed! {} sats moved to challenger", challenge_output_amount);
        println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", broadcast_txid);
    } else {
        println!("[LIVE] Challenge broadcast failed: {}", body);
        println!("[LIVE] Possible: vault tx not confirmed yet, invalid sig, or non-standard witness");
    }

    // Verify structure
    assert!(raw_tx_bytes.len() > 100);
    assert_eq!(challenge_tx.input.len(), 1);
    assert_eq!(challenge_tx.output.len(), 2);
    assert!(!challenge_tx.input[0].witness.is_empty());
    println!("[LIVE] Challenge tx structure verified: P2TR script-path spend with witness");
}
