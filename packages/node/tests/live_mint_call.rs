//! Live JKC Testnet: Mint, Call, Attest, Challenge
//!
//! Run with: cargo test -p utxo-vmd --test live_mint_call -- --nocapture --ignored
//!
//! Full end-to-end live test:
//! 1. Deploy SOT token (mint) — inscribe utxovm:deploy with token init args
//! 2. Call transfer — inscribe utxovm:call with transfer args
//! 3. Post attestation — inscribe utxovm:batch with state root
//! 4. Build challenge tx — from equivocation proof (not broadcast)
//! 5. Verify all txs on electrs

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
use utxo_vmd::consensus::challenge;
use utxo_vmd::consensus::l1_scripts;

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";
const SENDER_ADDRESS: &str = "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi";
const PRIV_KEY_HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

async fn fetch_utxos(http: &HttpClient) -> Vec<serde_json::Value> {
    http.get(format!("{}/address/{}/utxo", ELECTRS_URL, SENDER_ADDRESS))
        .send().await.unwrap().json().await.unwrap()
}

fn find_utxo(utxos: &[serde_json::Value], target_value: u64) -> Option<&serde_json::Value> {
    utxos.iter().find(|u| u["value"].as_u64() == Some(target_value))
}

/// Build, sign, and broadcast a P2PKH tx with an OP_RETURN envelope.
async fn build_sign_broadcast(
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

    // Build OP_RETURN: OP_RETURN <tag> <payload_hash>
    let payload_hash = Sha256::digest(payload);
    let mut tag_push = PushBytesBuf::new();
    tag_push.extend_from_slice(op_return_tag);
    let mut hash_push = PushBytesBuf::new();
    hash_push.extend_from_slice(&payload_hash);

    let mut op_return_script = ScriptBuf::new();
    op_return_script.push_opcode(OP_RETURN);
    op_return_script.push_slice(&tag_push);
    op_return_script.push_slice(&hash_push);

    // Change output (P2PKH back to sender)
    let addr = Address::p2pkh(&wpk, bitcoin::network::Network::Testnet);
    let change_script = addr.script_pubkey();
    let change_amount = input_amount.saturating_sub(miner_fee);

    // Build tx
    let input_txid_parsed: bitcoin::Txid = input_txid.parse().unwrap();
    let tx = Transaction {
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

    // Sign
    let mut tx = tx;
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

    // Serialize & broadcast
    let raw_tx_bytes = encode::serialize(&tx);
    let raw_tx_hex = hex::encode(&raw_tx_bytes);
    let txid = tx.compute_txid();

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
        let broadcast_txid = body.trim().to_string();
        println!("[LIVE] {} broadcast OK: txid={}", label, broadcast_txid);
        println!("[LIVE]   Expected txid: {}", txid);
        println!("[LIVE]   Payload: {}", String::from_utf8_lossy(payload));
        println!("[LIVE]   Payload hash: {}", hex::encode(&payload_hash));
        println!("[LIVE]   Fee: {} sats, Change: {} sats", miner_fee, change_amount);
        Ok(broadcast_txid)
    } else {
        Err(format!("Broadcast failed: HTTP {} — {}", status, body))
    }
}

