/*
 * UTXO-VM C ABI — version 1 (frozen)
 *
 * Deterministic WASM execution engine for UTXO smart objects.
 * All requests are JSON; all responses are JSON strings that MUST be
 * released with utxovm_free_string().
 *
 * Response envelope (all functions):
 *   {"success": true,  "result": {...}}
 *   {"success": false, "error": "..."}
 *
 * Changing the request/response schema is consensus-breaking.
 */
#ifndef UTXOVM_H
#define UTXOVM_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Deploy a contract.
 *
 * req JSON: {
 *   wasm_hex:      string (required) — hex-encoded WASM bytecode
 *   caller:        string (optional, default "unknown")
 *   seal_txid:     string (optional, default 64 zeroes)
 *   seal_vout:     number (optional, default 0)
 *   satoshis:      number (optional, default 0)
 *   init_args_hex: string (optional) — hex-encoded init calldata
 * }
 *
 * Returns: JSON string (free with utxovm_free_string).
 *   result: {gas_consumed, return_code, updated_state_hex, events[],
 *            created_objects, stealth_settlements, mweb_peg_outs}
 */
char *utxovm_deploy(const unsigned char *req_ptr, size_t req_len);

/*
 * Execute a method call on an existing object.
 *
 * req JSON: {
 *   wasm_hex:   string (required)
 *   state_hex:  string (optional) — hex-encoded prior state
 *   caller:     string (optional)
 *   method:     string (required)
 *   args_hex:   string (optional)
 *   seal_txid:  string (optional)
 *   seal_vout:  number (optional)
 *   satoshis:   number (optional)
 *   object_id:  string (optional)
 * }
 *
 * Returns: JSON string (free with utxovm_free_string).
 */
char *utxovm_execute(const unsigned char *req_ptr, size_t req_len);

/*
 * Replay a fixture — the "you are not the indexer" check.
 *
 * fixture JSON: {
 *   version: 1,
 *   operations: [
 *     {type: "deploy", wasm_hex, caller?, seal_txid?, seal_vout?,
 *      satoshis?, init_args_hex?, object_id?},
 *     {type: "call",   object_id, method, args_hex?, caller?,
 *      seal_txid?, seal_vout?, satoshis?}
 *   ]
 * }
 *
 * Two independent processes replaying the same fixture MUST return the
 * same result.state_root. If they differ, one of them is lying.
 *
 * Returns: JSON string (free with utxovm_free_string).
 *   result: {state_root, ops_executed, total_gas, object_count,
 *            runtime_version, runtime_hash}
 */
char *utxovm_verify(const unsigned char *fixture_ptr, size_t fixture_len);

/*
 * Compute SHA-256 code hash of raw WASM bytes (NOT JSON).
 * Returns: JSON string → result: {code_hash}
 */
char *utxovm_code_hash(const unsigned char *wasm_ptr, size_t wasm_len);

/*
 * Runtime version info.
 * Returns: JSON string → result: {version, wasmtime_version, runtime_hash}
 */
char *utxovm_runtime_version(void);

/*
 * Free a string returned by any utxovm_* function.
 * Passing NULL is a no-op. Passing any other pointer is UB.
 */
void utxovm_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif /* UTXOVM_H */
