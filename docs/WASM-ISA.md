# UTXO-VM WASM ISA & Protocol Specification

This document formalizes the WebAssembly Instruction Set Architecture (ISA) and wire protocol supported by UTXO-VM. It consolidates the WASM execution model, inscription envelope format, and host ABI call sequence.

## 1. Overview

UTXO-VM executes WebAssembly (WASM) bytecode via the Wasmtime runtime. All contracts are authored in AssemblyScript (strict TypeScript dialect) and compile to `.wasm` binaries.

## 2. Wire Protocol: Inscription Envelope

### 2.1 Taproot / Witness Script Envelope (Primary)
```text
OP_FALSE
OP_IF
  OP_PUSH "utxovm"               // 6 bytes tag (chain-agnostic)
  OP_PUSH 0x01                  // 1 byte protocol version
  OP_PUSH "application/wasm"    // MIME content type (or application/json)
  OP_PUSH <CONTENT_BYTES>       // Raw WASM bytecode or calldata payload
OP_ENDIF
```

### 2.2 OP_RETURN Payload (Compact Method Calls)
For small function invocations (<= 80 bytes), calldata can be encoded in standard `OP_RETURN`:
```text
OP_RETURN <0x6a> <"UTXOVM"> <method_id: 4 bytes> <calldata_bytes>
```

### 2.3 Content Types
| Content Type | Purpose |
|:---|:---|
| `application/wasm` | Deploy WASM smart contract |
| `application/json` | Execute method call |
| `application/octet-stream` | Raw data inscription |

## 3. Supported WASM Features

### 3.1 Core WASM MVP
- [x] Integer operations (i32, i64)
- [x] Float operations (DISABLED - see Section 3)
- [x] Memory operations
- [x] Control flow (blocks, loops, if/else)
- [x] Function calls
- [x] Global variables
- [x] Import/Export

### 3.2 WASM Extensions
- [x] Bulk memory operations
- [x] Mutable globals
- [x] Non-trapping float-to-int conversions
- [x] Sign extension operations

### 3.3 Disabled Features
- [ ] SIMD operations
- [ ] Reference types
- [ ] Multi-value returns
- [ ] Threads

## 4. Determinism Rules

### 4.1 Prohibited Operations
To ensure deterministic execution across all nodes:

| Category | Operations | Reason |
|:---|:---|:---|
| **Floating-point** | `f32.*`, `f64.*` | Non-deterministic across platforms |
| **Random** | `env.random` | Non-deterministic seed |
| **Time** | `env.time`, `env.clock` | System time leak |
| **System** | `env.syscall` | OS-dependent behavior |

### 4.2 Allowed Host Functions
All host functions are registered under the `"env"` namespace:

```wasm
(import "env" "host_get_caller" (func $host_get_caller (param i32) (result i32)))
(import "env" "host_get_satoshis" (func $host_get_satoshis (result i64)))
(import "env" "host_get_seal" (func $host_get_seal (param i32) (result i32)))
(import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
(import "env" "host_create_object" (func $host_create_object (param i32 i32 i64) (result i32)))
(import "env" "host_stealth_settle" (func $host_stealth_settle (param i32 i64) (result i32)))
(import "env" "host_mweb_peg_out" (func $host_mweb_peg_out (param i32 i64) (result i32)))
(import "env" "host_verify_groth16" (func $host_verify_groth16 (param i32 i32 i32 i32 i32 i32) (result i32)))
(import "env" "abort" (func $abort (param i32 i32 i32 i32)))
```

## 5. Memory Model

### 5.1 Memory Limits
| Parameter | Value | Description |
|:---|:---|:---|
| `min_memory_pages` | 1 | Initial memory (64 KB) |
| `max_memory_pages` | 256 | Maximum memory (16 MB) |
| `static_memory_maximum_size` | 16 MB | Wasmtime static memory cap |

### 5.2 Memory Layout
```
+------------------+ 0x00000000
| Stack            | (grows downward)
+------------------+ 0x00010000
| Heap             | (grows upward)
+------------------+ 0x01000000
| Host buffers     | (read/write)
+------------------+ 0x02000000
```

### 5.3 Host Buffer Protocol
Host functions write data to memory using this protocol:
1. Host allocates buffer in linear memory
2. Host writes data to buffer
3. Host returns pointer and length to WASM
4. WASM reads from pointer

## 6. Execution Model

### 6.1 Fuel Metering
Each WASM instruction consumes fuel:

| Instruction Type | Fuel Cost |
|:---|:---|
| Base instruction | 1 |
| Memory load/store | 2 |
| Function call | 3 |
| Host function call | Per-function cost |

### 6.2 Host Function Fuel Costs
| Host Function | Fuel Cost |
|:---|:---|
| `host_get_caller` | 100 |
| `host_get_satoshis` | 100 |
| `host_get_seal` | 100 |
| `host_emit_event` | 500 |
| `host_create_object` | 1000 |
| `host_stealth_settle` | 500 |
| `host_mweb_peg_out` | 500 |
| `host_verify_groth16` | 10000 |

### 6.3 Trap Conditions
Execution traps (aborts) when:
- Fuel exhausted
- Memory overflow
- Host function returns error
- `abort()` called
- Invalid WASM bytecode

## 7. Contract Interface

### 7.1 Required Exports
Every contract must export these functions:
```typescript
export function allocate(size: i32): usize;
export function deallocate(ptr: usize, size: i32): void;
export function init(args_ptr: usize, args_len: i32): i32;
export function call(method_ptr: usize, args_ptr: usize, args_len: i32): i32;
export function get_state(out_ptr: usize): i32;
```

