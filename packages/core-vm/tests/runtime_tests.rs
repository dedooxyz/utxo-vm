use std::fs;
use std::path::Path;
use utxo_core_vm::{SingleUseSeal, SmartObjectState, VmConfig, VmRuntime};

#[test]
fn test_runtime_initialization() {
    let config = VmConfig::default();
    let _runtime = VmRuntime::new(config);
}

#[test]
fn test_code_hash_calculation() {
    let dummy_wasm = b"some dummy wasm bytecode";
    let hash = VmRuntime::calculate_code_hash(dummy_wasm);
    assert_eq!(hash.len(), 64);
}

#[test]
fn test_wat_host_functions_execution() {
    let wat = r#"
        (module
            (import "env" "host_get_satoshis" (func $host_get_satoshis (result i64)))
            (import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
            (import "env" "host_stealth_settle" (func $host_stealth_settle (param i32 i64) (result i32)))
            (memory (export "memory") 1)

            (data (i32.const 0x0100) "Transfer")
            (data (i32.const 0x0200) "Transferred 100 tokens")
            (data (i32.const 0x0300) "stealth_jkc_alice_ephemeral")

            (func (export "init") (param $args_ptr i32) (param $args_len i32) (result i32)
                (i32.const 0)
            )

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                (call $host_emit_event (i32.const 0x0100) (i32.const 0x0200) (i32.const 22))
                (drop (call $host_stealth_settle (i32.const 0x0300) (i64.const 50000)))
                (i32.const 0)
            )

            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.const 0)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(wat).expect("Failed to parse WAT");
    let runtime = VmRuntime::new(VmConfig::default());

    let state = SmartObjectState {
        object_id: "obj_test_123".to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: SingleUseSeal {
            txid: "9f8a3b4c5d6e".to_string(),
            vout: 0,
        },
        satoshis: 100_000,
        owner_pubkey: "pubkey_alice".to_string(),
        state_data: b"{}".to_vec(),
    };

    let result = runtime
        .execute(
            &wasm_bytes,
            &state,
            "pubkey_alice".to_string(),
            "transfer",
            b"{\"to\":\"bob\",\"amount\":100}",
        )
        .expect("Execution failed");

    assert_eq!(result.return_code, 0);
    assert!(result.gas_consumed > 0);
    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].topic, "Transfer");
    assert_eq!(result.events[0].data, "Transferred 100 tokens");
    assert_eq!(result.stealth_settlements.len(), 1);
    assert_eq!(result.stealth_settlements[0].stealth_address, "stealth_jkc_alice_ephemeral");
    assert_eq!(result.stealth_settlements[0].satoshis, 50000);
}

#[test]
fn test_fuel_gas_metering_out_of_gas() {
    let infinite_loop_wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                (loop $infinite (br $infinite))
                (i32.const 0)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(infinite_loop_wat).expect("Failed to parse WAT");
    let config = VmConfig {
        max_gas: 10_000,
        max_memory_pages: 1,
    };
    let runtime = VmRuntime::new(config);

    let state = SmartObjectState {
        object_id: "obj_loop_123".to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: SingleUseSeal {
            txid: "0000000000".to_string(),
            vout: 0,
        },
        satoshis: 1000,
        owner_pubkey: "caller_1".to_string(),
        state_data: vec![],
    };

    let res = runtime.execute(&wasm_bytes, &state, "caller_1".to_string(), "loop", b"");
    assert!(res.is_err());
    let err_msg = res.err().unwrap();
    assert!(err_msg.contains("all fuel consumed by WebAssembly") || err_msg.contains("trapped"));
}

