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
```bash
node test-jkc-contracts.cjs
```

### Verify Deployment
```bash
node replay-verifier.js <txid>
```

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
```javascript
const challengeScript = bitcoin.script.compile([
  Buffer.from(challengerPubkey, 'hex'),
  bitcoin.opcodes.OP_CHECKSIGVERIFY,
  Buffer.from(fraudProofHash, 'hex'),
  bitcoin.opcodes.OP_EQUALVERIFY,
]);

const psbt = new bitcoin.Psbt({ network });
// Add input with challenger funds
// Add OP_RETURN with fraud proof
// Add output to operator vault (to slash bond)
// Add change output
```

### Verify Challenge
```bash
node replay-verifier.js <challenge-txid>
```

## 5. Slash

If the challenge is valid:
1. Operator's bond is slashed
2. Challenger receives portion of slashed bond
3. Remaining funds go to protocol treasury

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

# Deploy
node test-jkc-contracts.cjs

# Verify
node replay-verifier.js 6a697245861d25435ca42bec07ffaac581f6022494907dd86926889c598c7a13
```

### Step 2: Operator Posts Batch
```bash
# Operator creates batch transaction
# Posts state root to JKC testnet
```

### Step 3: Challenger Verifies
```bash
# Independent verifier checks state root
node replay-verifier.js <batch-txid>

# If fraud detected, submit challenge
```

### Step 4: Challenge Fraud
```bash
# Create and broadcast challenge transaction
# Bond is slashed if fraud is proven
```

### Step 5: Claim Rewards
```bash
# Challenger receives portion of slashed bond
# Protocol maintains integrity
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
- Challenger reward = portion of slashed bond
