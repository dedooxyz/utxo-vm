# UTXO-VM ARCHITECTURAL SPECIFICATION

---

## 1. System Overview

UTXO-VM provides a universal, chain-agnostic smart contract execution layer for UTXO blockchains.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            LAYER 2: STATE GRAPH                             │
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
│                    LAYER 1: ANY UTXO BLOCKCHAIN (BTC/LTC/DOGE/JKC/BEL)      │
│                                                                             │
│   [ Block N ] ──────────────────────► [ Block N+1 ]                         │
│   - Canonical UTXO Transactions       - Optional Privacy / Extension Block  │
│   - Taproot / Witness Inscriptions    - Single-Use Seal Spending & Creation │
└─────────────────────────────────────────────────────────────────────────────┘
```

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
