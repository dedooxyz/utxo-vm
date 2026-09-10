//! Live JKC Testnet: 3-Validator Quorum + Equivocation
//!
//! Run with: cargo test -p utxo-vmd --test live_quorum_equivocation -- --nocapture --ignored

use std::cmp::min;

use bitcoin::address::Address;
use bitcoin::consensus::encode;
use bitcoin::hashes::Hash;
use bitcoin::opcodes::all::*;
use bitcoin::script::{ScriptBuf, PushBytesBuf};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::sighash::SighashCache;
use bitcoin::transaction::Transaction;
use bitcoin::amount::Amount;
use bitcoin::Sequence;
use reqwest::Client as HttpClient;
use sha2::{Digest, Sha256};

use utxo_vmd::consensus::attestation::ConsensusManager;
use utxo_vmd::consensus::covenants;
use utxo_vmd::consensus::challenge;
use utxo_vmd::consensus::l1_scripts;

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";
const SENDER_ADDRESS: &str = "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi";
const PRIV_KEY_HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

async fn fetch_utxos(http: &HttpClient) -> Vec<serde_json::Value> {
    http.get(format!("{}/address/{}/utxo", ELECTRS_URL, SENDER_ADDRESS))
        .send().await.unwrap().json().await.unwrap()
}

async fn sign_and_broadcast(
    http: &HttpClient,
    secp: &Secp256k1<secp256k1::All>,
    sk: &SecretKey,
    pk_comp: &[u8],
    input_txid: &str,
    input_vout: u32,
    input_amount: u64,
    op_return_tag: &[u8],
    payload: &[u8],
    miner_fee: u64,
    label: &str,
) -> Result<String, String> {
    let wpk = bitcoin::PublicKey::from_slice(pk_comp).unwrap();
    let payload_hash = Sha256::digest(payload);

    let mut tag_push = PushBytesBuf::new();
    tag_push.extend_from_slice(op_return_tag).ok();
    let mut hash_push = PushBytesBuf::new();
    hash_push.extend_from_slice(&payload_hash).ok();

    let mut op_return_script = ScriptBuf::new();
    op_return_script.push_opcode(OP_RETURN);
    op_return_script.push_slice(&tag_push);
    op_return_script.push_slice(&hash_push);

    let addr = Address::p2pkh(&wpk, bitcoin::network::Network::Testnet);
    let change_script = addr.script_pubkey();
    let change_amount = input_amount.saturating_sub(miner_fee);

    let input_txid_parsed: bitcoin::Txid = input_txid.parse().unwrap();
    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::locktime::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn {
            previous_output: bitcoin::OutPoint { txid: input_txid_parsed, vout: input_vout },
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xFFFFFFFD),
            witness: bitcoin::Witness::default(),
        }],
        output: vec![
            bitcoin::TxOut { value: Amount::from_sat(0), script_pubkey: op_return_script },
            bitcoin::TxOut { value: Amount::from_sat(change_amount), script_pubkey: change_script },
        ],
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
async fn live_quorum_3_validators_same_height() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();

    // Generate 3 validator keypairs
    let mut validators = Vec::new();
    for i in 0..3 {
        let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
        let pk_hex = hex::encode(pk.serialize());
        println!("[LIVE] Validator {}: {}...", i, &pk_hex[..16]);
        validators.push((sk, pk, pk_hex));
    }

    // Register all 3 in consensus manager
    let mgr = ConsensusManager::new(2); // threshold = 2 of 3
    for (_, _, pk_hex) in &validators {
        mgr.register_validator(pk_hex);
    }

    let chain = "JKC_TESTNET";
    let height = 177_610u64;
    let block_hash = "quorum_test_block_hash";
    let state_root = hex::encode(Sha256::digest(b"quorum_test_state_root"));

    // All 3 sign the same state
    let mut attestations = Vec::new();
    for (i, (sk, _, _)) in validators.iter().enumerate() {
        let att = mgr.sign_state_root(sk, chain, height, block_hash, &state_root)
            .expect("Failed to sign");
        println!("[LIVE] Validator {} signed: sig={}...", i, &att.signature_hex[..16]);
        attestations.push(att);
    }

    // Add all attestations
    for att in &attestations {
        mgr.add_attestation(att.clone());
    }

    // Check quorum
    let quorum = mgr.build_quorum_result(chain, height)
        .expect("No quorum result");
    println!("[LIVE] Quorum result:");
    println!("[LIVE]   Chain: {}", quorum.chain);
    println!("[LIVE]   Height: {}", quorum.block_height);
    println!("[LIVE]   Support count: {}", quorum.support_count);
    println!("[LIVE]   Threshold: {}", quorum.quorum_threshold);
    println!("[LIVE]   State root: {}", quorum.state_root);
    println!("[LIVE]   Merkle root: {}", quorum.merkle_root);

    assert!(quorum.support_count >= quorum.quorum_threshold, "Must have quorum");
    assert_eq!(quorum.support_count, 3);
    assert_eq!(quorum.state_root, state_root);

    // Post quorum attestation on-chain
    let (funder_sk, _, pk_comp) = {
        let sk = SecretKey::from_slice(&hex::decode(PRIV_KEY_HEX).unwrap()).unwrap();
        let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
        (sk, (), pk.serialize().to_vec())
    };

    let utxos = fetch_utxos(&http).await;
    let utxo = utxos.iter().find(|u| u["value"].as_u64() == Some(500))
        .or(utxos.iter().find(|u| u["value"].as_u64() == Some(200)))
        .expect("Need UTXO");

    let quorum_payload = format!(
        r#"{{"type":"quorum","chain":"{}","height":{},"blockHash":"{}","stateRoot":"{}","signatures":{},"threshold":{}}}"#,
        chain, height, block_hash, state_root, quorum.support_count, quorum.quorum_threshold
    );

    let result = sign_and_broadcast(
        &http, &secp, &funder_sk, &pk_comp,
        utxo["txid"].as_str().unwrap(), utxo["vout"].as_u64().unwrap() as u32,
        utxo["value"].as_u64().unwrap(),
        b"utxovm:quorum", quorum_payload.as_bytes(), 300,
        "POST QUORUM (3-of-3)",
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] Quorum attestation posted! TXID: {}", txid);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => println!("[LIVE] Quorum broadcast failed: {}", e),
    }
}