#[tokio::test]
#[ignore]
async fn live_mint_sot_token() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();
    let sk_bytes = hex::decode(PRIV_KEY_HEX).unwrap();
    let sk = SecretKey::from_slice(&sk_bytes).unwrap();
    let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pk_comp = pk.serialize();

    let utxos = fetch_utxos(&http).await;

    // Use a 500-sat UTXO for the mint
    let utxo = find_utxo(&utxos, 500).expect("Need a 500-sat UTXO");
    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // Mint payload: deploy a SOT token
    let mint_payload = br#"{"name":"TestJKC","symbol":"tJKC","decimals":8,"totalSupply":1000000,"balance":1000000,"owner":"muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi"}"#;

    let result = build_sign_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:deploy", mint_payload, 300,
        "MINT SOT Token",
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] Token minted! TXID: {}", txid);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
            // Wait for indexing
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let check = http.get(format!("{}/tx/{}", ELECTRS_URL, txid)).send().await;
            if let Ok(resp) = check {
                println!("[LIVE] TX indexed: {}", resp.status().is_success());
            }
        }
        Err(e) => {
            println!("[LIVE] Mint broadcast failed: {}", e);
            println!("[LIVE] (UTXO may already be spent)");
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_call_transfer() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();
    let sk_bytes = hex::decode(PRIV_KEY_HEX).unwrap();
    let sk = SecretKey::from_slice(&sk_bytes).unwrap();
    let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pk_comp = pk.serialize();

    let utxos = fetch_utxos(&http).await;

    // Use a different 500-sat UTXO
    let utxo = utxos.iter().find(|u| {
        u["value"].as_u64() == Some(500) && {
            let txid = u["txid"].as_str().unwrap();
            // Skip the one we used for mint (if mint ran first)
            !txid.starts_with("1a2969aa")
        }
    }).or(utxos.iter().find(|u| u["value"].as_u64() == Some(500)));

    let utxo = utxo.expect("Need a 500-sat UTXO for transfer");
    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // Transfer payload: full balance transfer (UTXO model)
    let transfer_payload = br#"{"method":"transfer","to":"mscKWCFZ7YsBY7Xc8ygY9DwmDAPE37CFFf","amount":1000000}"#;

    let result = build_sign_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:call", transfer_payload, 300,
        "CALL transfer",
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] Transfer call broadcast! TXID: {}", txid);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => {
            println!("[LIVE] Transfer broadcast failed: {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_post_attestation() {
    let http = HttpClient::new();
    let secp = Secp256k1::new();
    let sk_bytes = hex::decode(PRIV_KEY_HEX).unwrap();
    let sk = SecretKey::from_slice(&sk_bytes).unwrap();
    let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pk_comp = pk.serialize();

    // Generate a signed attestation using ConsensusManager
    let mgr = ConsensusManager::new(1);
    let pk_hex = hex::encode(&pk_comp);
    mgr.register_validator(&pk_hex);

    let chain = "JKC_TESTNET";
    let height = 177_590u64;
    let block_hash = "live_block_hash_placeholder";
    let state_root = hex::encode(Sha256::digest(b"live_test_state_root"));

    let attestation = mgr.sign_state_root(&sk, chain, height, block_hash, &state_root)
        .expect("Failed to sign attestation");

    println!("[LIVE] Attestation signed:");
    println!("[LIVE]   Chain: {}", attestation.chain);
    println!("[LIVE]   Height: {}", attestation.block_height);
    println!("[LIVE]   State root: {}", attestation.state_root);
    println!("[LIVE]   Validator: {}...", &attestation.validator_pubkey[..16]);
    println!("[LIVE]   Signature: {}...", &attestation.signature_hex[..16]);

    // Verify the attestation
    assert!(mgr.verify_attestation(&attestation), "Attestation must verify");
    println!("[LIVE] Attestation verified: OK");

    // Post attestation on-chain
    let utxos = fetch_utxos(&http).await;
    let utxo = find_utxo(&utxos, 500).or(utxos.iter().find(|u| u["value"].as_u64() == Some(1000)));
    let utxo = utxo.expect("Need a UTXO for attestation");
    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    let attestation_payload = format!(
        r#"{{"chain":"{}","height":{},"blockHash":"{}","stateRoot":"{}","validator":"{}","signature":"{}"}}"#,
        attestation.chain, attestation.block_height, attestation.block_hash,
        attestation.state_root, attestation.validator_pubkey, attestation.signature_hex
    );

    let result = build_sign_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:batch", attestation_payload.as_bytes(), 300,
        "POST ATTESTATION",
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] Attestation posted on-chain! TXID: {}", txid);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => {
            println!("[LIVE] Attestation broadcast failed: {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_equivocation_and_challenge_build() {
    // Generate a real equivocation: sign two conflicting roots
    let mgr = ConsensusManager::new(1);
    let secp = Secp256k1::new();
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);
    let pk_hex = hex::encode(pk.serialize());
    mgr.register_validator(&pk_hex);

    let att1 = mgr.sign_state_root(&sk, "JKC_TESTNET", 177_595, "block_h", "root_honest_live").unwrap();
    mgr.add_attestation(att1);

    let att2 = mgr.sign_state_root(&sk, "JKC_TESTNET", 177_595, "block_h", "root_malicious_live").unwrap();
    mgr.add_attestation(att2);

    let proofs = mgr.get_slashing_proofs();
    assert_eq!(proofs.len(), 1);
    let proof = &proofs[0];

    println!("[LIVE] Equivocation detected:");
    println!("[LIVE]   Validator: {}...", &proof.validator_pubkey[..16]);
    println!("[LIVE]   Root 1 (honest): {}", proof.first_attestation.state_root);
    println!("[LIVE]   Root 2 (malicious): {}", proof.second_attestation.state_root);

    // Verify the proof cryptographically
    assert!(utxo_vmd::consensus::covenants::verify_equivocation_proof(proof));
    println!("[LIVE] Equivocation proof verified: OK");

    // Build challenge tx (not broadcast — this would slash a real bond)
    let input = challenge::ChallengeInput {
        bond_txid: "ab".repeat(32),
        bond_vout: 0,
        bond_amount: 100_000_000, // 1 JKC bond
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

    println!("[LIVE] Challenge tx built (NOT broadcast — would slash real bond):");
    println!("[LIVE]   Raw hex: {}...", &tx.raw_hex[..min(48, tx.raw_hex.len())]);
    println!("[LIVE]   Challenge UTXO: {} sats", tx.challenge_output.amount);
    println!("[LIVE]   Evidence: {} bytes", tx.evidence_output.len());
    println!("[LIVE]   Contains 'utxovm:challenge': {}", tx.evidence_output.windows(16).any(|w| w == b"utxovm:challenge"));
    println!("[LIVE] Challenge tx NOT broadcast (no real bond to slash in test)");
}