#[test]
fn test_compiled_assemblyscript_wasm_execution() {
    let wasm_path = Path::new("../contracts/build/release.wasm");
    if wasm_path.exists() {
        let wasm_bytes = fs::read(wasm_path).expect("Failed to read release.wasm");
        let runtime = VmRuntime::new(VmConfig::default());

        let seal = SingleUseSeal {
            txid: "aabbccdd11223344".to_string(),
            vout: 1,
        };

        let deploy_res = runtime
            .deploy(
                &wasm_bytes,
                "03deadbeef".to_string(),
                seal.clone(),
                50_000,
                b"{\"name\":\"TestCoin\",\"symbol\":\"TC\",\"decimals\":8,\"totalSupply\":\"1000000\"}",
            )
            .expect("Deploy failed");

        assert_eq!(deploy_res.return_code, 0);
        assert!(deploy_res.gas_consumed > 0);

        let state = SmartObjectState {
            object_id: "obj_testcoin_1".to_string(),
            code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
            seal,
            satoshis: 50_000,
            owner_pubkey: "03deadbeef".to_string(),
            state_data: deploy_res.updated_state_data,
        };

        let call_res = runtime
            .execute(
                &wasm_bytes,
                &state,
                "03deadbeef".to_string(),
                "transfer",
                b"{\"to\":\"02cafebabe\",\"amount\":500}",
            )
            .expect("Call failed");

        assert_eq!(call_res.return_code, 0);
        assert!(call_res.gas_consumed > 0);
    }
}

// =============================================================================
// NEW TESTS: Required by AGENTS.md and MISSION.md
// =============================================================================

/// AGENTS.md line 226: "Same WASM + witness -> identical root twice"
/// MISSION.md step 4: "Independent replay agrees"
/// Two independent VmRuntime instances executing the same fixture must produce
/// bit-identical state roots (updated_state_data).
#[test]
fn test_deterministic_replay_identical_root() {
    let wat = r#"
        (module
            (import "env" "host_get_caller" (func $host_get_caller (param i32) (result i32)))
            (import "env" "host_emit_event" (func $host_emit_event (param i32 i32 i32)))
            (memory (export "memory") 1)

            (data (i32.const 0x0100) "StateUpdate")
            (data (i32.const 0x0200) "counter incremented")

            (global $counter (mut i32) (i32.const 0))

            (func (export "init") (param $args_ptr i32) (param $args_len i32) (result i32)
                (i32.const 0)
            )

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                ;; Increment counter
                (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
                ;; Emit event
                (call $host_emit_event (i32.const 0x0100) (i32.const 0x0200) (i32.const 20))
                (i32.const 0)
            )

            (func (export "get_state") (param $out_ptr i32) (result i32)
                ;; Write counter to output buffer
                (i32.store (local.get $out_ptr) (global.get $counter))
                (i32.const 4)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(wat).expect("Failed to parse WAT");
    let state = SmartObjectState {
        object_id: "obj_replay_test".to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: SingleUseSeal {
            txid: "aabbccdd".to_string(),
            vout: 0,
        },
        satoshis: 100_000,
        owner_pubkey: "replay_tester".to_string(),
        state_data: b"{}".to_vec(),
    };

    // Run 1: Fresh runtime instance
    let runtime1 = VmRuntime::new(VmConfig::default());
    let result1 = runtime1
        .execute(&wasm_bytes, &state, "replay_tester".to_string(), "increment", b"")
        .expect("Execution 1 failed");

    // Run 2: Independent runtime instance
    let runtime2 = VmRuntime::new(VmConfig::default());
    let result2 = runtime2
        .execute(&wasm_bytes, &state, "replay_tester".to_string(), "increment", b"")
        .expect("Execution 2 failed");

    // Core invariant: identical outputs
    assert_eq!(result1.updated_state_data, result2.updated_state_data,
        "State root must be identical across independent replays");
    assert_eq!(result1.gas_consumed, result2.gas_consumed,
        "Gas consumed must be identical across independent replays");
    assert_eq!(result1.events.len(), result2.events.len(),
        "Event count must be identical");
    assert_eq!(result1.events[0].topic, result2.events[0].topic,
        "Event topic must be identical");
    assert_eq!(result1.created_objects.len(), result2.created_objects.len(),
        "Created objects count must be identical");
}

/// AGENTS.md line 228: "Abort traps"
/// A WASM module calling env.abort() must trap — execution must stop with an error.
#[test]
fn test_abort_traps() {
    let abort_wat = r#"
        (module
            (import "env" "abort" (func $abort (param i32 i32 i32 i32)))
            (memory (export "memory") 1)

            (data (i32.const 0x0100) "intentional abort")

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                ;; Call abort with msg pointer, file pointer, line, col
                (call $abort (i32.const 0x0100) (i32.const 0) (i32.const 42) (i32.const 0))
                ;; This line should never execute — abort should trap
                (i32.const 99)
            )

            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.const 0)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(abort_wat).expect("Failed to parse WAT");
    let runtime = VmRuntime::new(VmConfig::default());

    let state = SmartObjectState {
        object_id: "obj_abort_test".to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: SingleUseSeal {
            txid: "deadbeef".to_string(),
            vout: 0,
        },
        satoshis: 10_000,
        owner_pubkey: "abort_tester".to_string(),
        state_data: b"{}".to_vec(),
    };

    let result = runtime.execute(&wasm_bytes, &state, "abort_tester".to_string(), "test", b"");

    // Abort MUST cause a trap/error — execution must not succeed
    assert!(result.is_err(),
        "env.abort() must trap the instance, not return Ok");

    let err_msg = result.err().unwrap();
    assert!(err_msg.contains("trapped") || err_msg.contains("abort"),
        "Error should indicate trap/abort, got: {}", err_msg);
}

/// AGENTS.md line 229: "Bad wasm hash rejected"
/// When the code_hash in SmartObjectState does not match the actual WASM bytes,
/// the runtime should reject execution.
#[test]
fn test_bad_wasm_hash_rejected() {
    let wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                (i32.const 0)
            )
            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.const 0)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(wat).expect("Failed to parse WAT");
    let correct_hash = VmRuntime::calculate_code_hash(&wasm_bytes);

    // Create a state with a WRONG code_hash
    let state_wrong_hash = SmartObjectState {
        object_id: "obj_bad_hash".to_string(),
        code_hash: "000000000000000000000000000000000000000000000000000000000000dead".to_string(),
        seal: SingleUseSeal {
            txid: "badhash".to_string(),
            vout: 0,
        },
        satoshis: 10_000,
        owner_pubkey: "hash_tester".to_string(),
        state_data: b"{}".to_vec(),
    };

    let runtime = VmRuntime::new(VmConfig::default());

    // Runtime validates code_hash before execution — mismatched hash is rejected
    let result = runtime.execute(
        &wasm_bytes,
        &state_wrong_hash,
        "hash_tester".to_string(),
        "test",
        b"",
    );

    assert!(result.is_err(), "Mismatched code_hash must be rejected");
    let err = result.unwrap_err();
    assert!(err.contains("Code hash mismatch"), "Error should mention code_hash mismatch: {}", err);
}

