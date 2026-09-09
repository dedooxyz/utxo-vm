# JKC Chain Work

This document outlines the JKC chain work required for UTXO-VM integration.

## 1. Taproot Activation Plan

### Current Status
- JKC Testnet: Taproot active (confirmed by developer)
- JKC Mainnet: Taproot active (confirmed by developer)
- MWEB: Planned for next month

### Activation Height
- **Testnet:** Already active
- **Mainnet:** Already active

### Required Features
- [x] P2TR (Pay-to-Taproot) support
- [x] Schnorr signatures
- [x] Tapscript (v0/v1)
- [x] Key path spending
- [x] Script path spending
- [ ] MWEB (planned next month)

## 2. Required Opcode List

### Core Opcodes
| Opcode | Status | Purpose |
|:---|:---|:---|
| OP_CHECKSIGADD | ✅ Active | Multi-signature verification |
| OP_CHECKSIG | ✅ Active | Single signature verification |
| OP_CHECKMULTISIG | ✅ Active | Legacy multi-signature |
| OP_HASH160 | ✅ Active | Hash160 for addresses |
| OP_HASH256 | ✅ Active | Double SHA256 |
| OP_SHA256 | ✅ Active | Single SHA256 |
| OP_RIPEMD160 | ✅ Active | RIPEMD160 hash |

### Time Lock Opcodes
| Opcode | Status | Purpose |
|:---|:---|:---|
| OP_CHECKLOCKTIMEVERIFY (CLTV) | ✅ Active | Absolute time lock |
| OP_CHECKSEQUENCEVERIFY (CSV) | ✅ Active | Relative time lock |

### Cryptographic Opcodes
| Opcode | Status | Purpose |
|:---|:---|:---|
| OP_CAT | ✅ Active | Concatenate two byte arrays |
| OP_SPLIT | ✅ Active | Split byte array at position |
| OP_SIZE | ✅ Active | Push size of stack item |
| OP_LEFT | ✅ Active | Take left bytes |
| OP_RIGHT | ✅ Active | Take right bytes |

### Control Flow Opcodes
| Opcode | Status | Purpose |
|:---|:---|:---|
| OP_IF | ✅ Active | Conditional execution |
| OP_NOTIF | ✅ Active | Negative conditional |
| OP_ELSE | ✅ Active | Alternative branch |
| OP_ENDIF | ✅ Active | End conditional |
| OP_VERIFY | ✅ Active | Verify top stack item |
| OP_RETURN | ✅ Active | Mark transaction as invalid |

### Stack Opcodes
| Opcode | Status | Purpose |
|:---|:---|:---|
| OP_DUP | ✅ Active | Duplicate top item |
| OP_DROP | ✅ Active | Remove top item |
| OP_SWAP | ✅ Active | Swap top two items |
| OP_OVER | ✅ Active | Copy second item to top |
| OP_ROT | ✅ Active | Rotate top three items |
| OP_PICK | ✅ Active | Copy Nth item to top |
| OP_ROLL | ✅ Active | Move Nth item to top |
| OP_2DUP | ✅ Active | Duplicate top two items |
| OP_3DUP | ✅ Active | Duplicate top three items |

## 3. Standard Address Format

### P2TR (Pay-to-Taproot)
```
Bech32m encoding
Prefix: "tjkc" (testnet), "jkc" (mainnet)
Witness version: 1
Program: 32-byte x-only public key
```

### P2WPKH (Pay-to-Witness-Public-Key-Hash)
```
Bech32 encoding
Prefix: "tjkc" (testnet), "jkc" (mainnet)
Witness version: 0
Program: 20-byte HASH160(public_key)
```

### P2WSH (Pay-to-Witness-Script-Hash)
```
Bech32 encoding
Prefix: "tjkc" (testnet), "jkc" (mainnet)
Witness version: 0
Program: 32-byte SHA256(script)
```

## 4. Commitment Encoding

### OP_RETURN Commitment
```
OP_FALSE OP_IF
  OP_PUSH "utxovm"           // Protocol identifier
  OP_PUSH 0x01               // Protocol version
  OP_PUSH "application/wasm" // Content type
  OP_PUSH <PAYLOAD>          // WASM bytecode or calldata
OP_ENDIF
```

### State Root Commitment
```
OP_FALSE OP_IF
  OP_PUSH "utxovm"
  OP_PUSH 0x01
  OP_PUSH "application/json"
  OP_PUSH <STATE_ROOT_JSON>  // {"root":"...","block":12345}
OP_ENDIF
```

### Batch Commitment
```
OP_FALSE OP_IF
  OP_PUSH "utxovm"
  OP_PUSH 0x01
  OP_PUSH "application/json"
  OP_PUSH <BATCH_JSON>       // {"batch":123,"root":"...","prev":"..."}
OP_ENDIF
```

## 5. Script Templates

### Vault Script (P2TR)
```rust
// Key path: operator can spend after unbond delay
// Script path: challenger can slash with proof
let vault_script = script! {
    // If operator key path
    if { check_multisig(2, vec![operator_key, operator_key_2]) } then {
        // Check unbond delay
        check_sequence_verify(UNBOND_DELAY)
    }
    // If challenger script path
    else if { check_multisig(2, vec![challenger_key, operator_key]) } then {
        // OP_CAT hash comparison
        // Verify fraud proof
    }
}
```

### Challenge Script
```rust
let challenge_script = script! {
    // Challenger key
    OP_CHECKSIGVERIFY
    // Fraud proof hash
    OP_SHA256
    OP_EQUALVERIFY
    // State root
    OP_SHA256
    OP_EQUAL
}
```

### Seal Spend Script
```rust
let seal_script = script! {
    // Seal binding: txid:vout
    OP_HASH160
    OP_EQUALVERIFY
    // Operator signature
    OP_CHECKSIG
}
```

## 6. QA Considerations

### Old Nodes
- Old nodes will see UTXO-VM transactions as standard OP_RETURN
- No consensus changes required
- Backward compatible

### Inscriptions
- UTXO-VM uses similar envelope format to inscriptions
- Protocol identifier "utxovm" differentiates from other inscriptions
- No conflict with existing inscription protocols

### Merge-Mine
- JKC supports merge-mining with other chains
- UTXO-VM transactions are standard Bitcoin transactions
- No special merge-mine handling required

## 7. Implementation Status

| Component | Status | Notes |
|:---|:---|:---|
| P2TR support | ✅ Active | Confirmed by developer |
| OP_CAT | ✅ Active | Confirmed by developer |
| CHECKSIGADD | ✅ Active | Confirmed by developer |
| CLTV/CSV | ✅ Active | Confirmed by developer |
| MWEB | ⏳ Planned | Next month |

## 8. Testing

### Testnet Testing
- [x] P2TR vault deployment
- [x] OP_CAT challenge script
- [x] Seal spend script
- [x] Batch commitment
- [ ] Full Taproot script path spending

### Mainnet Deployment
- [ ] Taproot activation verification
- [ ] Opcode availability verification
- [ ] Address format verification
