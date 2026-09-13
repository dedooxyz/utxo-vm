# UTXO-VM Complete Lifecycle Guide

This document covers the complete lifecycle of UTXO-VM: from compiling contracts to challenging fraudulent state transitions.

## 1. Compile

### Compile AssemblyScript to WASM
```bash
cd packages/contracts
npm run asbuild:release
```

This produces:
- `build/release.wasm` - Optimized WASM bytecode
- `build/release.wat` - WebAssembly Text format (for inspection)
- `build/release.wasm.map` - Source map for debugging

### Calculate WASM Hash
```bash
sha256sum build/release.wasm
```

The WASM hash is the artifact of record. All state transitions must reference this hash.

## 2. Deploy

### Create Deployment Transaction
```javascript
const bitcoin = require('bitcoinjs-lib');
const crypto = require('crypto');
const fs = require('fs');

// Load WASM
const wasmBytes = fs.readFileSync('packages/contracts/build/release.wasm');
const wasmHash = crypto.createHash('sha256').update(wasmBytes).digest('hex');

// Create OP_RETURN envelope
const envelope = {
  protocol: 'utxovm',
  version: 1,
  contentType: 'application/wasm',
  codeHash: wasmHash,
  initState: JSON.stringify({
    type: 'NativeVault',
    owner: 'your-address',
    totalLockedSatoshis: 0,
  }),
};

const psbt = new bitcoin.Psbt({ network });
// Add input and outputs...
```

### Broadcast Transaction
Use the SDK (`@utxo-vm/sdk`) or CLI (`@utxo-vm/cli`) to build and broadcast the
deployment transaction. See `packages/sdk/src/contracts/` for per-contract
client helpers (e.g. `sot_client.ts`, `native_vault_client.ts`).

### Verify Deployment
Replay-verify the deployment by re-executing the WASM against the on-chain
state with `utxo-core-vm` (Rust) or the `utxo-vmd` scanner, and comparing the
computed state root. Two independent replays of the same fixture must produce
the same root (see `test_deterministic_replay_identical_root`).

## 3. Call

### Create Call Transaction
```javascript
const callData = {
  protocol: 'utxovm',
  version: 1,
  contentType: 'application/json',
  method: 'deposit',
  args: {},
  vaultRef: 'vault-txid',
};

const psbt = new bitcoin.Psbt({ network });
// Add input with funds to deposit
// Add OP_RETURN with callData
// Add output to vault
// Add change output
```

### Execute Call
The WASM contract executes in the UTXO-VM runtime:
1. Load contract state from UTXO
2. Execute WASM function
3. Update state
4. Create new UTXO with updated state

## 4. Challenge

### Identify Fraud
A challenger identifies fraudulent behavior:
- **Invalid state root:** Operator posts incorrect state
- **Censorship:** Operator refuses to include valid transactions
- **Equivocation:** Two operators post different roots for same batch

### Create Fraud Proof
```javascript
const fraudProof = {
  type: 'equivocation',
  operator1: {
    pubkey: 'operator1-pubkey',
    signature1: 'signature-on-root-1',
    root1: 'state-root-1',
  },
  operator2: {
    pubkey: 'operator2-pubkey',
    signature2: 'signature-on-root-2',
    root2: 'state-root-2',
  },
  batchHeight: 12345,
  chain: 'JKC_TESTNET',
};
```

### Submit Challenge Transaction
The challenge spends the operator's vault UTXO via the Taproot script path
(Model B, committee-gated). The watcher committee co-signs with individual
`OP_CHECKSIG` + `OP_CHECKSIGADD` (BIP-342; `OP_CHECKMULTISIG` is disabled in
tapscript). Equivocation evidence is posted as an OP_RETURN output.

See `packages/node/src/consensus/challenge.rs` (`build_challenge_transaction`)
and `packages/node/src/consensus/l1_scripts.rs` (`build_challenge_leaf`) for
the canonical builders. Live testnet validation: tx `76ee0e2f...` (2026-09-10)
slashed a 100k-sat bond via this path.

### Verify Challenge
Re-execute the WASM against the on-chain state and confirm the two
attestations in the equivocation proof are valid signatures by the same
operator on divergent roots. See `packages/node/src/consensus/attestation.rs`
(`verify_equivocation_proof`) and `packages/core-vm/tests/runtime_tests.rs`
(`test_deterministic_replay_identical_root`).

## 5. Slash

If the challenge is valid:
1. Operator's bond is slashed
2. Challenger claims the bond after the claim CSV delay
3. Unclaimed remainder stays in the challenge UTXO until rebut or claim timeout

There is no protocol treasury and no second coin — the bond is JKC, claimed by
the challenger per the Model B challenge UTXO leaves
(`build_challenge_claim_script_tree` in `l1_scripts.rs`).

## 6. Exit

If operator is unresponsive (silence):
1. Wait for unbond delay (60 blocks on testnet)
2. Create exit transaction
3. Withdraw funds without operator approval

## Complete Flow Example

### Step 1: Deploy Contract
```bash
# Compile
cd packages/contracts && npm run asbuild:release

# Deploy (via @utxo-vm/sdk or @utxo-vm/cli — see packages/sdk/src/contracts/)
# The SDK builds the OP_RETURN envelope + PSBT and broadcasts to JKC testnet.
```

### Step 2: Operator Posts Batch
```bash
# Operator creates batch transaction
# Posts state root to JKC testnet (utxovm:batch envelope)
```

### Step 3: Challenger Verifies
```bash
# Independent verifier re-executes the WASM and compares the state root
# (two independent replays must produce the same root — see runtime_tests.rs)
# If fraud detected, submit challenge
```

### Step 4: Challenge Fraud
```bash
# Build + broadcast challenge transaction via build_challenge_transaction()
# Watcher committee co-signs (BIP-342 CHECKSIGADD, M-of-N)
# Bond is slashed if fraud is proven
```

### Step 5: Claim Rewards
```bash
# Challenger claims the slashed bond after the claim CSV delay
# (see build_challenge_claim_script_tree in l1_scripts.rs)
```

## Key Concepts

### Single-Use Seals
Each state transition spends the current UTXO and creates a new one. This ensures:
- No double-spending
- Clear ownership chain
- Verifiable state history

### Deterministic Execution
All WASM execution is deterministic:
- No floating-point operations
- No random number generators
- No system time access
- Same input always produces same output

### Fraud Proofs
Anyone can prove fraud by showing:
1. Two conflicting state roots from same operator
2. Invalid state transition
3. Censorship of valid transactions

### Bond System
Operators must bond JKC to participate:
- Bond = economic security guarantee
- Slash = punishment for fraud
- Exit = withdrawal after unbond delay

## Block Explorer Integration

### API Endpoints
- `GET /tx/{txid}` - Get transaction details
- `GET /address/{address}/utxo` - Get UTXOs for address
- `GET /block/{hash}` - Get block details

### Display Elements
- OP_RETURN data parsed and displayed
- State roots shown in transaction details
- Challenge status visible
- Bond status tracked

## Security Model

### Trust Assumptions
- L1 (JKC) is secure and decentralized
- At least one honest operator exists
- Challengers are monitoring the system

### Threat Model
- **Operator fraud:** Slashed via fraud proofs
- **Censorship:** Users can exit after timeout
- **Equivocation:** Two conflicting roots = provable fraud
- **Vanishing:** Bond remains locked until unbond delay

### Economic Security
- Bond amount must be > 10x TVL
- Slash amount = 100% of bond for equivocation
- Challenger claims the slashed bond after the claim CSV delay (Model B challenge UTXO)
