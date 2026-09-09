# UTXO-VM Fraud Proof: Comparative Analysis

## Problem Statement

How to prove operator lied about state root on JKC L1 without running full WASM?

## Solution Options

### Option 1: Interactive Fraud Proof (Current v0.3)

**How it works:**
1. Operator posts root $R_0$
2. Challenger disagrees
3. Binary search: split execution trace in half
4. Operator posts midpoint root
5. Repeat until single instruction disputed
6. L1 re-execute ONE instruction

**Complexity:** $\log_2(N)$ rounds. For 1000 objects: ~10 rounds.

**L1 Cost:** 10 transactions × ~500 bytes = ~5000 bytes on-chain.

**Pros:**
- No ZK needed
- Works with current JKC opcodes
- Well-understood (used by Optimism, Arbitrum)
- Only re-executes ONE instruction at the end

**Cons:**
- Multiple L1 transactions (slow, expensive)
- Complex UX (10 rounds of interaction)
- Requires operator to respond (liveness assumption)
- Challenger must be online during dispute

---

### Option 2: Optimistic + Social Consensus

**How it works:**
1. Operator posts root $R_0$
2. Challenge window: $\delta$ blocks
3. If unchallenged: root is FINAL (economic finality)
4. If challenged: social layer decides (off-chain governance)

**Complexity:** O(1) on-chain.

**L1 Cost:** 1 transaction per batch.

**Pros:**
- Simplest possible design
- Lowest on-chain cost
- Fast finality

**Cons:**
- NOT trustless (relies on social consensus)
- Governance attacks possible
- Not suitable for high-value TVL

---

### Option 3: ZK-SNARK (Future)

**How it works:**
1. Operator executes WASM off-chain
2. Operator generates ZK proof of correct execution
3. Operator posts proof on-chain
4. L1 verifies proof (O(1) cost)

**Complexity:** O(1) on-chain.

**L1 Cost:** 1 transaction with ~288 bytes proof (Groth16).

**Pros:**
- Single proof, instant finality
- No interaction needed
- No liveness assumption
- Strongest security model

**Cons:**
- Circuit doesn't exist yet (must be built)
- Proving time is expensive (~minutes for complex WASM)
- Needs trusted setup or PLONK (more complex)
- WASM → Circuit compiler needed

---

### Option 4: Multi-Operator Consensus

**How it works:**
1. Multiple operators (3+) execute same WASM independently
2. Each posts their root
3. If 2/3 agree: root is correct
4. If disagreement: equivocation detected

**Complexity:** O(n) on-chain (n = number of operators).

**L1 Cost:** n transactions per batch.

**Pros:**
- Simple to implement
- No new cryptography
- Works with current opcodes

**Cons:**
- Assumes honest majority (already the assumption)
- Doesn't detect invalid root (only detects equivocation)
- Higher on-chain cost

---

### Option 5: Merkle Proof Verification

**How it works:**
1. Operator posts root $R_0$ + Merkle proof of state transitions
2. Challenger provides counter-proof
3. L1 verifies Merkle proofs (OP_SHA256)
4. If proof is invalid: slash bond

**Complexity:** O(log N) on-chain.

**L1 Cost:** Merkle proof (~320 bytes per level × depth).

**Pros:**
- Works with current opcodes (OP_SHA256)
- No ZK needed
- Deterministic verification

**Cons:**
- Requires operator to commit full state tree
- Merkle proof size grows with state size
- Still needs off-chain re-execution to generate proofs

---

### Option 6: Replay-based Verification

**How it works:**
1. Operator posts root $R_0$
2. Challenger claims $R_0$ is wrong, provides $R_1$
3. Both post security deposits
4. L1 picks RANDOM block in execution trace
5. Both must provide state at that block
6. L1 re-executes from start to that block
7. If one party is wrong: slash deposit

**Complexity:** O(1) on-chain (single re-execution).

**L1 Cost:** 2 transactions (challenge + response).

**Pros:**
- Simple (no binary search)
- Random selection prevents gaming
- Single re-execution on-chain

**Cons:**
- Requires full re-execution from start (expensive)
- Not practical for long traces
- Random selection might pick easy instruction

---

## Comparison Table

| Option | Rounds | L1 Cost | Security | Complexity | JKC Compatible |
|--------|--------|---------|----------|------------|----------------|
| 1. Interactive | log₂(N) | High | Strong | Medium | ✅ |
| 2. Optimistic | 1 | Low | Weak | Low | ✅ |
| 3. ZK-SNARK | 1 | Low | Strongest | High | ❌ (no circuit) |
| 4. Multi-Op | n | Medium | Medium | Low | ✅ |
| 5. Merkle | 1 | Medium | Strong | Medium | ✅ |
| 6. Replay | 1 | Medium | Strong | Low | ✅ |

## Recommendation

**For UTXO-VM on JKC:**

### Phase 1 (v1): Equivocation Only
- Use Option 4 (Multi-Operator Consensus) for equivocation detection
- Simple, works now, no new cryptography

### Phase 2 (v2): Invalid Root
- Use Option 1 (Interactive Fraud Proof) for invalid root
- Works with current JKC opcodes
- Well-understood security model

### Phase 3 (v3): ZK When Ready
- Transition to Option 3 (ZK-SNARK) when circuit exists
- Best UX (no interaction)
- Strongest security

## Why Not Just Use ZK?

1. **Circuit doesn't exist**: Building WASM → Circuit compiler is a research project
2. **Proving time**: Even with circuit, proving takes minutes
3. **Trusted setup**: Groth16 needs trusted setup, PLONK is more complex
4. **JKC compatibility**: Need to verify on JKC L1, not Ethereum

## Why Interactive is Better Than Optimistic?

1. **Trustless**: No social layer needed
2. **Deterministic**: Math doesn't lie
3. **Economic incentive**: Challenger gets bond if right
4. **Works on JKC**: No new opcodes needed

## Final Recommendation

**Stick with Interactive Fraud Proof (v1) for now.** It's the best balance of:
- Security (trustless)
- Compatibility (works on JKC)
- Complexity (well-understood)
- Cost (acceptable)

**When ZK circuit exists:** Migrate to ZK-SNARK for better UX.
