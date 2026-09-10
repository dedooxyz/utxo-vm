use crate::types::UtxoVmEnvelope;

pub fn parse_envelope(script_bytes: &[u8]) -> Option<UtxoVmEnvelope> {
    if script_bytes.len() < 10 {
        return None;
    }

    // Check for OP_FALSE (0x00) OP_IF (0x63) pattern
    let mut offset = 0;
    while offset + 8 < script_bytes.len() {
        if script_bytes[offset] == 0x00 && script_bytes[offset + 1] == 0x63 {
            // Found OP_FALSE OP_IF
            offset += 2;
            // Next should push "utxovm"
            let tag_len = script_bytes[offset] as usize;
            offset += 1;
            if offset + tag_len <= script_bytes.len() && &script_bytes[offset..offset + tag_len] == b"utxovm" {
                offset += tag_len;

                // Version byte
                let ver_len = script_bytes[offset] as usize;
                offset += 1;
                let version = if ver_len == 1 && offset < script_bytes.len() {
                    script_bytes[offset]
                } else {
                    1
                };
                offset += ver_len;

                // Content type
                let ct_len = script_bytes[offset] as usize;
                offset += 1;
                let content_type = if offset + ct_len <= script_bytes.len() {
                    String::from_utf8_lossy(&script_bytes[offset..offset + ct_len]).to_string()
                } else {
                    "application/json".to_string()
                };
                offset += ct_len;

                // Payload
                let mut payload = Vec::new();
                while offset < script_bytes.len() && script_bytes[offset] != 0x68 {
                    let chunk_len = script_bytes[offset] as usize;
                    offset += 1;
                    if offset + chunk_len <= script_bytes.len() {
                        payload.extend_from_slice(&script_bytes[offset..offset + chunk_len]);
                        offset += chunk_len;
                    } else {
                        break;
                    }
                }

                let mut metadata = None;
                if content_type == "application/json" {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&payload) {
                        metadata = Some(json);
                    }
                }

                return Some(UtxoVmEnvelope {
                    protocol: "utxovm".to_string(),
                    version,
                    content_type,
                    payload,
                    metadata,
                });
            }
        }
        offset += 1;
    }

    // Check OP_RETURN format: 0x6a ...
    if script_bytes.starts_with(&[0x6a]) {
        let mut offset = 1;
        // Handle PUSHDATA opcodes after OP_RETURN
        if offset < script_bytes.len() {
            match script_bytes[offset] {
                // OP_PUSHBYTES_1..75 (0x01..0x4b) - inline push
                0x01..=0x4b => {
                    let push_len = script_bytes[offset] as usize;
                    offset += 1;
                    offset += push_len.min(script_bytes.len().saturating_sub(offset));
                }
                // OP_PUSHDATA1 (0x4c)
                0x4c => {
                    offset += 1;
                    if offset < script_bytes.len() {
                        let push_len = script_bytes[offset] as usize;
                        offset += 1;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                // OP_PUSHDATA2 (0x4d)
                0x4d => {
                    offset += 1;
                    if offset + 1 < script_bytes.len() {
                        let push_len = u16::from_le_bytes([script_bytes[offset], script_bytes[offset + 1]]) as usize;
                        offset += 2;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                // OP_PUSHDATA4 (0x4e)
                0x4e => {
                    offset += 1;
                    if offset + 3 < script_bytes.len() {
                        let push_len = u32::from_le_bytes([
                            script_bytes[offset], script_bytes[offset + 1],
                            script_bytes[offset + 2], script_bytes[offset + 3],
                        ]) as usize;
                        offset += 4;
                        offset += push_len.min(script_bytes.len().saturating_sub(offset));
                    }
                }
                _ => {}
            }
        }
        // Now scan for utxovm tag in the remaining bytes
        while offset + 8 < script_bytes.len() {
            if script_bytes[offset] == 0x00 && script_bytes[offset + 1] == 0x63 {
                offset += 2;
                let tag_len = script_bytes[offset] as usize;
                offset += 1;
                if offset + tag_len <= script_bytes.len()
                    && &script_bytes[offset..offset + tag_len] == b"utxovm"
                {
                    offset += tag_len;
                    // Version byte
                    let ver_len = script_bytes[offset] as usize;
                    offset += 1;
                    let version = if ver_len == 1 && offset < script_bytes.len() {
                        script_bytes[offset]
                    } else {
                        1
                    };
                    offset += ver_len;

                    // Content type
                    let ct_len = script_bytes[offset] as usize;
                    offset += 1;
                    let content_type = if offset + ct_len <= script_bytes.len() {
                        String::from_utf8_lossy(&script_bytes[offset..offset + ct_len]).to_string()
                    } else {
                        "application/json".to_string()
                    };
                    offset += ct_len;

                    // Payload
                    let mut payload = Vec::new();
                    while offset < script_bytes.len() && script_bytes[offset] != 0x68 {
                        let chunk_len = script_bytes[offset] as usize;
                        offset += 1;
                        if offset + chunk_len <= script_bytes.len() {
                            payload.extend_from_slice(&script_bytes[offset..offset + chunk_len]);
                            offset += chunk_len;
                        } else {
                            break;
                        }
                    }

                    let mut metadata = None;
                    if content_type == "application/json" {
                        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&payload) {
                            metadata = Some(json);
                        }
                    }

                    return Some(UtxoVmEnvelope {
                        protocol: "utxovm".to_string(),
                        version,
                        content_type,
                        payload,
                        metadata,
                    });
                }
            }
            offset += 1;
        }
    }

    None
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
