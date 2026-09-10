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
    for _ in 0..zeros { result.push(ALPHABET[0]); }
    result.reverse();
    String::from_utf8(result).unwrap()
}

fn main() {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap()).unwrap();
    let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pk_comp = pk.serialize();
    let sha = Sha256::digest(&pk_comp);
    let hash160 = Ripemd160::digest(&sha);
    let mut v = vec![0x6fu8];
    v.extend_from_slice(&hash160);
    let cs = Sha256::digest(&Sha256::digest(&v));
    v.extend_from_slice(&cs[..4]);
    println!("Address: {}", base58_encode(&v));
    println!("Expected: muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi");
}
