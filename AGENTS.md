# AI AGENTS MASTER GUIDE & PROTOCOL PLAYBOOK (`AGENTS.md`)

> **Single Source of Truth for Autonomous AI Agents and Core Contributors working on the UTXO-VM Codebase.**

> **Trust Model:** Before writing any code, read `docs/TRUST-MODEL.tex` (or `TRUST-MODEL.pdf`). It defines what L1 guarantees, what indexers guarantee, what a liar can steal, and how a stranger slashes them. If your code conflicts with the trust model, the trust model wins.

---

## 1. System Philosophy & Mental Model

### 1.1 What is UTXO-VM?
**UTXO-VM** is a universal, chain-agnostic, clean-room, patent-free, Turing-complete smart object execution framework engineered for all UTXO Proof-of-Work blockchains.

### 1.2 Core Architectural Invariants:
1. **Chain-Agnostic Single-Use Seals**:
   - There is NO global account balance dictionary stored on L1.
   - Every on-chain smart object instance is bound to a specific UTXO (`location = txid:vout`).
   - Updating an object's state **spends** the current UTXO seal and **creates** a new UTXO seal representing the state $S_{t+1}$.
2. **AssemblyScript -> WASM**:
   - All smart contracts are authored in AssemblyScript (strict TypeScript dialect).
   - Contracts compile to binary **WebAssembly (`.wasm`)**, drastically reducing on-chain byte footprint and transaction fees compared to raw JavaScript text.
3. **Deterministic Execution & Fuel Metering**:
   - The runtime (`core-vm`) executes WASM bytecode via `Wasmtime` with fuel enabled.
   - Execution is purely deterministic: floats without fixed-precision emulation, non-seeded randoms, and OS clock leaks are strictly prohibited.
   - Wasmtime fuel enforces strict gas limits per transaction. Each host call deducts a fixed fuel cost.
4. **Universal Privacy & Stealth Hooks**:
   - Built-in support for stealth addresses and confidential extension layers (e.g. MWEB / ZK-covenants).
   - Smart contracts can receive shielded funds, lock native satoshis (`_satoshis`), and settle payments to stealth addresses.

---

## 2. Inscription & Wire Format Specification

### 2.1 The `utxovm` Envelope (BIP-341 Witness Script / OP_RETURN)
Transactions containing UTXO-VM instructions embed data using standard inscription envelopes:

```text
OP_FALSE OP_IF
  OP_PUSH "utxovm"              // Protocol Identifier (Chain-Agnostic)
  OP_PUSH 0x01                  // Protocol Version (1)
  OP_PUSH "application/wasm"    // Content Type (or application/json for calls)
  OP_PUSH <PAYLOAD_BYTES>       // WASM Bytecode or Method Call Calldata
OP_ENDIF
```

---

## 3. Host Environment & Runtime ABI

When a WASM smart contract executes inside `core-vm`, it communicates with the host via deterministic Host APIs (all registered under `"env"` namespace):

| Host Function | Signature | Fuel Cost | Purpose |
| :--- | :--- | :--- | :--- |
| `host_get_caller` | `(out_ptr: i32) -> i32` | 100 | Returns public key / stealth address of transaction signer |
| `host_get_satoshis` | `() -> u64` | 100 | Returns native chain satoshis/base units locked in this UTXO |
| `host_get_seal` | `(out_ptr: i32) -> i32` | 100 | Returns the input UTXO identifier (`txid:vout`) |
| `host_emit_event` | `(topic_ptr: i32, data_ptr: i32, len: i32)` | 500 | Emits an indexable event log |
| `host_create_object` | `(code_hash_ptr: i32, state_ptr: i32, satoshis: u64) -> i32` | 1000 | Spawns a child smart object (e.g. minted token) |
| `host_stealth_settle` | `(stealth_addr_ptr: i32, satoshis: u64) -> i32` | 500 | Authorizes a privacy extension / stealth settlement |
| `host_mweb_peg_out` | `(stealth_addr_ptr: i32, satoshis: u64) -> i32` | 500 | Initiates MWEB peg-out to a stealth address |
| `host_verify_groth16` | `(vk_ptr, vk_len, proof_ptr, proof_len, inputs_ptr, inputs_len: i32) -> i32` | 10000 | Verifies a Groth16 ZK proof (currently accepts mock proofs) |
| `abort` | `(msg, file, line, col: i32)` | -- | AssemblyScript built-in abort handler (logs warning, does NOT trap) |

---

## 4. Multi-Chain Network Configuration

The canonical chain configuration lives in `chain.json` (source of truth). The SDK reads from it:

