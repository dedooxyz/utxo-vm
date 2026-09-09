# UTXO-VM WASM ISA Specification

This document formalizes the WebAssembly Instruction Set Architecture (ISA) supported by UTXO-VM.

## 1. Overview

UTXO-VM executes WebAssembly (WASM) bytecode via the Wasmtime runtime. All contracts are authored in AssemblyScript (strict TypeScript dialect) and compile to `.wasm` binaries.

## 2. Supported WASM Features

### 2.1 Core WASM MVP
- [x] Integer operations (i32, i64)
- [x] Float operations (DISABLED - see Section 3)
- [x] Memory operations
- [x] Control flow (blocks, loops, if/else)
- [x] Function calls
- [x] Global variables
- [x] Import/Export

### 2.2 WASM Extensions
- [x] Bulk memory operations
- [x] Mutable globals
- [x] Non-trapping float-to-int conversions
- [x] Sign extension operations

### 2.3 Disabled Features
- [ ] SIMD operations
- [ ] Reference types
- [ ] Multi-value returns
- [ ] Threads

## 3. Determinism Rules

### 3.1 Prohibited Operations
To ensure deterministic execution across all nodes:

| Category | Operations | Reason |
|:---|:---|:---|
| **Floating-point** | `f32.*`, `f64.*` | Non-deterministic across platforms |
| **Random** | `env.random` | Non-deterministic seed |
| **Time** | `env.time`, `env.clock` | System time leak |
| **System** | `env.syscall` | OS-dependent behavior |

### 3.2 Allowed Host Functions
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

## 4. Memory Model

### 4.1 Memory Limits
| Parameter | Value | Description |
|:---|:---|:---|
| `min_memory_pages` | 1 | Initial memory (64 KB) |
| `max_memory_pages` | 256 | Maximum memory (16 MB) |
| `static_memory_maximum_size` | 16 MB | Wasmtime static memory cap |

### 4.2 Memory Layout
```
+------------------+ 0x00000000
| Stack            | (grows downward)
+------------------+ 0x00010000
| Heap             | (grows upward)
+------------------+ 0x01000000
| Host buffers     | (read/write)
+------------------+ 0x02000000
```

### 4.3 Host Buffer Protocol
Host functions write data to memory using this protocol:
1. Host allocates buffer in linear memory
2. Host writes data to buffer
3. Host returns pointer and length to WASM
4. WASM reads from pointer

## 5. Execution Model

### 5.1 Fuel Metering
Each WASM instruction consumes fuel:

| Instruction Type | Fuel Cost |
|:---|:---|
| Base instruction | 1 |
| Memory load/store | 2 |
| Function call | 3 |
| Host function call | Per-function cost |

### 5.2 Host Function Fuel Costs
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

### 5.3 Trap Conditions
Execution traps (aborts) when:
- Fuel exhausted
- Memory overflow
- Host function returns error
- `abort()` called
- Invalid WASM bytecode

## 6. Contract Interface

### 6.1 Required Exports
Every contract must export these functions:

```wasm
(func (export "init") (param $args_ptr i32) (param $args_len i32) (result i32))
(func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32))
(func (export "get_state") (param $out_ptr i32) (result i32))
```

### 6.2 Optional Exports
```wasm
(func (export "allocate") (param $size i32) (result i32))
(func (export "deallocate") (param $ptr i32) (param $size i32))
```

### 6.3 Function Signatures

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

## 7. AssemblyScript Compilation

### 7.1 Compiler Settings
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

### 7.2 Optimized Build
```bash
asc assembly/index.ts --target release
```

### 7.3 Debug Build
```bash
asc assembly/index.ts --target debug
```

## 8. WASM Binary Format

### 8.1 Header
```
Magic: 0x00 0x61 0x73 0x6D (\0asm)
Version: 0x01 0x00 0x00 0x00
```

### 8.2 Section Order
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

### 8.3 Content Hash
The artifact of record is the SHA-256 hash of the raw WASM bytes (excluding custom sections):

```
hash = SHA256(wasm_bytes)
```

This hash is:
- Stored in deployment transaction
- Verified before execution
- Used for content-addressed storage

## 9. Validation Rules

### 9.1 Pre-Execution Validation
1. WASM magic and version must be valid
2. All imports must be satisfied
3. All exports must have correct signatures
4. Memory must not exceed limits
5. Code must be valid according to WASM spec

### 9.2 Runtime Validation
1. Fuel must be available for each instruction
2. Memory accesses must be within bounds
3. Host function calls must have valid parameters
4. State mutations must be deterministic

## 10. Security Considerations

### 10.1 Memory Safety
- All memory accesses are bounds-checked
- Stack overflow traps execution
- Heap overflow traps execution

### 10.2 Determinism
- No floating-point operations
- No system time access
- No random number generation
- No OS-dependent behavior

### 10.3 Resource Limits
- Fuel limits execution time
- Memory limits space usage
- Host function costs prevent abuse
