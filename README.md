# UTXO-VM: Universal Smart Object Engine for UTXO Blockchains

> **Chain-Agnostic, Turing-Complete, Patent-Free Smart Object Protocol for UTXO Blockchains.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Language: Rust / AssemblyScript](https://img.shields.io/badge/Language-Rust%20%7C%20AssemblyScript-orange.svg)](https://www.rust-lang.org/)

---

## ⚠️ Current Status: L1.5 Protocol Kit (NOT v1)

**UTXO-VM is a metaprotocol that executes WASM on UTXO chains.** It is NOT L2, NOT L3, and does NOT modify L1 consensus.

**What works today:**
- Deterministic WASM execution via Wasmtime with fuel metering
- Single-use seal model (state bound to UTXOs)
- Contract compilation (AssemblyScript → WASM)
- State replay verification (two replays → same root)
- Script templates for vault, challenge, silence escape (testnet only)
- **On-chain challenge slash (BIP-341 P2TR script-path spend)** — validated on JKC testnet (2026-09-10)
- **Operator slashing on-chain** — bonded vault slashed via watcher committee challenge leaf
- Equivocation detection + cryptographic proof verification
- 3-of-3 validator quorum attestations posted on-chain
- Scanner daemon (`utxo-vmd`) syncing live JKC testnet blocks
- Live testnet operations: SOT mint, NFT mint, NativeVault, AtomicSwap, attestation posting

**What does NOT work yet:**
- C ABI for Core wallet integration
- Second independent implementation
- On-chain divergence verification (Bitcoin Script cannot verify WASM execution — off-chain only)

**Do not use in production.** This is a research prototype.

> **Test results**: 84 tests pass (60 offline + 24 live testnet). See `docs/TESTING.md` for full test documentation, live tx IDs, and BIP-342 compliance fixes.

---

## 📦 Monorepo Packages

| Package | Path | Description |
| :--- | :--- | :--- |
| **`utxo-core-vm`** | `packages/core-vm` | Rust + Wasmtime deterministic execution engine |
| **`utxo-vmd`** | `packages/node` | Node daemon with scanner, consensus, L1 scripts |
| **`@utxo-vm/contracts`** | `packages/contracts` | AssemblyScript contracts (UTX20, UTX721, NativeVault, Swap) |
| **`@utxo-vm/sdk`** | `packages/sdk` | TypeScript SDK for transaction building |
| **`@utxo-vm/cli`** | `packages/cli` | Developer CLI tool |

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────┐
│                  User / Wallet                   │
└─────────────────┬───────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────┐
│              UTXO-VM Runtime (L1.5)              │
│  ┌─────────────────────────────────────────┐    │
│  │  WASM Execution (Wasmtime + Fuel)       │    │
│  └─────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────┐    │
│  │  Single-Use Seals (UTXO Binding)        │    │
│  └─────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────┐    │
│  │  State Management (SMT)                 │    │
│  └─────────────────────────────────────────┘    │
└─────────────────┬───────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────┐
│           L1 Blockchain (JKC, BTC, LTC)         │
│  • Blocks, UTXOs, fees                          │
│  • Taproot bonds (planned)                      │
│  • Challenge / exit (planned)                   │
└─────────────────────────────────────────────────┘
```

---

## 🔧 Quick Start

```bash
# Clone & install
git clone https://github.com/dedooxyz/utxo-vm.git
cd utxo-vm
npm install

# Build WASM contracts
cd packages/contracts
npm run asbuild:release

# Run core-vm tests
cd ../core-vm
cargo test

# Run node tests
cd ../node
cargo test
```

---

## 📋 Contract Types

| Contract | Description | Status |
| :--- | :--- | :--- |
| **SOT** | Smart Object Token (base) | ✅ Implemented |
| **UTX20** | Fungible token (ERC-20 like) | ✅ Implemented |
| **UTX721/SON** | Non-fungible token | ✅ Implemented |
| **NativeVault** | Native coin vault | ✅ Implemented |
| **AtomicSwap** | Hash time-locked contract | ✅ Implemented |
| **Entry** | Contract dispatcher | ✅ Implemented |

---

## 🧪 Testing

```bash
# Core VM tests (12 tests)
cargo test -p utxo-core-vm

# Node library tests (51 tests: consensus, scanner, storage, p2p, rpc)
cargo test -p utxo-vmd --lib

# Node integration tests (12 tests)
cargo test -p utxo-vmd --test consensus_tests --test processor_tests --test rpc_tests --test smt_tests --test storage_tests --test p2p_tests --test cross_chain_tests

# Full workspace (all offline tests, 60 total)
cargo test --workspace

# Live testnet tests (24 tests, spend real tJKC — must pass --ignored)
cargo test -p utxo-vmd --test live_testnet -- --nocapture --ignored
cargo test -p utxo-vmd --test live_write_testnet -- --nocapture --ignored
cargo test -p utxo-vmd --test live_broadcast -- --nocapture --ignored
cargo test -p utxo-vmd --test live_mint_call -- --nocapture --ignored
cargo test -p utxo-vmd --test live_contracts -- --nocapture --ignored
cargo test -p utxo-vmd --test live_quorum_equivocation -- --nocapture --ignored
cargo test -p utxo-vmd --test live_vault_challenge -- --nocapture --ignored

# Scanner daemon against live testnet
cargo run -p utxo-vmd -- --chain JKC_TESTNET --electrs-url https://jkc-testnet-api.s3na.xyz
```

> **Full test documentation**: see [`docs/TESTING.md`](docs/TESTING.md) for every test file, what it covers, live testnet results (10 confirmed txs), and BIP-342 compliance fixes discovered through live testing.

---

## 📄 Documentation

- [Testing Guide](docs/TESTING.md) - Full test suite, live testnet results, BIP-342 fixes
- [Trust Model](docs/TRUST-MODEL.tex) - Adversary model, JKC parameters, slashing
- [Court Model](docs/COURT.md) - L1 checks, operator checks, slashing mechanics, stubs
- [WASM ISA](docs/WASM-ISA.md) - Supported WASM features and execution model
- [Lifecycle](docs/LIFECYCLE.md) - Compile → Deploy → Call → Challenge
- [JKC Chain Work](docs/JKC-CHAIN-WORK.md) - Taproot, opcodes, addresses
- [Cross-Chain Architecture](docs/CROSS-CHAIN-ARCHITECTURE.md) - Multi-chain settlement
- [Mission](MISSION.md) - 9-step roadmap with progress

---

## ⚖️ Trust Model (Summary)

- **L1 guarantees:** UTXO ownership, transaction ordering, block rewards
- **Indexer guarantees:** Deterministic WASM execution, signed attestation
- **What a liar can steal:** Nothing if bond > TVL and challenge works
- **What a stranger can slash:** Equivocation (two signed conflicting roots)

See [Trust Model](docs/TRUST-MODEL.tex) for full specification.

---

## 🚫 What This Is NOT

- **NOT L2** - No separate consensus, no sequencer, no bridge
- **NOT a new coin** - Uses existing chain native currency
- **NOT production ready** - Research prototype, testnet only
- **NOT patent-free yet** - Needs legal review before claims

---

## 📄 License

MIT
