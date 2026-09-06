# Cross-Chain Settlement Architecture

## Problem
AuxPow chains (Junkcoin, Dogecoin, Litecoin forks) merged mine with Bitcoin/Litecoin for security, but have NO settlement layer for:
- State transitions between chains
- Cross-chain asset transfers
- Contract verification across chains

## Solution: Junkcoin as Settlement Layer

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

### Implementation Roadmap

1. **Phase 1: State Anchoring** ✅
   - Child chain broadcasts merkle root to Junkcoin
   - Junkcoin stores and seals

2. **Phase 2: Seal Verification** 🔄
   - Child chain queries Junkcoin for seal
   - Merkle proof verification

3. **Phase 3: Cross-Chain Transfer** ⏳
   - Atomic swaps via seals
   - Bridge contracts

4. **Phase 4: Full Settlement** ⏳
   - Multi-chain state sync
   - Governance and upgradeability
