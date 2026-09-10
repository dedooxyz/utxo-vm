# UTXO-VM ARCHITECTURAL SPECIFICATION

---

## 1. System Overview

UTXO-VM provides a universal, chain-agnostic smart contract execution layer for UTXO blockchains.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    LAYER 1.5: STATE GRAPH (UTXO-VM METAPROTOCOL)             │
│                                                                             │
│   [ Smart Object #1: UTX20 Token ] ──────(Transfer Call)──────► [ New Seal ]│
│         UTXO: 3a7f...01:0                                  UTXO: 9c2b...02:0│
│                                                                             │
│   [ Smart Object #2: Vault ] ────────────(Withdraw)──────────► [ New Seal ] │
│         UTXO: 5e11...04:1                                  UTXO: 8f44...03:0│
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ Anchored via Single-Use Seals
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    LAYER 1: ANY UTXO BLOCKCHAIN (JKC v1; BTC/LTC/DOGE FUTURE)│
│                                                                             │
│   [ Block N ] ──────────────────────► [ Block N+1 ]                         │
│   - Canonical UTXO Transactions       - Optional Privacy / Extension Block  │
│   - Taproot / Witness Inscriptions    - Single-Use Seal Spending & Creation │
└─────────────────────────────────────────────────────────────────────────────┘
```

> **Note:** UTXO-VM is an L1.5 metaprotocol, NOT an L2. v1 is JKC-only.
> Cross-chain (BTC/LTC/DOGE) is future/optional — see `AGENTS.md` and `MISSION.md`.

---

## 2. State Model: Single-Use Seals

A Single-Use Seal is a cryptographic primitive that closes exactly once. In UTXO-VM:
* A Seal is defined by an Unspent Transaction Output:
  $$\text{Seal} = (\text{TxID}, \text{Vout})$$
* A State Transition consumes $n$ input seals and creates $m$ output seals:
  $$\Delta(S) : \{\text{Seal}_{in}^1, \dots, \text{Seal}_{in}^n\} \xrightarrow{\text{Method}(\text{Args})} \{\text{Seal}_{out}^1, \dots, \text{Seal}_{out}^m\}$$

---

## 3. Universal Token Standards

1. **UTX20 (Fungible Token Standard)**:
   - Universal replacement for ERC-20 / BRC-20 / TBC-20 across all UTXO networks.
   - Deterministic transfer, split, and mint mechanisms.
2. **UTX721 (Non-Fungible Inscription Standard)**:
   - Universal NFT and Inscription standard with arbitrary metadata URI and provenance tracking.
3. **NativeVault (Wrapped Native Asset Standard)**:
   - Locks native chain coins (`_satoshis`) and mints wrapped tokens or executes conditional payouts.

---

## 4. Clean-Room Design

UTXO-VM is a 100% original, clean-room, FOSS (MIT) implementation. Key distinctions from patented systems (e.g. Bitcoin Computer, US Patents 11,694,197 and 11,188,911):

| Aspect | Patented Implementation | UTXO-VM |
| :--- | :--- | :--- |
| **Execution** | Proprietary off-chain JS evaluation with license hooks | WASM bytecode in public-domain Wasmtime |
| **State** | Proprietary property graph (`_rev`, `_root`, `_owners`) | Standard Single-Use Seals (Peter Todd 2014) |
| **Fees** | Hardcoded commercial payment requirements | Zero proprietary fees; MIT licensed |
| **Privacy** | Not compatible with Mimblewimble | MWEB Extension Block & Stealth (planned) |

---

## 5. Privacy: MWEB & Stealth (FUTURE)

> **Status:** MWEB is NOT yet active on JKC (planned at height 180,000; empirically verified not active at block 177,269). Host functions (`host_stealth_settle`, `host_mweb_peg_out`) exist in the runtime but are NOT on the v1 settlement path.

Planned privacy patterns once MWEB activates:

- **Shielded Liquidity Vault**: Peg-In to MWEB → transfer to fresh stealth address → Peg-Out to UTXO-VM contract UTXO (clean funds, no on-chain ancestry).
- **Stealth Smart Object Ownership**: Set `_owner` to an MWEB stealth key instead of a static public key.
- **Dark Pool Orderbook Settlement (PSOB + MWEB)**: PSOB orders matched peer-to-peer; settlement deposits proceeds directly into MWEB stealth output.