```json
{
  "chains": {
    "JKC_TESTNET": {
      "name": "Junkcoin Testnet", "ticker": "tJKC",
      "electrsUrl": "https://jkc-testnet-api.s3na.xyz",
      "network": { "p2pkhPrefix": "0x6f", "p2shPrefix": "0xc4", "wifPrefix": "0xef", "bech32Prefix": "tjkc" },
      "opcodesSupported": true,
      "settlement": { "enabled": true, "role": "settlement_layer", "supportedChains": ["BTC","LTC","DOGE","BEL","DINGO","LKY","SHIC","TRMP","B1T","CRC","PEP"] }
    },
    "JKC": {
      "name": "Junkcoin", "ticker": "JKC",
      "network": { "p2pkhPrefix": "0x10", "p2shPrefix": "0x05", "wifPrefix": "0x90", "bech32Prefix": "jkc" }
    },
    "BTC": { "name": "Bitcoin", "ticker": "BTC", "settlement": { "role": "child_chain", "settlementChain": "JKC" } },
    "LTC": { "name": "Litecoin", "ticker": "LTC", "settlement": { "role": "child_chain", "settlementChain": "JKC" } },
    "DOGE": { "name": "Dogecoin", "ticker": "DOGE", "settlement": { "role": "child_chain", "settlementChain": "JKC" } },
    "BEL": { "name": "Bells", "ticker": "BEL", "settlement": { "role": "child_chain", "settlementChain": "JKC" } },
    "DINGO": { "name": "Dingocoin", "ticker": "DINGO" },
    "LKY": { "name": "Luckycoin", "ticker": "LKY" },
    "SHIC": { "name": "Shiba Inu Coin", "ticker": "SHIC" },
    "TRMP": { "name": "Trumpcoin", "ticker": "TRMP" },
    "B1T": { "name": "Bean Cash", "ticker": "B1T" },
    "CRC": { "name": "Crocodile Cash", "ticker": "CRC" },
    "PEP": { "name": "Pepecoin", "ticker": "PEP" }
  },
  "defaults": { "chain": "JKC_TESTNET" }
}
```

13 chains total. JKC_TESTNET is the active settlement layer. Child chains (BTC, LTC, DOGE, BEL, etc.) anchor to JKC_TESTNET.

---

## 5. Non-Negotiable Rules for AI Agents

1. **Keep Everything Chain-Agnostic**:
   - Never hardcode specific coin names, tickers, or addresses into core contracts or runtime.
   - Use generic standards like `UTX20` (Fungible Token), `UTX721` (NFT), and `NativeVault` (Coin Vault).
2. **Clean-Room Enforcement**:
   - Never import or adapt proprietary code from patented systems.
3. **Pure Determinism**:
   - No floating-point non-determinism, no time leaks.


# UTXO-VM Agent Prompt

You are implementing UTXO-VM on Junkcoin (JKC). You write code and tests. You do not invent a new token, an L2, or marketing.

> Full roadmap and phased plan: see `MISSION.md`.

## Mission (one sentence)

Seal-native WASM objects on JKC UTXOs. L1 is the court. Bonded `utxo-vmd` operators execute. A stranger can punish a lie. No second coin.

## Layer model (do not rename)

- L1 = Junkcoin: blocks, UTXOs, fees, (future) Taproot bonds / challenge / exit.
- UTXO-VM = L1.5 metaprotocol: pinned WASM + seals. Not L2, not L3, not JKC consensus (yet).
- Optional later: Core wallet links `libutxovm`. Consensus embed is OUT OF SCOPE until the court works.

## Stack (do not change without an explicit human OK)

- Contracts: AssemblyScript or Rust → `.wasm`. Artifact of record = SHA-256 of raw WASM.
- Runtime: Rust + Wasmtime, fuel enabled, no WASI, no floats in the ABI, one frozen engine config.
- Node: `packages/node` (`utxo-vmd`) only. Do not grow `packages/indexer` (TS) except to delete or shim to the Rust lib.
- Core: Rust lib from `utxo-core-vm`. C ABI is **not yet implemented** — the binary reads JSON from stdin. Plans for C ABI exist but are not shipped.
- ZK: not required for v1. No Groth16 on the happy path. Mocks only under `#[cfg(test)]`.

Current execution entrypoint (Rust methods, not C ABI):

```text
VmRuntime::deploy(wasm_bytes, caller, seal, satoshis, init_args) -> Result<ExecutionResult, Error>
VmRuntime::execute(wasm_bytes, state, caller, method, args) -> Result<ExecutionResult, Error>
```

A unified `verify()` C ABI entrypoint is **planned but not implemented**. Current callers: node scanner, CLI (JSON stdin/stdout), tests.

## v1 product (only this)

1. Contract types shipped in `packages/contracts`: SOT, UTX20, UTX721, SON, NativeVault, AtomicSwap, Entry. Not vault + NFT + AMM + MWEB as a combined protocol.
2. Deploy: lock seal, pin wasm hash, init state.
3. Call: spend input seal, run `call`, create output seal with new state.
4. `utxo-vmd` re-executes, posts attestation `(chain, height, block_hash, state_root)` signed secp256k1.
5. Two independent runs of the same fixture produce the same root.
6. Equivocation: same operator, same chain+height, two different roots, two valid sigs → proof object. Off-chain verify MUST work. On-chain tapleaf MAY be a stub labeled `NOT CONSENSUS`.
7. Silence: document + test a timeout path (even if script is stub): no attestation for N JKC blocks → user may exit without operators.

