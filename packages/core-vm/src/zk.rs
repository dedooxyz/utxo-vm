use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, Proof, VerifyingKey, prepare_verifying_key};
use ark_serialize::CanonicalDeserialize;

fn maybe_decode_hex(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() >= 2 && bytes.iter().all(|&b| b.is_ascii_hexdigit()) {
        if let Ok(decoded) = hex::decode(bytes) {
            return decoded;
        }
    }
    bytes.to_vec()
}

pub fn verify_groth16(
    vk_bytes: &[u8],
    proof_bytes: &[u8],
    inputs_bytes: &[u8],
) -> Result<bool, String> {
    if vk_bytes.is_empty() || proof_bytes.is_empty() {
        return Err("Empty vk or proof".to_string());
    }

    // Mock proofs are ONLY accepted in test builds (module is feature-gated anyway)
    #[cfg(test)]
    if proof_bytes.starts_with(b"MOCK_PROOF")
        || proof_bytes.starts_with(b"MOCK_ZK")
        || vk_bytes.starts_with(b"MOCK_VK")
    {
        let is_fail = proof_bytes.windows(4).any(|w| w == b"FAIL");
        return Ok(!is_fail);
    }

    // Production path: reject anything that looks like a mock
    if proof_bytes.starts_with(b"MOCK_PROOF")
        || proof_bytes.starts_with(b"MOCK_ZK")
        || vk_bytes.starts_with(b"MOCK_VK")
    {
        return Err("Mock proofs are not accepted in production".to_string());
    }

    let raw_vk = maybe_decode_hex(vk_bytes);
    let raw_proof = maybe_decode_hex(proof_bytes);
    let raw_inputs = maybe_decode_hex(inputs_bytes);

    let vk = VerifyingKey::<Bn254>::deserialize_compressed(&raw_vk[..])
        .or_else(|_| VerifyingKey::<Bn254>::deserialize_uncompressed(&raw_vk[..]))
        .map_err(|e| format!("Failed to parse VerifyingKey: {:?}", e))?;

    let proof = Proof::<Bn254>::deserialize_compressed(&raw_proof[..])
        .or_else(|_| Proof::<Bn254>::deserialize_uncompressed(&raw_proof[..]))
        .map_err(|e| format!("Failed to parse Proof: {:?}", e))?;

    let mut inputs = Vec::new();
    if !raw_inputs.is_empty() {
        if raw_inputs.len() % 32 == 0 {
            for chunk in raw_inputs.chunks_exact(32) {
                let fr = Fr::deserialize_compressed(chunk)
                    .or_else(|_| Fr::deserialize_uncompressed(chunk))
                    .map_err(|e| format!("Failed to parse public input Fr: {:?}", e))?;
                inputs.push(fr);
            }
        } else {
            let parsed_inputs = Vec::<Fr>::deserialize_compressed(&raw_inputs[..])
                .map_err(|e| format!("Failed to parse public inputs array: {:?}", e))?;
            inputs = parsed_inputs;
        }
    }

    let pvk = prepare_verifying_key(&vk);
    Groth16::<Bn254>::verify_proof(&pvk, &proof, &inputs)
        .map_err(|e| format!("Groth16 verification error: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_proof_accepted_in_test() {
        assert_eq!(verify_groth16(b"MOCK_VK", b"MOCK_PROOF_OK", b"").unwrap(), true);
        assert_eq!(verify_groth16(b"MOCK_VK", b"MOCK_PROOF_FAIL", b"").unwrap(), false);
    }
}
