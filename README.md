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

**What does NOT work yet:**
- On-chain challenge spending a liar's bond (requires BIP-341 script path spending)
- Operator slashing on-chain (only in-memory struct)
- C ABI for Core wallet integration
- Second independent implementation

**Do not use in production.** This is a research prototype.

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
# Core VM tests (9 tests)
cargo test -p utxo-core-vm

# Node consensus tests (24 tests)
cargo test -p utxo-vmd --lib consensus

# Full testnet deployment
node test-jkc-minimal-product.cjs
```

---

## 📄 Documentation

- [Trust Model](docs/TRUST-MODEL.tex) - Adversary model, JKC parameters, slashing
- [WASM ISA](docs/WASM-ISA.md) - Supported WASM features and execution model
- [Lifcycle](docs/LIFECYCLE.md) - Compile → Deploy → Call → Challenge
- [JKC Chain Work](docs/JKC-CHAIN-WORK.md) - Taproot, opcodes, addresses
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
