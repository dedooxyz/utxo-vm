//! Generate a new JKC testnet wallet for live testing.
//!
//! Run with: cargo run -p utxo-vmd --example gen_wallet

use secp256k1::{Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use ripemd::Ripemd160;

fn base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut result = Vec::new();
    let mut num = data.to_vec();
    let zeros = num.iter().take_while(|&&b| b == 0).count();
    while !num.is_empty() && num.iter().any(|&b| b != 0) {
        let mut remainder = 0u32;
        for byte in num.iter_mut() {
            let acc = (remainder << 8) | *byte as u32;
            *byte = (acc / 58) as u8;
            remainder = acc % 58;
        }
        result.push(ALPHABET[remainder as usize]);
    }
    for _ in 0..zeros {
        result.push(ALPHABET[0]);
    }
    result.reverse();
    String::from_utf8(result).unwrap()
}

fn main() {
    let secp = Secp256k1::new();
    let (sk, pk) = secp.generate_keypair(&mut secp256k1::rand::rngs::OsRng);

    let sk_hex = hex::encode(&sk.secret_bytes());
    let pk_comp = pk.serialize();
    let pk_hex = hex::encode(&pk_comp);

    // P2PKH: version 0x6f (testnet) + HASH160(pubkey) + base58check
    let sha = Sha256::digest(&pk_comp);
    let hash160 = Ripemd160::digest(&sha);

    let mut versioned = vec![0x6f]; // JKC testnet P2PKH prefix
    versioned.extend_from_slice(&hash160);

    let checksum = Sha256::digest(&Sha256::digest(&versioned));
    versioned.extend_from_slice(&checksum[..4]);

    let address = base58_encode(&versioned);

    println!("=== New JKC Testnet Wallet ===");
    println!("Private Key (hex): {}", sk_hex);
    println!("Public Key (hex):  {}", pk_hex);
    println!("Address (tJKC):    {}", address);
    println!("");
    println!("Fund this address with tJKC to run live write tests.");
    println!("Set env: export JKC_TEST_PRIV={}", sk_hex);
}
