# JKC Chain Work

This document outlines the JKC chain work required for UTXO-VM integration.

## 1. Taproot Activation Plan

### Current Status (empirically verified at JKC testnet block 177,269 via `getblockchaininfo` RPC)

| Soft fork | Testnet | Mainnet |
|:---|:---|:---|
| CSV (BIP-68) | ✅ Active (h=120,000) | ❌ Not yet active (scheduled h=1,145,000, junkcoin-core v4.0.3) |
| SegWit (BIP-141) | ✅ Active (h=140,000) | ❌ Not yet active (scheduled h=1,145,000, concurrent with CSV) |
| Taproot (BIP-341) | ✅ Active (h=160,000) | ❌ Not yet active (scheduled h=1,155,000) |
| CLTV (BIP-65) | ❌ Not active (bip65=99,999,999) | ✅ Active |
| OP_CAT | ✅ Active (confirmed by developer) | ❌ Not yet active (scheduled h=1,155,000, concurrent with Taproot) |
| MWEB | ✅ Active (verified h=183,374 — HogEx tx present in blocks) | ❌ Not yet active (scheduled h=1,165,000) |

> Source: `packages/node/src/consensus/l1_scripts.rs` header comment (testnet);
> mainnet schedule from [junkcoin-core v4.0.3](https://github.com/Junkcoin-Foundation/junkcoin/releases/tag/v4.0.3)
> (released 2026-09-12). Mainnet height at investigation: 1,130,195 — all softforks below
> 1,145,000 are NOT yet active. Bonding (`--bonding-enabled`) is testnet-only until CSV
> activates at 1,145,000.
> Note: CLTV is available on mainnet but NOT on testnet. Testnet scripts use CSV only.
> OP_CAT is active on testnet — not used in live bonding scripts (covenants.rs deleted in Issue 14).

### Required Features
- [x] P2TR (Pay-to-Taproot) support
- [x] Schnorr signatures
- [x] Tapscript (v0/v1)
- [x] Key path spending
- [x] Script path spending
- [x] MWEB (active on testnet — verified h=183,374, HogEx tx present)

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
| Opcode | Testnet | Mainnet | Purpose |
|:---|:---|:---|:---|
| OP_CHECKSEQUENCEVERIFY (CSV) | ✅ Active (h=120,000) | ✅ Active | Relative time lock |
| OP_CHECKLOCKTIMEVERIFY (CLTV) | ❌ Not active (bip65=99,999,999) | ✅ Active | Absolute time lock (mainnet only) |

### Cryptographic Opcodes
| Opcode | Testnet | Mainnet | Purpose |
|:---|:---|:---|:---|
| OP_CAT | ✅ Active | ✅ Active | Concatenate two byte arrays |
| OP_SPLIT | ✅ Active | ✅ Active | Split byte array at position |
| OP_SIZE | ✅ Active | ✅ Active | Push size of stack item |
| OP_EQUAL / OP_EQUALVERIFY | ✅ Active | ✅ Active | Hash comparison (universal fallback) |

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

### Vault Script (P2TR, Model B — committee-gated)
```rust
// Key path: operator can spend after unbond delay (CSV)
// Script path: M-of-N watcher committee can spend to challenge UTXO
//
// Leaf 0 (unbond): <unbond_delay> CSV DROP <operator_pubkey> CHECKSIG
// Leaf 1 (challenge): <pk1> CHECKSIG <pk2> CHECKSIGADD ... <pkN> CHECKSIGADD <M> OP_EQUAL
//                      (BIP-342: individual CHECKSIG + CHECKSIGADD, NOT CHECKMULTISIG)
//
// Challenge UTXO second stage:
//   Claim leaf: <claim_delay> CSV DROP <challenger_pubkey> CHECKSIG
//   Rebut leaf: <operator_pubkey> CHECKSIG (no timelock)
//
// No CLTV, no OP_CAT. CSV + Taproot only.
// See packages/node/src/consensus/l1_scripts.rs for the implementation.
```

### Challenge Script (no OP_CAT)
```rust
// Equivocation proof verified OFF-CHAIN (two signed attestations,
// different roots, same operator). L1 only checks committee authorization.
// BIP-342: use CHECKSIGADD for M-of-N, NOT CHECKMULTISIG (disabled in tapscript).
let challenge_script = script! {
    // Committee authorization (M-of-N watchers, BIP-342)
    <pk1> OP_CHECKSIG <pk2> OP_CHECKSIGADD ... <pkN> OP_CHECKSIGADD <M> OP_EQUAL
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
| P2TR support | ✅ Active | Verified at h=160,000 |
| CSV (BIP-68) | ✅ Active | Verified at h=120,000 |
| SegWit (BIP-141) | ✅ Active | Verified at h=140,000 |
| CHECKSIGADD | ✅ Active | Tapscript |
| CLTV (BIP-65) | ✅ Mainnet / ❌ Testnet | Testnet bip65=99,999,999; mainnet active |
| OP_CAT | ✅ Active | Testnet confirmed; not used in live bonding scripts (covenants.rs deleted in Issue 14) |
| MWEB | ✅ Active (testnet) | Verified h=183,374 — blocks contain HogEx txs (OP_8 witness-v8 programs). Peg-out from contracts still requires an MWEB-capable wallet (kernels/rangeproofs). |

## 8. Testing

### Testnet Testing
- [x] P2TR vault deployment
- [x] CSV-based challenge/silence scripts
- [x] Seal spend script
- [x] Batch commitment
- [x] Full Taproot script path spending (BIP-341 script-path spend confirmed on JKC testnet, 2026-09-10; tx `76ee0e2f...`)
- [x] BIP-342 compliance: CHECKSIGADD (not CHECKMULTISIG), x-only pubkeys, TapSighashType::All

### Mainnet Deployment
- [x] Taproot activation verification (mainnet) — scheduled h=1,155,000 (v4.0.3), NOT yet active at h=1,130,195
- [x] CSV/SegWit availability verification (mainnet) — scheduled h=1,145,000 (v4.0.3), NOT yet active at h=1,130,195
- [x] CLTV status verification (mainnet active)
- [x] OP_CAT status verification (testnet active; mainnet scheduled h=1,155,000)
- [ ] Address format verification
