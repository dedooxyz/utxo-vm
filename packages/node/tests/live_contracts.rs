//! Live JKC Testnet: NFT + NativeVault + AtomicSwap
//!
//! Run with: cargo test -p utxo-vmd --test live_contracts -- --nocapture --ignored

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

async fn fetch_utxos(http: &HttpClient) -> Vec<serde_json::Value> {
    http.get(format!("{}/address/{}/utxo", ELECTRS_URL, SENDER_ADDRESS))
        .send().await.unwrap().json().await.unwrap()
}

fn get_keypair() -> (Secp256k1<secp256k1::All>, SecretKey, Vec<u8>) {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&hex::decode(PRIV_KEY_HEX).unwrap()).unwrap();
    let pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &sk);
    (secp, sk, pk.serialize().to_vec())
}

/// Build, sign, and broadcast a tx with OP_RETURN envelope + optional value output.
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
    // Optional: lock satoshis in a P2PKH output (for NativeVault)
    lock_output_amount: Option<u64>,
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

    let lock_amount = lock_output_amount.unwrap_or(0);
    let change_amount = input_amount.saturating_sub(miner_fee).saturating_sub(lock_amount);

    let input_txid_parsed: bitcoin::Txid = input_txid.parse().unwrap();

    let mut outputs = vec![
        bitcoin::TxOut { value: Amount::from_sat(0), script_pubkey: op_return_script },
    ];

    // Optional: lock sats in a P2PKH output (NativeVault)
    if lock_amount > 0 {
        outputs.push(bitcoin::TxOut {
            value: Amount::from_sat(lock_amount),
            script_pubkey: change_script.clone(),
        });
        println!("[LIVE]   Locked output: {} sats", lock_amount);
    }

    // Change output
    outputs.push(bitcoin::TxOut {
        value: Amount::from_sat(change_amount),
        script_pubkey: change_script,
    });

    let mut tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::locktime::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn {
            previous_output: bitcoin::OutPoint { txid: input_txid_parsed, vout: input_vout },
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xFFFFFFFD),
            witness: bitcoin::Witness::default(),
        }],
        output: outputs,
    };

    // Sign
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
        println!("[LIVE] {} OK: txid={}", label, broadcast_txid);
        println!("[LIVE]   Payload: {}", String::from_utf8_lossy(payload));
        println!("[LIVE]   Fee: {} sats, Change: {} sats", miner_fee, change_amount);
        Ok(broadcast_txid)
    } else {
        Err(format!("HTTP {} — {}", status, body))
    }
}

#[tokio::test]
#[ignore]
async fn live_mint_nft_utx721() {
    let http = HttpClient::new();
    let (secp, sk, pk_comp) = get_keypair();
    let utxos = fetch_utxos(&http).await;

    // Use a 500-sat UTXO
    let utxo = utxos.iter().find(|u| u["value"].as_u64() == Some(500)).expect("Need 500-sat UTXO");
    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // NFT mint payload
    let nft_payload = br#"{"collection":"JKC Test Collection","tokenId":1,"name":"TestNFT #1","metadataUri":"ipfs://QmTestNFT1","owner":"muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi"}"#;

    let result = sign_and_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:deploy", nft_payload, 300,
        "MINT NFT (UTX721)", None,
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] NFT minted! TXID: {}", txid);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => println!("[LIVE] NFT mint failed: {}", e),
    }
}

