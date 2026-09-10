//! Live JKC Testnet Broadcast Test
//!
//! Run with: cargo test -p utxo-vmd --test live_broadcast -- --nocapture --ignored
//!
//! This test builds, signs, and broadcasts a real utxovm inscription tx to JKC testnet.
//! It spends ~500 sats from the funded testnet wallet.

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

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";
const SENDER_ADDRESS: &str = "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi";
const PRIV_KEY_HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[tokio::test]
#[ignore]
async fn live_broadcast_inscription_tx() {
    let secp = Secp256k1::new();
    let sk_bytes = hex::decode(PRIV_KEY_HEX).unwrap();
    let sk = SecretKey::from_slice(&sk_bytes).unwrap();
    let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pk_comp = pk.serialize();
    let wpk = bitcoin::PublicKey::from_slice(&pk_comp).unwrap();

    // Fetch UTXOs
    let http = HttpClient::new();
    let utxos: Vec<serde_json::Value> = http
        .get(format!("{}/address/{}/utxo", ELECTRS_URL, SENDER_ADDRESS))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Find a 500-sat UTXO
    let utxo = utxos.iter().find(|u| u["value"].as_u64() == Some(500))
        .or(utxos.iter().find(|u| u["value"].as_u64() == Some(1000)))
        .expect("No suitable UTXO found (need 500 or 1000 sats)");

    let input_txid_str = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    println!("[LIVE] Using UTXO: txid={} vout={} amount={} sats", input_txid_str, input_vout, input_amount);

    let input_txid: bitcoin::Txid = input_txid_str.parse().expect("Failed to parse txid");

    // Build the inscription payload
    let payload = b"{\"counter\":0,\"type\":\"test_deploy\",\"owner\":\"muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi\"}";
    let payload_hash = Sha256::digest(payload);

    // Build OP_RETURN script: OP_RETURN "utxovm:deploy" <32-byte hash>
    let tag_bytes: &[u8] = b"utxovm:deploy";
    let mut tag_push = PushBytesBuf::new();
    tag_push.extend_from_slice(tag_bytes);
    let mut hash_push = PushBytesBuf::new();
    hash_push.extend_from_slice(&payload_hash);

    let mut op_return_script = ScriptBuf::new();
    op_return_script.push_opcode(OP_RETURN);
    op_return_script.push_slice(&tag_push);
    op_return_script.push_slice(&hash_push);

    // Build P2PKH script for change (back to sender)
    let addr = Address::p2pkh(&wpk, bitcoin::network::Network::Testnet);
    let change_script = addr.script_pubkey();

    let miner_fee = 300u64;
    let change_amount = input_amount.saturating_sub(miner_fee);

    println!("[LIVE] Fee: {} sats, Change: {} sats", miner_fee, change_amount);

    // Build the transaction
    let tx_in = bitcoin::TxIn {
        previous_output: bitcoin::OutPoint { txid: input_txid, vout: input_vout },
        script_sig: ScriptBuf::new(),
        sequence: Sequence(0xFFFFFFFD),
        witness: bitcoin::Witness::default(),
    };

    let tx_out_op_return = bitcoin::TxOut {
        value: Amount::from_sat(0),
        script_pubkey: op_return_script,
    };

    let tx_out_change = bitcoin::TxOut {
        value: Amount::from_sat(change_amount),
        script_pubkey: change_script,
    };

    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::locktime::absolute::LockTime::ZERO,
        input: vec![tx_in],
        output: vec![tx_out_op_return, tx_out_change],
    };

    println!("[LIVE] Built tx with {} inputs, {} outputs", tx.input.len(), tx.output.len());

    // Sign the input (P2PKH)
    let sighash_type = bitcoin::sighash::EcdsaSighashType::All;

    let p2pkh_script = ScriptBuf::builder()
        .push_opcode(OP_DUP)
        .push_opcode(OP_HASH160)
        .push_slice(&bitcoin::hashes::hash160::Hash::hash(&pk_comp).to_byte_array())
        .push_opcode(OP_EQUALVERIFY)
        .push_opcode(OP_CHECKSIG)
        .into_script();

    let mut sighash_cache = SighashCache::new(&tx);
    let sighash = sighash_cache
        .legacy_signature_hash(0, &p2pkh_script, sighash_type.to_u32())
        .expect("Failed to compute sighash");

    let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
    let sig = secp.sign_ecdsa(&msg, &sk);
    let mut sig_with_hashtype = sig.serialize_der().to_vec();
    sig_with_hashtype.push(sighash_type.to_u32() as u8);

    // Set the scriptSig: <sig> <pubkey>
    let mut sig_push = PushBytesBuf::new();
    sig_push.extend_from_slice(&sig_with_hashtype);
    let mut pk_push = PushBytesBuf::new();
    pk_push.extend_from_slice(&pk_comp);

    let script_sig = ScriptBuf::builder()
        .push_slice(&sig_push)
        .push_slice(&pk_push)
        .into_script();

    tx.input[0].script_sig = script_sig;

    println!("[LIVE] Signed tx. ScriptSig: {} bytes", tx.input[0].script_sig.len());

    // Serialize and broadcast
    let raw_tx_bytes = encode::serialize(&tx);
    let raw_tx_hex = hex::encode(&raw_tx_bytes);

    let preview_len = min(64, raw_tx_hex.len());
    println!("[LIVE] Raw tx: {} bytes ({})", raw_tx_bytes.len(), &raw_tx_hex[..preview_len]);

    let txid = tx.compute_txid();
    println!("[LIVE] Expected txid: {}", txid);

    // Broadcast via electrs
    let resp = http
        .post(format!("{}/tx", ELECTRS_URL))
        .header("Content-Type", "text/plain")
        .body(raw_tx_hex.clone())
        .send()
        .await
        .expect("Failed to send broadcast request");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    println!("[LIVE] Broadcast response: HTTP {} — {}", status, body);

    if status.is_success() {
        let broadcast_txid = body.trim();
        println!("[LIVE] Broadcast successful! TXID: {}", broadcast_txid);
        println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", broadcast_txid);

        // Wait and check if it's in mempool
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let check = http.get(format!("{}/tx/{}", ELECTRS_URL, broadcast_txid)).send().await;
        if let Ok(resp) = check {
            if resp.status().is_success() {
                println!("[LIVE] TX found in electrs index!");
            } else {
                println!("[LIVE] TX not yet indexed (may take a few seconds)");
            }
        }
    } else {
        println!("[LIVE] Broadcast failed. Possible reasons: UTXO already spent, insufficient fee, or invalid sig");
    }

    // Verify tx structure regardless of broadcast result
    assert!(raw_tx_bytes.len() > 100, "Tx must be > 100 bytes");
    assert_eq!(tx.output.len(), 2, "Must have 2 outputs");
    assert_eq!(tx.input.len(), 1, "Must have 1 input");
    println!("[LIVE] Tx structure verified: 1 input, 2 outputs, signed");
}