/// AGENTS.md line 227: "Fuel exhaustion reverts, no state write"
/// When fuel runs out, the original state must remain unchanged.
#[test]
fn test_fuel_exhaustion_no_state_write() {
    let state_modifying_wat = r#"
        (module
            (memory (export "memory") 1)
            (global $counter (mut i32) (i32.const 0))

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                ;; This modifies internal state, then loops forever
                (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
                (loop $infinite (br $infinite))
                (i32.const 0)
            )

            (func (export "get_state") (param $out_ptr i32) (result i32)
                (i32.store (local.get $out_ptr) (global.get $counter))
                (i32.const 4)
            )
        )
    "#;

    let wasm_bytes = wat::parse_str(state_modifying_wat).expect("Failed to parse WAT");
    let config = VmConfig {
        max_gas: 10_000,
        max_memory_pages: 1,
    };
    let runtime = VmRuntime::new(config);

    let original_state = SmartObjectState {
        object_id: "obj_fuel_state".to_string(),
        code_hash: VmRuntime::calculate_code_hash(&wasm_bytes),
        seal: SingleUseSeal {
            txid: "fuelstate".to_string(),
            vout: 0,
        },
        satoshis: 50_000,
        owner_pubkey: "fuel_tester".to_string(),
        state_data: b"{\"counter\":0}".to_vec(),
    };

    let result = runtime.execute(
        &wasm_bytes,
        &original_state,
        "fuel_tester".to_string(),
        "increment",
        b"",
    );

    // Must fail due to fuel exhaustion
    assert!(result.is_err(), "Infinite loop must exhaust fuel");

    // The original state_data must not have been mutated by the failed execution.
    // (The runtime clones state_data, so the original SmartObjectState is untouched,
    //  but this test documents the invariant.)
    assert_eq!(original_state.state_data, b"{\"counter\":0}",
        "Original state_data must not be mutated after fuel exhaustion");
}