#[tokio::test]
#[ignore]
async fn live_equivocation_double_sign_on_chain() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();

    // Generate 1 validator (the equivocator)
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let pk_hex = hex::encode(pk.serialize());

    let mgr = ConsensusManager::new(1);
    mgr.register_validator(&pk_hex);

    let chain = "JKC_TESTNET";
    let height = 177_615u64;
    let block_hash = "equivocation_test_block";

    // Sign two different roots at the same height
    let root_honest = hex::encode(Sha256::digest(b"equivocation_honest_root"));
    let root_malicious = hex::encode(Sha256::digest(b"equivocation_malicious_root"));

    let att1 = mgr.sign_state_root(&sk, chain, height, block_hash, &root_honest).unwrap();
    let att2 = mgr.sign_state_root(&sk, chain, height, block_hash, &root_malicious).unwrap();

    println!("[LIVE] Equivocation: validator double-signed at height {}", height);
    println!("[LIVE]   Root 1 (honest): {}", root_honest);
    println!("[LIVE]   Root 2 (malicious): {}", root_malicious);

    // Add both attestations — this triggers equivocation detection
    mgr.add_attestation(att1.clone());
    mgr.add_attestation(att2.clone());

    // Verify equivocation was detected
    let proofs = mgr.get_slashing_proofs();
    assert_eq!(proofs.len(), 1, "Must detect 1 equivocation");
    let proof = &proofs[0];

    println!("[LIVE] Equivocation detected!");
    println!("[LIVE]   Validator: {}...", &proof.validator_pubkey[..16]);
    println!("[LIVE]   First root: {}", proof.first_attestation.state_root);
    println!("[LIVE]   Second root: {}", proof.second_attestation.state_root);

    // Verify the proof cryptographically
    assert!(covenants::verify_equivocation_proof(proof), "Proof must verify");
    println!("[LIVE] Equivocation proof cryptographically verified: OK");

    // Post both conflicting attestations on-chain (this is what a real operator would do)
    let (funder_sk, _, pk_comp) = {
        let sk = SecretKey::from_slice(&hex::decode(PRIV_KEY_HEX).unwrap()).unwrap();
        let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
        (sk, (), pk.serialize().to_vec())
    };

    let utxos = fetch_utxos(&http).await;

    // Post attestation 1 (honest root)
    let utxo1 = utxos.iter().find(|u| u["value"].as_u64() == Some(500))
        .or(utxos.iter().find(|u| u["value"].as_u64() == Some(200)));
    if let Some(utxo1) = utxo1 {
        let payload1 = format!(
            r#"{{"chain":"{}","height":{},"blockHash":"{}","stateRoot":"{}","validator":"{}","signature":"{}"}}"#,
            att1.chain, att1.block_height, att1.block_hash,
            att1.state_root, att1.validator_pubkey, att1.signature_hex
        );
        let result = sign_and_broadcast(
            &http, &secp, &funder_sk, &pk_comp,
            utxo1["txid"].as_str().unwrap(), utxo1["vout"].as_u64().unwrap() as u32,
            utxo1["value"].as_u64().unwrap(),
            b"utxovm:batch", payload1.as_bytes(), 300,
            "POST ATTESTATION 1 (honest root)",
        ).await;
        match result {
            Ok(txid) => println!("[LIVE] Attestation 1 posted: {}", txid),
            Err(e) => println!("[LIVE] Attestation 1 failed: {}", e),
        }
    }

    // Wait for UTXO to update
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let utxos2 = fetch_utxos(&http).await;

    // Post attestation 2 (malicious root) — this is the equivocation
    let utxo2 = utxos2.iter().find(|u| u["value"].as_u64() == Some(500))
        .or(utxos2.iter().find(|u| u["value"].as_u64() == Some(200)));
    if let Some(utxo2) = utxo2 {
        let payload2 = format!(
            r#"{{"chain":"{}","height":{},"blockHash":"{}","stateRoot":"{}","validator":"{}","signature":"{}"}}"#,
            att2.chain, att2.block_height, att2.block_hash,
            att2.state_root, att2.validator_pubkey, att2.signature_hex
        );
        let result = sign_and_broadcast(
            &http, &secp, &funder_sk, &pk_comp,
            utxo2["txid"].as_str().unwrap(), utxo2["vout"].as_u64().unwrap() as u32,
            utxo2["value"].as_u64().unwrap(),
            b"utxovm:batch", payload2.as_bytes(), 300,
            "POST ATTESTATION 2 (MALICIOUS root — EQUIVOCATION)",
        ).await;
        match result {
            Ok(txid) => {
                println!("[LIVE] Attestation 2 (equivocation) posted: {}", txid);
                println!("[LIVE] Both conflicting attestations now on-chain!");
            }
            Err(e) => println!("[LIVE] Attestation 2 failed: {}", e),
        }
    }

    // Build challenge tx from the proof
    let input = challenge::ChallengeInput {
        bond_txid: "ab".repeat(32),
        bond_vout: 0,
        bond_amount: 100_000_000,
    };

    let vault_config = l1_scripts::VaultConfig {
        operator_pubkey: hex::decode(&pk_hex).unwrap(),
        challenger_pubkey: vec![0x03; 33],
        unbond_delay: 60,
        claim_delay: 10,
        watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33]],
        watcher_threshold: 1,
    };

    let watcher_sig = vec![0xAAu8; 64];
    let tx = challenge::build_challenge_transaction(proof, &input, &vault_config, &[watcher_sig], 1000)
        .expect("Failed to build challenge tx");

    println!("[LIVE] Challenge tx built from on-chain equivocation:");
    println!("[LIVE]   Raw hex: {}...", &tx.raw_hex[..min(48, tx.raw_hex.len())]);
    println!("[LIVE]   Challenge UTXO: {} sats", tx.challenge_output.amount);
    println!("[LIVE]   Evidence: {} bytes", tx.evidence_output.len());
    println!("[LIVE]   Contains 'utxovm:challenge': {}",
        tx.evidence_output.windows(16).any(|w| w == b"utxovm:challenge"));
}
