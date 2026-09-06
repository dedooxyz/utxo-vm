# UTXO-VM: Universal Smart Object Engine for UTXO Blockchains

> **Clean-Room, Turing-Complete, Patent-Free Smart Object Protocol for Any UTXO Blockchain (Bitcoin, Litecoin, Dogecoin, Junkcoin, Bells, etc.) with Native Privacy Extension Support.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Language: Rust / AssemblyScript](https://img.shields.io/badge/Language-Rust%20%7C%20AssemblyScript-orange.svg)](https://www.rust-lang.org/)
[![Multi-Chain](https://img.shields.io/badge/Chains-BTC%20%7C%20LTC%20%7C%20DOGE%20%7C%20JKC%20%7C%20BEL-blue.svg)](https://github.com/DedooProjects/utxo-vm)

---

## 📖 Overview

**UTXO-VM** is a **chain-agnostic, stateful smart object execution framework** engineered for any UTXO-based Proof-of-Work blockchain.

Instead of treating transactions as passive text inscriptions (like BRC-20) or using patented proprietary JS runtimes, UTXO-VM executes **gas-metered WebAssembly (WASM)** compiled from **AssemblyScript (TypeScript)**, bound directly to UTXO **Single-Use Seals**.

---

## ⚡ Key Highlights

1. **Chain-Agnostic by Design**: Runs seamlessly across Bitcoin, Litecoin, Dogecoin, Junkcoin, Bells, Bitcoin Cash, and custom UTXO chains.
2. **100% Clean-Room FOSS**: Completely free of proprietary patent claims and third-party commercial royalties.
3. **AssemblyScript & WASM Native**: High-level TypeScript syntax compiled to minimal, high-speed WASM binary payloads.
4. **UTXO Single-Use Seals**: Each smart object instance is mapped to a UTXO (`txid:vout`). State transitions spend the input seal and create a new output seal.
5. **Universal Privacy & Stealth Hooks**: Compatible with confidential extension blocks (MWEB, Pedersen commitments, and Stealth Addresses).
6. **Deterministic Gas Metering**: Strict instruction counting preventing DoS and infinite loops.

---

## 📦 Monorepo Packages

| Package | Path | Description |
| :--- | :--- | :--- |
| **`@utxo-vm/contracts`** | `packages/contracts` | Standard smart contract library (UTX20, UTX721, NativeVault, Swap) |
| **`@utxo-vm/core-vm`** | `packages/core-vm` | High-performance Rust & Wasmtime deterministic execution engine |
| **`@utxo-vm/sdk`** | `packages/sdk` | Multi-chain TypeScript SDK for web wallets, dApps, and transaction serialization |
| **`@utxo-vm/indexer`** | `packages/indexer` | Multi-chain block scanner & UTXO state database |
| **`@utxo-vm/cli`** | `packages/cli` | Developer CLI tool (`utxo-vm compile`, `utxo-vm deploy`, `utxo-vm call`) |

---

## 🚀 Quick Start

```bash
# Clone & install dependencies
git clone https://github.com/DedooProjects/utxo-vm.git
cd utxo-vm
pnpm install

# Build all packages & compile contracts to WASM
pnpm build
```
