//! Fixture replay tests — the "you are not the indexer" check.
//!
//! A fixture is a JSON document of ordered deploy/call operations.
//! Replaying it must produce an identical state_root every time —
//! in the same process, in a fresh process, or on another machine.

use std::io::Write;
use std::process::{Command, Stdio};
use utxo_core_vm::fixture::verify_fixture;

/// A minimal WAT counter contract: init() sets counter=0,
/// restore_state() loads the 4-byte LE counter from prior state,
/// call("increment") increments it, get_state() returns the 4-byte LE value.
fn counter_wat() -> Vec<u8> {
    let wat = r#"
        (module
            (import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0x0100) "inc")
            (data (i32.const 0x0200) "counter incremented")

            (global $counter (mut i32) (i32.const 0))

            (func (export "init") (param $args_ptr i32) (param $args_len i32) (result i32)
                (i32.const 0)
            )

            (func (export "allocate") (param $size i32) (result i32)
                (i32.const 0x1000)
            )

            ;; Load prior state: read 4-byte LE counter into the global.
            (func (export "restore_state") (param $state_ptr i32) (param $state_len i32) (result i32)
                (if (i32.ge_s (local.get $state_len) (i32.const 4))
                    (then
                        (global.set $counter (i32.load (local.get $state_ptr)))
                    )
                )
                (i32.const 0)
            )

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
                (call $host_emit_event (i32.const 0x0100) (i32.const 0x0200) (i32.const 20))
                (i32.const 0)
            )

            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.store (local.get $out_ptr) (global.get $counter))
                (i32.const 4)
            )
        )
    "#;
    wat::parse_str(wat).expect("WAT parse failed")
}

fn make_fixture(wasm_hex: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "operations": [
            {
                "type": "deploy",
                "wasm_hex": wasm_hex,
                "object_id": "counter1",
                "caller": "alice",
                "seal_txid": "11".repeat(32),
                "seal_vout": 0,
                "satoshis": 100_000,
                "init_args_hex": ""
            },
            {
                "type": "call",
                "object_id": "counter1",
                "method": "increment",
                "caller": "alice",
                "seal_txid": "22".repeat(32),
                "seal_vout": 0,
                "satoshis": 100_000
            },
            {
                "type": "call",
                "object_id": "counter1",
                "method": "increment",
                "caller": "alice",
                "seal_txid": "33".repeat(32),
                "seal_vout": 0,
                "satoshis": 100_000
            }
        ]
    })
}

/// In-process replay: verify_fixture twice on the same bytes → same root.
#[test]
fn test_fixture_replay_identical_root() {
    let wasm = counter_wat();
    let fixture = make_fixture(&hex::encode(&wasm));
    let fixture_bytes = serde_json::to_vec(&fixture).unwrap();

    let r1 = verify_fixture(&fixture_bytes).expect("replay 1 failed");
    let r2 = verify_fixture(&fixture_bytes).expect("replay 2 failed");

    assert_eq!(r1["state_root"], r2["state_root"], "roots must match");
    assert_eq!(r1["ops_executed"], 3);
    assert_eq!(r1["object_count"], 1);
    assert_eq!(r1["total_gas"], r2["total_gas"]);
    // Root must be a 64-char hex string
    assert_eq!(r1["state_root"].as_str().unwrap().len(), 64);
}

/// Two-process replay: spawn the CLI binary twice on the same fixture.
/// This is the real "you are not the indexer" proof — independent processes
/// must agree on the state root.
#[test]
fn test_fixture_replay_two_processes() {
    let wasm = counter_wat();
    let fixture = serde_json::json!({
        "command": "verify",
        "args": make_fixture(&hex::encode(&wasm))
    });
    let input = serde_json::to_vec(&fixture).unwrap();

    let bin = env!("CARGO_BIN_EXE_utxo-core-vm-cli");
    let run_once = || -> serde_json::Value {
        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn utxo-core-vm-cli");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&input)
            .expect("write stdin failed");
        let out = child.wait_with_output().expect("wait failed");
        assert!(out.status.success(), "CLI exited non-zero");
        serde_json::from_slice(&out.stdout).expect("CLI output not JSON")
    };

    let resp1 = run_once();
    let resp2 = run_once();

    assert_eq!(resp1["success"], true, "resp1 failed: {:?}", resp1);
    assert_eq!(resp2["success"], true, "resp2 failed: {:?}", resp2);
    assert_eq!(
        resp1["result"]["state_root"], resp2["result"]["state_root"],
        "two processes must produce identical state_root"
    );
}

/// Fixture determinism: state root changes when the op list changes,
/// stays the same when reordered fields produce identical semantics.
#[test]
fn test_fixture_root_changes_with_ops() {
    let wasm = counter_wat();
    let mut fixture_a = make_fixture(&hex::encode(&wasm));
    let mut fixture_b = make_fixture(&hex::encode(&wasm));

    // fixture_b has one extra call op → different root
    let ops_b = fixture_b["operations"].as_array_mut().unwrap();
    ops_b.push(serde_json::json!({
        "type": "call", "object_id": "counter1", "method": "increment",
        "caller": "alice", "seal_txid": "44".repeat(32), "seal_vout": 0,
        "satoshis": 100_000
    }));

    let root_a = verify_fixture(&serde_json::to_vec(&fixture_a).unwrap()).unwrap()["state_root"].clone();
    let root_b = verify_fixture(&serde_json::to_vec(&fixture_b).unwrap()).unwrap()["state_root"].clone();
    assert_ne!(root_a, root_b, "different op sequences must produce different roots");

    // Same ops, different caller on first call → same root (caller is metadata,
    // not state) — but wait: caller IS part of the host context and CAN affect
    // state via host_get_caller. This counter contract ignores caller, so
    // roots should match.
    fixture_a["operations"][1]["caller"] = serde_json::json!("bob");
    let root_a2 = verify_fixture(&serde_json::to_vec(&fixture_a).unwrap()).unwrap()["state_root"].clone();
    assert_eq!(root_a, root_a2, "caller change on caller-agnostic contract must not change root");
}

/// Error paths: bad version, unknown op type, call to unknown object.
#[test]
fn test_fixture_error_paths() {
    let wasm = counter_wat();
    let wh = hex::encode(&wasm);

    // Bad version
    let bad_version = serde_json::json!({"version": 2, "operations": []});
    let err = verify_fixture(&serde_json::to_vec(&bad_version).unwrap()).unwrap_err();
    assert!(err.contains("unsupported fixture version"), "got: {}", err);

    // Unknown op type
    let bad_op = serde_json::json!({
        "version": 1,
        "operations": [{"type": "explode", "wasm_hex": wh}]
    });
    let err = verify_fixture(&serde_json::to_vec(&bad_op).unwrap()).unwrap_err();
    assert!(err.contains("unknown op type"), "got: {}", err);

    // Call to non-existent object
    let bad_call = serde_json::json!({
        "version": 1,
        "operations": [{"type": "call", "object_id": "ghost", "method": "m"}]
    });
    let err = verify_fixture(&serde_json::to_vec(&bad_call).unwrap()).unwrap_err();
    assert!(err.contains("unknown object_id"), "got: {}", err);

    // Malformed JSON
    let err = verify_fixture(b"not json").unwrap_err();
    assert!(err.contains("invalid fixture JSON"), "got: {}", err);
}
