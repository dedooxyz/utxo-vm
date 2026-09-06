# JKC-VM WebAssembly ABI Specification

## 1. Memory Layout

WASM modules in JKC-VM use a standardized memory layout for host-guest communication:
* **`0x0000 - 0x0FFF`**: Reserved scratch memory / return buffer.
* **`0x1000 - ...`**: Dynamic heap managed by AssemblyScript runtime.

## 2. Exported Entrypoints

Every compiled smart contract module must export:
```typescript
export function allocate(size: i32): i32;
export function deallocate(ptr: i32, size: i32): void;
export function init(args_ptr: i32, args_len: i32): i32;
export function call(method_ptr: i32, args_ptr: i32, args_len: i32): i32;
export function get_state(out_ptr: i32): i32;
```

## 3. Host Imports (`env`)

```wat
(import "env" "host_get_caller" (func $host_get_caller (param i32) (result i32)))
(import "env" "host_get_satoshis" (func $host_get_satoshis (result i64)))
(import "env" "host_get_seal" (func $host_get_seal (param i32) (result i32)))
(import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
(import "env" "host_create_object" (func $host_create_object (param i32 i32 i64) (result i32)))
(import "env" "host_mweb_peg_out" (func $host_mweb_peg_out (param i32 i64) (result i32)))
```
