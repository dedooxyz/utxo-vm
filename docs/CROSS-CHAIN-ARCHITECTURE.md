# Cross-Chain Settlement Architecture

> **Status: FUTURE / OUT OF SCOPE FOR v1.**
> UTXO-VM v1 is JKC-only (seal-native WASM on JKC UTXOs). Cross-chain anchoring is a **future, optional** capability — not a required architecture. Hub-and-spoke DOGE/LTC/BTC is NOT part of the v1 product. See `MISSION.md` and `AGENTS.md` for the v1 scope.

## Problem
AuxPow chains (Junkcoin, Dogecoin, Litecoin forks) merged mine with Bitcoin/Litecoin for security, but have NO settlement layer for:
- State transitions between chains
- Cross-chain asset transfers
- Contract verification across chains

## Solution: Junkcoin as Settlement Layer (FUTURE)

### Architecture
```
                    ┌─────────────────────────────────────┐
                    │         BITCOIN (PoW)               │
                    │    Merged Mining (AuxPow)           │
                    └──────────────┬──────────────────────┘
                                   │
                    ┌──────────────▼──────────────────────┐
                    │      JUNKCOIN (Settlement Layer)    │
                    │  ┌────────────────────────────────┐ │
                    │  │ UTXO-VM Runtime                 │ │
                    │  │ - Seal Chaining                 │ │
                    │  │ - State Transitions             │ │
                    │  │ - Cross-Chain Verification      │ │
                    │  └────────────────────────────────┘ │
                    │  ┌────────────────────────────────┐ │
                    │  │ State Anchors                   │ │
                    │  │ - Chain A: merkle_root          │ │
                    │  │ - Chain B: merkle_root          │ │
                    │  │ - Chain C: merkle_root          │ │
                    │  └────────────────────────────────┘ │
                    └──────────────┬──────────────────────┘
                                   │
          ┌────────────────────────┼────────────────────────┐
          │                        │                        │
   ┌──────▼──────┐          ┌──────▼──────┐          ┌──────▼──────┐
   │  Chain A    │          │  Chain B    │          │  Chain C    │
   │  (testnet)  │          │  (dogecoin) │          │  (ltc fork) │
   └─────────────┘          └─────────────┘          └─────────────┘
```

### Cross-Chain Envelope Format

#### 1. State Anchor (Child Chain → Junkcoin)
```json
{
  "protocol": "utxovm",
  "version": 1,
  "contentType": "application/json",
  "payload": {
    "type": "state_anchor",
    "chainId": "chain_a_testnet",
    "merkleRoot": "0xabc123...",
    "blockHeight": 12345,
    "blockHash": "0xdef456...",
    "stateRoot": "0x789abc...",
    "prevSeal": "txid:vout"
  }
}
```

#### 2. State Claim (Junkcoin → Child Chain)
```json
{
  "type": "state_claim",
  "chainId": "chain_a_testnet",
  "junkcoinSeal": "txid:vout",
  "merkleRoot": "0xabc123...",
  "proof": ["0x...", "0x..."],  // Merkle proof
  "signatures": ["0x..."]       // Validator signatures
}
```

#### 3. Cross-Chain Transfer
```json
{
  "type": "cross_chain_transfer",
  "fromChain": "chain_a_testnet",
  "toChain": "chain_b_testnet",
  "asset": "JKCT",
  "amount": 10000,
  "sender": "addr_a",
  "receiver": "addr_b",
  "sourceSeal": "txid:vout",
  "destSeal": null
}
```

### Verification Flow

#### Child Chain A wants to verify state on Junkcoin:

1. **A broadcasts state anchor to Junkcoin**
   ```
   OP_RETURN OP_FALSE OP_IF "utxovm" 1 "application/json" <state_anchor> OP_ENDIF
   ```

2. **Junkcoin indexer stores the anchor**
   - Seal: txid:vout
   - State: merkleRoot, blockHeight, etc.

3. **A creates state_claim on its own chain**
   - Includes Junkcoin seal
   - Includes merkle proof
   - Validators sign

4. **B receives state_claim from A**
   - B queries Junkcoin for seal existence
   - B verifies merkle proof
   - B accepts state

### Security Model

#### Who validates?
- Junkcoin full nodes verify envelopes
- Child chain validators verify seals
- Cross-chain relayers propagate state

#### What prevents fraud?
- Seals are UTXOs (can only be spent once)
- Merkle proofs are deterministic
- Signatures are cryptographic

#### What if Junkcoin reorgs?
- Child chains should wait for N confirmations
- Economic finality increases with depth

### Economic Model

#### Fee Structure
- State anchor: minimal fee (data storage)
- State claim: child chain fee
- Cross-chain transfer: combined fees

#### Incentives
- Relayers earn fees for propagating state
- Validators earn fees for signing claims
- Junkcoin miners earn fees for anchoring state

### Implementation Roadmap (FUTURE — not v1)

1. **Phase 1: State Anchoring** ⏳ Future
   - Child chain broadcasts merkle root to Junkcoin
   - Junkcoin stores and seals

2. **Phase 2: Seal Verification** ⏳ Future
   - Child chain queries Junkcoin for seal
   - Merkle proof verification

3. **Phase 3: Cross-Chain Transfer** ⏳ Future
   - Atomic swaps via seals
   - Bridge contracts

4. **Phase 4: Full Settlement** ⏳ Future
   - Multi-chain state sync
   - Governance and upgradeability

> **Note:** v1 ships JKC-only. Cross-chain is explicitly out of scope until the JKC court (equivocation slash) works end-to-end. See `AGENTS.md` forbidden list.

## Technical Prerequisites (FUTURE)

### VM Engine (already implemented for v1 JKC-only)
- Engine: Wasmtime 18.0.4 (pinned)
- Gas Model: Fuel-based metering
- Memory: Bounded (32 pages max, 2 MB)
- State: Seal-based, not account-based

### Security Primitives
- **Seals**: UTXO-bound, double-spend prevention (L1 consensus)
- **Merkle proofs**: Deterministic state verification
- **Signatures**: Cryptographic (secp256k1 ECDSA)
- **Reorgs**: Child chains wait N confirmations for economic finality