#[tokio::test]
#[ignore]
async fn live_verify_all_recent_utxovm_txs() {
    let http = HttpClient::new();
    let tip: u64 = http
        .get(format!("{}/blocks/tip/height", ELECTRS_URL))
        .send().await.unwrap().text().await.unwrap().trim().parse().unwrap();

    println!("[LIVE] Scanning blocks {}..{} for utxovm transactions", tip - 20, tip);

    let mut found_count = 0;
    for height in (tip - 20)..=tip {
        let block_hash = match http
            .get(format!("{}/block-height/{}", ELECTRS_URL, height))
            .send().await {
            Ok(r) => r.text().await.unwrap_or_default().trim().to_string(),
            Err(_) => continue,
        };

        let txs: Vec<serde_json::Value> = match http
            .get(format!("{}/block/{}/txs", ELECTRS_URL, block_hash))
            .send().await {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(_) => continue,
        };

        for tx in &txs {
            let txid = tx["txid"].as_str().unwrap_or("");
            for (vout, output) in tx["vout"].as_array().unwrap_or(&vec![]).iter().enumerate() {
                let asm = output["scriptpubkey_asm"].as_str().unwrap_or("");
                let hex_str = output["scriptpubkey_hex"].as_str().unwrap_or("");
                if asm.contains("utxovm") || hex_str.contains("7574786f766d") {
                    println!("[LIVE] Block {} tx {} vout {}: {}", height, &txid[..min(16, txid.len())], vout, asm);
                    found_count += 1;
                }
            }
        }
    }

    println!("[LIVE] Found {} utxovm transactions in last 21 blocks", found_count);
}