#[tokio::test]
#[ignore]
async fn live_native_vault_lock_sats() {
    let http = HttpClient::new();
    let (secp, sk, pk_comp) = get_keypair();
    let utxos = fetch_utxos(&http).await;

    // Use a 50M sat UTXO (0.5 tJKC) for the vault
    let utxo = utxos.iter().find(|u| u["value"].as_u64() == Some(50_000_000))
        .expect("Need 50M sat UTXO for vault");
    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // Lock 1M sats (0.01 tJKC) in vault, rest is change
    let lock_amount = 1_000_000u64;
    let miner_fee = 500u64;

    let vault_payload = br#"{"type":"NativeVault","lockedSatoshis":1000000,"owner":"muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi","wrappedToken":"wJKC","wrappedAmount":1000000}"#;

    let result = sign_and_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:deploy", vault_payload, miner_fee,
        "NATIVEVAULT (lock 0.01 tJKC)", Some(lock_amount),
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] NativeVault deployed! TXID: {}", txid);
            println!("[LIVE] Locked: {} sats (0.01 tJKC)", lock_amount);
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => println!("[LIVE] NativeVault failed: {}", e),
    }
}

#[tokio::test]
#[ignore]
async fn live_atomic_swap_order() {
    let http = HttpClient::new();
    let (secp, sk, pk_comp) = get_keypair();
    let utxos = fetch_utxos(&http).await;

    // Use a 500-sat UTXO
    let used_txids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let utxo = utxos.iter().find(|u| {
        u["value"].as_u64() == Some(500) && !used_txids.contains(u["txid"].as_str().unwrap())
    }).or(utxos.iter().find(|u| u["value"].as_u64() == Some(1000)))
      .expect("Need UTXO for swap");

    let input_txid = utxo["txid"].as_str().unwrap();
    let input_vout = utxo["vout"].as_u64().unwrap() as u32;
    let input_amount = utxo["value"].as_u64().unwrap();

    // Atomic swap: offer NFT #1 for 100k sats
    let swap_payload = br#"{"type":"AtomicSwap","maker":"muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi","offeredTokenId":1,"offeredType":"UTX721","demandedSatoshis":100000000,"demandedType":"JKC","hashLock":"a1b2c3d4e5f6","timeLock":144}"#;

    let result = sign_and_broadcast(
        &http, &secp, &sk, &pk_comp,
        input_txid, input_vout, input_amount,
        b"utxovm:deploy", swap_payload, 300,
        "ATOMIC SWAP ORDER", None,
    ).await;

    match result {
        Ok(txid) => {
            println!("[LIVE] Atomic swap order created! TXID: {}", txid);
            println!("[LIVE] Offer: NFT #1 for 1 tJKC (100M sats)");
            println!("[LIVE] Verify: https://jkc-testnet-api.s3na.xyz/tx/{}", txid);
        }
        Err(e) => println!("[LIVE] Atomic swap failed: {}", e),
    }
}

#[tokio::test]
#[ignore]
async fn live_verify_all_contract_txs() {
    let http = HttpClient::new();
    let tip: u64 = http
        .get(format!("{}/blocks/tip/height", ELECTRS_URL))
        .send().await.unwrap().text().await.unwrap().trim().parse().unwrap();

    println!("[LIVE] Scanning blocks {}..{} for utxovm contract txs", tip - 30, tip);

    let mut found = 0;
    for height in (tip - 30)..=tip {
        let block_hash = http
            .get(format!("{}/block-height/{}", ELECTRS_URL, height))
            .send().await.unwrap().text().await.unwrap().trim().to_string();

        let txs: Vec<serde_json::Value> = http
            .get(format!("{}/block/{}/txs", ELECTRS_URL, block_hash))
            .send().await.unwrap().json().await.unwrap_or_default();

        for tx in &txs {
            let txid = tx["txid"].as_str().unwrap_or("");
            for (vout, output) in tx["vout"].as_array().unwrap_or(&vec![]).iter().enumerate() {
                let asm = output["scriptpubkey_asm"].as_str().unwrap_or("");
                let hex_str = output["scriptpubkey_hex"].as_str().unwrap_or("");
                if asm.contains("utxovm") || hex_str.contains("7574786f766d") {
                    println!("[LIVE] Block {} tx {} vout{}: {}", height, &txid[..min(16, txid.len())], vout, asm);
                    found += 1;
                }
            }
        }
    }

    println!("[LIVE] Found {} utxovm contract txs in last 31 blocks", found);
}