Done means tests pass for 2, 3, 5, 6. Not a PDF.

## Court rules

L1 never runs WASM.

Slash v1 = equivocation only (two signed attestations).  
Invalid-transition slash = re-execute + operator vote or later fraud game. Not ZK until a real circuit exists.

Do not claim JKC “has Taproot + OP_CAT activated” unless you cite a JKC Core merge + activation height in-repo. If uncertain, write “planned.” Script builders must use Bitcoin script-num encoding. No 32-byte CHECKSIG keys. Incomplete CAT scripts must compile only behind `feature = "experimental-scripts"`.

Bonds: JKC locked in a documented vault template. No indexer token. Fees: optional extra output in the same user tx; Phase 1 fee may be 0.

## Fuel / determinism

- Use Wasmtime fuel only. Host costs are constants in `runtime::fuel_costs`, applied before the host returns.
- `env.abort` traps the instance via `Err(anyhow::anyhow!(...))`.
- `max_memory_pages` enforced via `Config::static_memory_maximum_size()`.
- Pin Wasmtime version in Cargo.toml. Do not bump it in the same PR as contract fixtures.

**Completed:**
- `GasMeter` in `gas.rs` is dead code — runtime uses Wasmtime fuel directly. Export removed from `lib.rs`.
- `env.abort` handler (`runtime.rs`) returns `Err(...)` — traps the instance.
- `max_memory_pages` enforced via `wasmtime::Config::static_memory_maximum_size()`.
- Host fuel costs centralized in `runtime::fuel_costs` module.
- `ark-*` ZK deps moved to optional `[dependencies]` behind `experimental-zk` feature.
- Code hash validation: `execute()` validates WASM hash against pinned `state.code_hash` before execution.

**Remaining:**
- Formalize WASM ISA spec in docs.

## Forbidden in this phase

- New coin / points / airdrop
- Hub-and-spoke DOGE/LTC/BTC as required architecture
- SPaaS, DHT rent, EIP-1559, “100k JKC APY” as code
- `MOCK_PROOF` / `MOCK_VK` succeeding outside tests
- `host_verify_groth16` on the settlement path
- Embedding VM in JKC consensus
- Rewriting the VM in C++
- Second execution engine
- README claiming L2, “on-chain ZK slashing,” or “OP_CAT live” without code that spends a real script

**Known violations (must be fixed):**
- `runtime.rs` — `host_verify_groth16` is now feature-gated behind `experimental-zk` (fixed).
- `zk.rs` — mock proofs are now behind `#[cfg(test)]` and the module is feature-gated (fixed).
- `covenants.rs` — OP_CAT functions now behind `feature = "experimental-scripts"` (fixed).

## Docs policy

Update README to match code: packages include `utxo-vmd`; model is L1.5; court status honest.
Do not add new ECONOMIC / PATENT / LaTeX files.
If you touch scripts, add `docs/COURT.md` ≤ 40 lines: what L1 checks, what operators check, how a liar loses JKC, what is stub.

**Current status:**
- README lists 5 packages (omits `packages/node` / `utxo-vmd`) — needs update.
- `docs/` contains ECONOMIC-MODEL.md, PATENT-ANALYSIS.md, .tex/.pdf files — legacy, do not add more.
- `docs/COURT.md` does not exist — needs creation.

**Current status:**
- README lists 5 packages (omits `packages/node` / `utxo-vmd`) — needs update.
- `docs/` contains ECONOMIC-MODEL.md, PATENT-ANALYSIS.md, .tex/.pdf files — legacy, do not add more.
- `docs/COURT.md` does not exist — needs creation.

## PR discipline

One concern per PR. Preferred order:

1. `verify()` + C ABI + fixture replay (two processes, same root)
2. Fuel unify + abort trap
3. Remove mock ZK from non-test
4. Equivocation proof tests (known key, two messages)
5. Seal deploy/call e2e on JKC regtest/testnet
6. Stub tapleaves + COURT.md
7. Deprecate TS indexer

Every PR: what can still lie, and whether that lie is slashable.

## Tests that must exist

- Same WASM + witness → identical root twice
- Fuel exhaustion reverts, no state write
- Abort traps
- Bad wasm hash rejected
- Equivocation true/false cases
- Mock proof rejected when `cfg(test)` is off (or feature `allow-mock-zk` off)
- Reorg: undo last object transition when JKC reorgs (if scanner exists)

## Voice

Be a protocol engineer. Prefer deleting scope over adding subsystems. If a request conflicts with this file, follow this file and say so.
```
