use crate::types::UtxoVmEnvelope;

/// Parse a utxovm envelope from script bytes.
/// Returns None if the script is malformed, truncated, or doesn't contain a valid envelope.
/// All offset accesses are bounds-checked to prevent panics on malformed input.
pub fn parse_envelope(script_bytes: &[u8]) -> Option<UtxoVmEnvelope> {
    if script_bytes.len() < 10 {
        return None;
    }

    // Try OP_FALSE OP_IF path (witness-style inscription)
    if let Some(env) = parse_if_envelope(script_bytes, 0) {
        return Some(env);
    }

    // Try OP_RETURN path
    if script_bytes[0] == 0x6a {
        // Skip the OP_RETURN push data prefix
        let mut offset = 1;
        if offset < script_bytes.len() {
            match script_bytes[offset] {
                0x01..=0x4b => {
                    let push_len = script_bytes[offset] as usize;
                    offset += 1;
                    offset += push_len.min(script_bytes.len().saturating_sub(offset));
                }
                0x4c => {
                    offset += 1;
                    if offset < script_bytes.len() {
                        let push_len = script_bytes[offset] as usize;
                        offset += 1;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                0x4d => {
                    offset += 1;
                    if offset + 1 < script_bytes.len() {
                        let push_len = u16::from_le_bytes([
                            script_bytes[offset],
                            script_bytes[offset + 1],
                        ]) as usize;
                        offset += 2;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                0x4e => {
                    offset += 1;
                    if offset + 3 < script_bytes.len() {
                        let push_len = u32::from_le_bytes([
                            script_bytes[offset],
                            script_bytes[offset + 1],
                            script_bytes[offset + 2],
                            script_bytes[offset + 3],
                        ]) as usize;
                        offset += 4;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                _ => {}
            }
        }
        // Scan for OP_FALSE OP_IF within the OP_RETURN payload
        while offset + 8 < script_bytes.len() {
            if let Some(env) = parse_if_envelope(script_bytes, offset) {
                return Some(env);
            }
            offset += 1;
        }
    }

    None
}

/// Parse an OP_FALSE OP_IF envelope starting at the given offset.
/// All accesses are bounds-checked. Returns None if the envelope is
/// malformed or truncated.
fn parse_if_envelope(script: &[u8], start: usize) -> Option<UtxoVmEnvelope> {
    // Need at least: OP_FALSE OP_IF <tag_len> "utxovm" <ver_len> <ver> <ct_len> <ct> <payload...> OP_ENDIF
    if start + 10 > script.len() {
        return None;
    }

    // Check OP_FALSE OP_IF
    if script[start] != 0x00 || script[start + 1] != 0x63 {
        return None;
    }

    let mut offset = start + 2;

    // Read tag length and check "utxovm"
    let tag_len = script[offset] as usize;
    offset += 1;
    if offset + tag_len > script.len() {
        return None;
    }
    if &script[offset..offset + tag_len] != b"utxovm" {
        return None;
    }
    offset += tag_len;

    // Read version
    if offset >= script.len() {
        return None;
    }
    let ver_len = script[offset] as usize;
    offset += 1;
    let version = if ver_len == 1 && offset < script.len() {
        let v = script[offset];
        offset += 1;
        v
    } else {
        // Skip ver_len bytes if possible, else default
        if offset + ver_len > script.len() {
            return None;
        }
        offset += ver_len;
        1
    };

    // Read content type
    if offset >= script.len() {
        return None;
    }
    let ct_len = script[offset] as usize;
    offset += 1;
    if offset + ct_len > script.len() {
        return None;
    }
    let content_type = String::from_utf8_lossy(&script[offset..offset + ct_len]).to_string();
    offset += ct_len;

    // Read payload chunks until OP_ENDIF (0x68) or end of script
    let mut payload = Vec::new();
    let mut found_endif = false;
    while offset < script.len() {
        if script[offset] == 0x68 {
            found_endif = true;
            break;
        }
        // Support OP_PUSHDATA1/2/4 for large payloads
        let chunk_len = match script[offset] {
            0x01..=0x4b => {
                let len = script[offset] as usize;
                offset += 1;
                len
            }
            0x4c => {
                offset += 1;
                if offset >= script.len() {
                    break;
                }
                let len = script[offset] as usize;
                offset += 1;
                len
            }
            0x4d => {
                offset += 1;
                if offset + 1 >= script.len() {
                    break;
                }
                let len = u16::from_le_bytes([script[offset], script[offset + 1]]) as usize;
                offset += 2;
                len
            }
            0x4e => {
                offset += 1;
                if offset + 3 >= script.len() {
                    break;
                }
                let len = u32::from_le_bytes([
                    script[offset],
                    script[offset + 1],
                    script[offset + 2],
                    script[offset + 3],
                ]) as usize;
                offset += 4;
                len
            }
            0x00 => {
                // OP_FALSE — skip
                offset += 1;
                continue;
            }
            _ => {
                // Unknown opcode — stop parsing
                break;
            }
        };

        if offset + chunk_len > script.len() {
            break;
        }
        payload.extend_from_slice(&script[offset..offset + chunk_len]);
        offset += chunk_len;
    }

    if !found_endif {
        // Truncated envelope — no OP_ENDIF found
        return None;
    }

    let mut metadata = None;
    if content_type == "application/json" {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&payload) {
            metadata = Some(json);
        }
    }

    Some(UtxoVmEnvelope {
        protocol: "utxovm".to_string(),
        version,
        content_type,
        payload,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_op_return_envelope() {
        // Construct a proper OP_RETURN envelope per AGENTS.md §2.1:
        // OP_RETURN OP_FALSE OP_IF <push "utxovm"> <push 0x01>
        //   <push "application/json"> <push payload> OP_ENDIF
        let payload = br#"{"method":"transfer","to":"bob","amount":50}"#;
        let mut script = vec![0x6a]; // OP_RETURN
        script.push(0x00); // OP_FALSE
        script.push(0x63); // OP_IF
        script.push(0x06); // push 6 bytes
        script.extend_from_slice(b"utxovm");
        script.push(0x01); // push 1 byte (version)
        script.push(0x01); // version = 1
        script.push(0x10); // push 16 bytes (content type)
        script.extend_from_slice(b"application/json");
        script.push(payload.len() as u8); // push payload
        script.extend_from_slice(payload);
        script.push(0x68); // OP_ENDIF

        let env = parse_envelope(&script).expect("Envelope should be parsed");
        assert_eq!(env.protocol, "utxovm");
        assert!(env.metadata.is_some());
    }
}
