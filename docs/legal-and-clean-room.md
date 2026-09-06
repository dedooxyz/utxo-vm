# Clean-Room Engineering & Patent Freedom Analysis

## 1. Purpose of this Document

This document records the design decisions ensuring that **JKC-VM is a 100% original, clean-room, Free and Open Source (FOSS) project**, completely free from the patent claims of US Patent Nos. 11,694,197 and 11,188,911 (associated with Bitcoin Computer).

## 2. Clean-Room Architectural Distinctions

| Aspect | Patented Implementation (Bitcoin Computer) | JKC-VM Clean-Room Implementation |
| :--- | :--- | :--- |
| **Language & Execution** | Inscribes raw JS text; relies on proprietary off-chain JS evaluation with license payment hooks. | Inscribes **WASM bytecode** compiled from AssemblyScript; executes in standard public-domain `Wasmtime` VM. |
| **State Encoding** | Proprietary property graph (`_rev`, `_root`, `_owners` metadata format). | Standard **Single-Use Seals** (Peter Todd 2014) and public domain State Transition Functions $S' = f(S, tx)$. |
| **Monetization & Fees** | Hardcoded commercial payment requirements on mainnet with legal threats in `LEGAL.md`. | **Zero proprietary fees**; 100% MIT licensed. Miners receive standard L1 network fees. |
| **Privacy Integration** | Not compatible with Mimblewimble. | **Native MWEB Extension Block & Stealth integration**. |
