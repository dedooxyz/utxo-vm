# AI AGENTS MASTER GUIDE & PROTOCOL PLAYBOOK (`AGENTS.md`)

> **Single Source of Truth for Autonomous AI Agents and Core Contributors working on the UTXO-VM Codebase.**

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
3. **Deterministic Execution & Gas Metering**:
   - The runtime (`core-vm`) executes WASM bytecode via `Wasmtime`.
   - Execution is purely deterministic: floats without fixed-precision emulation, non-seeded randoms, and OS clock leaks are strictly prohibited.
   - Instruction counting enforces strict gas limits per transaction.
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

When a WASM smart contract executes inside `core-vm`, it communicates with the host via deterministic Host APIs:

| Host Function | Signature | Purpose |
| :--- | :--- | :--- |
| `host_get_caller` | `(out_ptr: i32) -> i32` | Returns public key / stealth address of transaction signer |
| `host_get_satoshis` | `() -> u64` | Returns native chain satoshis/base units locked in this UTXO |
| `host_get_seal` | `(out_ptr: i32) -> i32` | Returns the input UTXO identifier (`txid:vout`) |
| `host_emit_event` | `(topic_ptr: i32, data_ptr: i32, len: i32) -> void` | Emits an indexable event log |
| `host_create_object` | `(code_hash_ptr: i32, state_ptr: i32, satoshis: u64) -> i32` | Spawns a child smart object (e.g. minted token) |
| `host_stealth_settle` | `(stealth_addr_ptr: i32, satoshis: u64) -> i32` | Authorizes a privacy extension / stealth settlement |

---

## 4. Multi-Chain Network Configuration in SDK

The SDK supports dynamic network configurations for any UTXO network:

```typescript
export interface ChainConfig {
  name: string;
  ticker: string;
  p2pkhPrefix: number;
  p2shPrefix: number;
  wifPrefix: number;
  bech32Prefix?: string;
  rpcDefaultPort: number;
}

export const SUPPORTED_CHAINS: Record<string, ChainConfig> = {
  BTC: { name: "Bitcoin", ticker: "BTC", p2pkhPrefix: 0x00, p2shPrefix: 0x05, wifPrefix: 0x80, rpcDefaultPort: 8332 },
  LTC: { name: "Litecoin", ticker: "LTC", p2pkhPrefix: 0x30, p2shPrefix: 0x32, wifPrefix: 0xb0, rpcDefaultPort: 9332 },
  DOGE: { name: "Dogecoin", ticker: "DOGE", p2pkhPrefix: 0x1e, p2shPrefix: 0x16, wifPrefix: 0x9e, rpcDefaultPort: 22555 },
  JKC: { name: "Junkcoin", ticker: "JKC", p2pkhPrefix: 0x10, p2shPrefix: 0x05, wifPrefix: 0x90, rpcDefaultPort: 9772 },
  BEL: { name: "Bells", ticker: "BEL", p2pkhPrefix: 0x19, p2shPrefix: 0x1e, wifPrefix: 0x99, rpcDefaultPort: 19918 },
};
```

---

## 5. Non-Negotiable Rules for AI Agents

1. **Keep Everything Chain-Agnostic**:
   - Never hardcode specific coin names, tickers, or addresses into core contracts or runtime.
   - Use generic standards like `UTX20` (Fungible Token), `UTX721` (NFT), and `NativeVault` (Coin Vault).
2. **Clean-Room Enforcement**:
   - Never import or adapt proprietary code from patented systems.
3. **Pure Determinism**:
   - No floating-point non-determinism, no time leaks.
