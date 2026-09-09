# JKC-VM WebAssembly ABI Specification

## 1. Memory Layout

WASM modules in JKC-VM communicate with the host via dynamically allocated buffers. The host calls the guest's `allocate(size)` to obtain a pointer before writing or reading data, and `deallocate(ptr, size)` when the buffer is no longer needed.

There are **no fixed memory offsets** — all buffers are allocated on the guest's heap via the exported `allocate`/`deallocate` functions.

**Exception:** Modules that do not export `allocate` fall back to hardcoded offsets (`0x0500` for method, `0x1000` for args, `0x2000` for state output, `0x3000` for state restore) for backward compatibility. New contracts should always export `allocate`.

## 2. Exported Entrypoints

Every compiled smart contract module must export:
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

## 3. Host Calls Sequence

### deploy()

1. Host calls `allocate(init_args.len())` → gets `args_ptr`
2. Host writes init_args to `args_ptr`
3. Host calls `init(args_ptr, init_args.len())`
4. Host calls `deallocate(args_ptr, init_args.len())`
5. Host calls `allocate(65536)` → gets `state_out_ptr` (64 KiB scratch buffer)
6. Host calls `get_state(state_out_ptr)` → returns written length
7. Host reads `len` bytes from `state_out_ptr`
8. Host calls `deallocate(state_out_ptr, 65536)`

### execute()

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
12. Host calls `allocate(65536)` → gets `state_out_ptr`
13. Host calls `get_state(state_out_ptr)` → returns written length
14. Host reads `len` bytes from `state_out_ptr`
15. Host calls `deallocate(state_out_ptr, 65536)`

## 4. Host Imports (`env`)

```wat
(import "env" "host_get_caller" (func $host_get_caller (param i32) (result i32)))
(import "env" "host_get_satoshis" (func $host_get_satoshis (result i64)))
(import "env" "host_get_seal" (func $host_get_seal (param i32) (result i32)))
(import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
(import "env" "host_create_object" (func $host_create_object (param i32 i32 i64) (result i32)))
(import "env" "host_stealth_settle" (func $host_stealth_settle (param i32 i64) (result i32)))
(import "env" "host_mweb_peg_out" (func $host_mweb_peg_out (param i32 i64) (result i32)))
(import "env" "abort" (func $abort (param i32 i32 i32 i32)))
```

## 5. Design Notes

- **get_state scratch buffer:** `get_state` does not report its output size before writing. The host allocates a 64 KiB scratch buffer. If the state exceeds this, the read is truncated. A future ABI revision could add a `get_state_size() -> i32` export to allow exact allocation.
- **deallocate is currently a no-op** in AssemblyScript (GC-managed). Calling it keeps the host/guest contract correct for when that changes.
- **Fallback:** If the module does not export `allocate`, the host falls back to hardcoded offsets for backward compatibility.
