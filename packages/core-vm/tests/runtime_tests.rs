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

            ;; Data section for event topic and data
            (data (i32.const 0x0100) "Transfer")
            (data (i32.const 0x0200) "Transferred 100 tokens")
            (data (i32.const 0x0300) "stealth_jkc_alice_ephemeral")

            (func (export "init") (param $args_ptr i32) (param $args_len i32) (result i32)
                (i32.const 0)
            )

            (func (export "call") (param $method_ptr i32) (param $args_ptr i32) (param $args_len i32) (result i32)
                ;; Emit Event
                (call $host_emit_event (i32.const 0x0100) (i32.const 0x0200) (i32.const 22))

                ;; Stealth Settle
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

        // 1. Deploy
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

        // 2. Execute Method Call
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