Optional (recommended for stateful contracts):
```typescript
export function restore_state(state_ptr: usize, state_len: i32): i32;
```

Modules that do not export `allocate` fall back to hardcoded offsets (`0x0500` for method, `0x1000` for args, `0x2000` for state output, `0x3000` for state restore) for backward compatibility. New contracts should always export `allocate`.

### 7.2 Function Signatures

#### `init(args_ptr, args_len) -> i32`
Initialize contract state.
- `args_ptr`: Pointer to initialization arguments (JSON)
- `args_len`: Length of arguments
- Returns: 0 on success, non-zero on error

#### `call(method_ptr, args_ptr, args_len) -> i32`
Execute a contract method.
- `method_ptr`: Pointer to method name (max 64 bytes)
- `args_ptr`: Pointer to method arguments (JSON)
- `args_len`: Length of arguments
- Returns: 0 on success, non-zero on error

#### `get_state(out_ptr) -> i32`
Export current contract state.
- `out_ptr`: Pointer to output buffer
- Returns: Length of state data written

### 7.3 Host Call Sequence

#### deploy()
1. Host calls `allocate(init_args.len())` → gets `args_ptr`
2. Host writes init_args to `args_ptr`
3. Host calls `init(args_ptr, init_args.len())`
4. Host calls `deallocate(args_ptr, init_args.len())`
5. Host calls `get_state_size()` → returns state byte length (0 if unavailable)
6. Host calls `allocate(buf_size)` → gets `state_out_ptr` (buf_size = actual size, capped at 1MB, fallback 64KB)
7. Host calls `get_state(state_out_ptr)` → returns written length
8. Host checks `len <= buf_size` — rejects if overflow
9. Host reads `len` bytes from `state_out_ptr`
10. Host calls `deallocate(state_out_ptr, buf_size)`

#### execute()
1. Host calls `allocate(state_data.len())` → gets `state_ptr`
2. Host writes state_data to `state_ptr`
3. Host calls `restore_state(state_ptr, state_data.len())`
4. Host calls `deallocate(state_ptr, state_data.len())`
5. Host calls `allocate(method_bytes.len())` → gets `method_ptr`
6. Host writes null-terminated method name to `method_ptr`
7. Host calls `allocate(args.len())` → gets `args_ptr`
8. Host writes args to `args_ptr`
9. Host calls `call(method_ptr, args_ptr, args.len())`
10. Host calls `deallocate(method_ptr, ...)`
11. Host calls `deallocate(args_ptr, ...)`
12. Host calls `get_state_size()` → returns state byte length
13. Host calls `allocate(buf_size)` → gets `state_out_ptr`
14. Host calls `get_state(state_out_ptr)` → returns written length
15. Host checks `len <= buf_size` — rejects if overflow
16. Host reads `len` bytes from `state_out_ptr`
17. Host calls `deallocate(state_out_ptr, buf_size)`

### 7.4 Design Notes

- **get_state scratch buffer:** `get_state` does not report its output size before writing. The host allocates a 64 KiB scratch buffer. If the state exceeds this, the read is truncated. A future ABI revision could add a `get_state_size() -> i32` export to allow exact allocation.
- **deallocate is currently a no-op** in AssemblyScript (GC-managed). Calling it keeps the host/guest contract correct for when that changes.
- **Fallback:** If the module does not export `allocate`, the host falls back to hardcoded offsets for backward compatibility.

## 8. AssemblyScript Compilation

### 8.1 Compiler Settings
```json
{
  "extends": "assemblyscript/std/assembly.json",
  "options": {
    "measure": false,
    "noAssert": false,
    "noRuntime": false,
    "exportRuntime": true
  }
}
```

### 8.2 Optimized Build
```bash
asc assembly/index.ts --target release
```

### 8.3 Debug Build
```bash
asc assembly/index.ts --target debug
```

## 9. WASM Binary Format

### 9.1 Header
```
Magic: 0x00 0x61 0x73 0x6D (\0asm)
Version: 0x01 0x00 0x00 0x00
```

### 9.2 Section Order
1. Type section
2. Import section
3. Function section
4. Table section
5. Memory section
6. Global section
7. Export section
8. Start section
9. Element section
10. Code section
11. Data section
12. Custom sections (debug info)

### 9.3 Content Hash
The artifact of record is the SHA-256 hash of the raw WASM bytes (excluding custom sections):

```
hash = SHA256(wasm_bytes)
```

This hash is:
- Stored in deployment transaction
- Verified before execution
- Used for content-addressed storage

## 10. Validation Rules

### 10.1 Pre-Execution Validation
1. WASM magic and version must be valid
2. All imports must be satisfied
3. All exports must have correct signatures
4. Memory must not exceed limits
5. Code must be valid according to WASM spec

### 10.2 Runtime Validation
1. Fuel must be available for each instruction
2. Memory accesses must be within bounds
3. Host function calls must have valid parameters
4. State mutations must be deterministic

## 11. Security Considerations

### 11.1 Memory Safety
- All memory accesses are bounds-checked
- Stack overflow traps execution
- Heap overflow traps execution

### 11.2 Determinism
- No floating-point operations
- No system time access
- No random number generation
- No OS-dependent behavior

### 11.3 Resource Limits
- Fuel limits execution time
- Memory limits space usage
- Host function costs prevent abuse
