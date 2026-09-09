use crate::host_functions::HostContext;
use crate::state::{CreatedObject, EventLog, SingleUseSeal, SmartObjectState, StealthSettlement};
use sha2::{Digest, Sha256};
use wasmtime::*;

/// Centralized fuel costs for all host calls.
/// One file, one source of truth. Do not scatter magic numbers.
pub mod fuel_costs {
    pub const HOST_GET_CALLER: u64 = 100;
    pub const HOST_GET_SATOSHIS: u64 = 100;
    pub const HOST_GET_SEAL: u64 = 100;
    pub const HOST_EMIT_EVENT: u64 = 500;
    pub const HOST_CREATE_OBJECT: u64 = 1000;
    pub const HOST_STEALTH_SETTLE: u64 = 500;
    pub const HOST_MWEB_PEG_OUT: u64 = 500;
    #[cfg(feature = "experimental-zk")]
    pub const HOST_VERIFY_GROTH16: u64 = 10000;
}

#[derive(Debug, Clone)]
pub struct VmConfig {
    pub max_gas: u64,
    pub max_memory_pages: u32,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            max_gas: 25_000_000,
            max_memory_pages: 32,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub gas_consumed: u64,
    pub return_code: i32,
    pub updated_state_data: Vec<u8>,
    pub events: Vec<EventLog>,
    pub created_objects: Vec<CreatedObject>,
    pub stealth_settlements: Vec<StealthSettlement>,
    pub mweb_peg_outs: Vec<StealthSettlement>,
}

pub struct VmRuntime {
    engine: Engine,
    config: VmConfig,
}

/// Runtime version information for content-addressed verification
#[derive(Debug, Clone)]
pub struct RuntimeVersion {
    pub version: String,
    pub wasmtime_version: String,
    pub runtime_hash: String,
}

impl VmRuntime {
    pub fn new(config: VmConfig) -> Self {
        let mut wasm_cfg = Config::new();
        wasm_cfg.consume_fuel(true);
        // Enforce memory page cap — prevents unbounded memory growth
        let max_bytes = (config.max_memory_pages as u64) * 64 * 1024; // 64 KiB per page
        wasm_cfg.static_memory_maximum_size(max_bytes);
        let engine = Engine::new(&wasm_cfg).expect("Failed to initialize Wasmtime engine");
        Self { engine, config }
    }

    /// Calculate SHA-256 hash of WASM bytecode (artifact of record)
    pub fn calculate_code_hash(wasm_bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(wasm_bytes);
        hex::encode(hasher.finalize())
    }

    /// Calculate content-addressed hash of this runtime binary
    /// This is used for runtime verification - callers can verify they're
    /// running the same runtime version by comparing this hash
    pub fn calculate_runtime_hash() -> String {
        // The runtime hash is the SHA-256 of the compiled binary
        // In production, this would be calculated at build time
        // For now, we return a placeholder that can be verified
        let runtime_bytes = env!("CARGO_PKG_VERSION").as_bytes();
        let mut hasher = Sha256::new();
        hasher.update(b"utxo-core-vm:");
        hasher.update(runtime_bytes);
        hex::encode(hasher.finalize())
    }

    /// Get runtime version information
    pub fn version() -> RuntimeVersion {
        RuntimeVersion {
            version: env!("CARGO_PKG_VERSION").to_string(),
            wasmtime_version: "18.0.2".to_string(), // From Cargo.toml
            runtime_hash: Self::calculate_runtime_hash(),
        }
    }

    fn read_guest_string(caller: &Caller<'_, HostContext>, mem: &Memory, ptr: usize, max_len: usize) -> String {
        let data = mem.data(caller);
        if ptr >= data.len() {
            return String::new();
        }
        let end = (ptr + max_len).min(data.len());
        let slice = &data[ptr..end];
        let str_slice = match slice.iter().position(|&b| b == 0) {
            Some(null_idx) => &slice[..null_idx],
            None => slice,
        };
        String::from_utf8_lossy(str_slice).to_string()
    }

    fn build_linker(&self) -> Result<Linker<HostContext>, String> {
        let mut linker = Linker::new(&self.engine);

        // host_get_caller(out_ptr: i32) -> i32
        linker
            .func_wrap("env", "host_get_caller", |mut caller: Caller<'_, HostContext>, out_ptr: i32| -> i32 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_GET_CALLER)); }
                let caller_str = caller.data().caller.clone();
                let bytes = caller_str.as_bytes();
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    if mem.write(&mut caller, out_ptr as usize, bytes).is_ok() {
                        return bytes.len() as i32;
                    }
                }
                0
            })
            .map_err(|e| e.to_string())?;

        // host_get_satoshis() -> u64
        linker
            .func_wrap("env", "host_get_satoshis", |mut caller: Caller<'_, HostContext>| -> u64 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_GET_SATOSHIS)); }
                caller.data().satoshis
            })
            .map_err(|e| e.to_string())?;

        // host_get_seal(out_ptr: i32) -> i32
        linker
            .func_wrap("env", "host_get_seal", |mut caller: Caller<'_, HostContext>, out_ptr: i32| -> i32 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_GET_SEAL)); }
                let seal_str = format!("{}:{}", caller.data().seal.txid, caller.data().seal.vout);
                let bytes = seal_str.as_bytes();
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    if mem.write(&mut caller, out_ptr as usize, bytes).is_ok() {
                        return bytes.len() as i32;
                    }
                }
                0
            })
            .map_err(|e| e.to_string())?;

        // host_emit_event(topic_ptr: i32, data_ptr: i32, len: i32)
        linker
            .func_wrap("env", "host_emit_event", |mut caller: Caller<'_, HostContext>, topic_ptr: i32, data_ptr: i32, len: i32| {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_EMIT_EVENT)); }
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    let topic = Self::read_guest_string(&caller, &mem, topic_ptr as usize, 64);
                    let mut data_buf = vec![0u8; len.max(0) as usize];
                    if mem.read(&caller, data_ptr as usize, &mut data_buf).is_ok() {
                        let data_str = String::from_utf8_lossy(&data_buf).to_string();
                        caller.data_mut().events.push(EventLog {
                            topic,
                            data: data_str,
                        });
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        // host_create_object(code_hash_ptr: i32, state_ptr: i32, satoshis: u64) -> i32
        linker
            .func_wrap("env", "host_create_object", |mut caller: Caller<'_, HostContext>, code_hash_ptr: i32, state_ptr: i32, satoshis: u64| -> i32 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_CREATE_OBJECT)); }
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    let code_hash = Self::read_guest_string(&caller, &mem, code_hash_ptr as usize, 64);
                    let initial_state = Self::read_guest_string(&caller, &mem, state_ptr as usize, 1024);
                    caller.data_mut().created_objects.push(CreatedObject {
                        code_hash,
                        initial_state: initial_state.into_bytes(),
                        satoshis,
                    });
                    1
                } else {
                    0
                }
            })
            .map_err(|e| e.to_string())?;

        // host_stealth_settle(stealth_addr_ptr: i32, satoshis: u64) -> i32
        linker
            .func_wrap("env", "host_stealth_settle", |mut caller: Caller<'_, HostContext>, stealth_addr_ptr: i32, satoshis: u64| -> i32 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_STEALTH_SETTLE)); }
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    let stealth_address = Self::read_guest_string(&caller, &mem, stealth_addr_ptr as usize, 128);
                    caller.data_mut().stealth_settlements.push(StealthSettlement {
                        stealth_address,
                        satoshis,
                    });
                    1
                } else {
                    0
                }
            })
            .map_err(|e| e.to_string())?;

        // host_mweb_peg_out(stealth_addr_ptr: i32, satoshis: u64) -> i32
        linker
            .func_wrap("env", "host_mweb_peg_out", |mut caller: Caller<'_, HostContext>, stealth_addr_ptr: i32, satoshis: u64| -> i32 {
                if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_MWEB_PEG_OUT)); }
                if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                    let stealth_address = Self::read_guest_string(&caller, &mem, stealth_addr_ptr as usize, 128);
                    caller.data_mut().mweb_peg_outs.push(StealthSettlement {
                        stealth_address,
                        satoshis,
                    });
                    1
                } else {
                    0
                }
            })
            .map_err(|e| e.to_string())?;

        // host_verify_groth16 — only available behind experimental-zk feature
        #[cfg(feature = "experimental-zk")]
        {
            linker
                .func_wrap(
                    "env",
                    "host_verify_groth16",
                    |mut caller: Caller<'_, HostContext>,
                     vk_ptr: i32,
                     vk_len: i32,
                     proof_ptr: i32,
                     proof_len: i32,
                     inputs_ptr: i32,
                     inputs_len: i32|
                     -> i32 {
                        if let Ok(fuel) = caller.get_fuel() { let _ = caller.set_fuel(fuel.saturating_sub(fuel_costs::HOST_VERIFY_GROTH16)); }
                        if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                            let mut vk_buf = vec![0u8; vk_len.max(0) as usize];
                            let mut proof_buf = vec![0u8; proof_len.max(0) as usize];
                            let mut inputs_buf = vec![0u8; inputs_len.max(0) as usize];

                            if vk_len > 0 && mem.read(&caller, vk_ptr as usize, &mut vk_buf).is_err() {
                                return 0;
                            }
                            if proof_len > 0 && mem.read(&caller, proof_ptr as usize, &mut proof_buf).is_err() {
                                return 0;
                            }
                            if inputs_len > 0 && mem.read(&caller, inputs_ptr as usize, &mut inputs_buf).is_err() {
                                return 0;
                            }

                            match crate::zk::verify_groth16(&vk_buf, &proof_buf, &inputs_buf) {
                                Ok(true) => 1,
                                _ => 0,
                            }
                        } else {
                            0
                        }
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        // Built-in abort handler for AssemblyScript — MUST trap, not just warn
        linker
            .func_wrap("env", "abort", |mut _caller: Caller<'_, HostContext>, _msg: i32, _file: i32, _line: i32, _col: i32| -> anyhow::Result<()> {
                tracing::error!("[VM] AssemblyScript abort called — trapping instance");
                Err(anyhow::anyhow!("AssemblyScript abort called"))
            })
            .map_err(|e| e.to_string())?;

        Ok(linker)
    }

    /// Maximum buffer size for get_state output.
    /// get_state doesn't report its own size upfront, so we allocate a fixed
    /// scratch buffer. 64 KiB covers all current contract state payloads.
    /// A future ABI revision could add a `get_state_size()` export to avoid
    /// this fixed cap.
    const GET_STATE_BUF_SIZE: usize = 64 * 1024;

    pub fn deploy(
        &self,
        wasm_bytes: &[u8],
        caller: String,
        seal: SingleUseSeal,
        satoshis: u64,
        init_args: &[u8],
    ) -> Result<ExecutionResult, String> {
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| format!("Module compilation error: {}", e))?;

        let host_ctx = HostContext::new(caller, seal, satoshis);
        let mut store = Store::new(&self.engine, host_ctx);
        store
            .set_fuel(self.config.max_gas)
            .map_err(|e| format!("Fuel setting error: {}", e))?;

        let linker = self.build_linker()?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| "Memory not found".to_string())?;

        let mut return_code = 0;
        if let Ok(init_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut store, "init") {
            // P2: Use allocate() instead of hardcoded offset
            let args_ptr = if !init_args.is_empty() {
                if let Ok(alloc_fn) = instance.get_typed_func::<i32, i32>(&mut store, "allocate") {
                    alloc_fn
                        .call(&mut store, init_args.len() as i32)
                        .map_err(|e| format!("allocate() error: {}", e))?
                } else {
                    // Fallback for modules without allocate — use low memory
                    0x1000
                }
            } else {
                0x1000 // init_args is empty, pointer doesn't matter
            };

            if !init_args.is_empty() {
                memory
                    .write(&mut store, args_ptr as usize, init_args)
                    .map_err(|e| format!("Memory write error: {}", e))?;
            }
            return_code = init_fn
                .call(&mut store, (args_ptr, init_args.len() as i32))
                .map_err(|e| format!("Init execution error: {}", e))?;

            // Deallocate init args buffer
            if !init_args.is_empty() {
                if let Ok(dealloc_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut store, "deallocate") {
                    let _ = dealloc_fn.call(&mut store, (args_ptr, init_args.len() as i32));
                }
            }
        }

        let mut state_data = Vec::new();
        if let Ok(get_state_fn) = instance.get_typed_func::<i32, i32>(&mut store, "get_state") {
            // P2: Allocate scratch buffer for get_state output
            let state_out_ptr = if let Ok(alloc_fn) = instance.get_typed_func::<i32, i32>(&mut store, "allocate") {
                alloc_fn
                    .call(&mut store, Self::GET_STATE_BUF_SIZE as i32)
                    .map_err(|e| format!("allocate() for get_state error: {}", e))?
            } else {
                0x2000
            };

            let len = get_state_fn
                .call(&mut store, state_out_ptr)
                .unwrap_or(0);
            if len > 0 {
                state_data.resize(len as usize, 0);
                memory
                    .read(&store, state_out_ptr as usize, &mut state_data)
                    .map_err(|e| format!("Memory read error: {}", e))?;
            }

            // Deallocate get_state buffer
            if let Ok(dealloc_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut store, "deallocate") {
                let _ = dealloc_fn.call(&mut store, (state_out_ptr, Self::GET_STATE_BUF_SIZE as i32));
            }
        }

        let fuel_left = store.get_fuel().unwrap_or(0);
        let gas_consumed = self.config.max_gas.saturating_sub(fuel_left);

        let data = store.into_data();
        Ok(ExecutionResult {
            gas_consumed,
            return_code,
            updated_state_data: if state_data.is_empty() { init_args.to_vec() } else { state_data },
            events: data.events,
            created_objects: data.created_objects,
            stealth_settlements: data.stealth_settlements,
            mweb_peg_outs: data.mweb_peg_outs,
        })
    }

    pub fn execute(
        &self,
        wasm_bytes: &[u8],
        state: &SmartObjectState,
        caller: String,
        method: &str,
        args: &[u8],
    ) -> Result<ExecutionResult, String> {
        // Enforce code_hash validation: WASM must match pinned hash
        if !state.code_hash.is_empty() {
            let actual_hash = Self::calculate_code_hash(wasm_bytes);
            if actual_hash != state.code_hash {
                return Err(format!(
                    "Code hash mismatch: expected={}, actual={}",
                    state.code_hash, actual_hash
                ));
            }
        }

        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| format!("Module compilation error: {}", e))?;

        let host_ctx = HostContext::from_state(state, caller);
        let mut store = Store::new(&self.engine, host_ctx);
        store
            .set_fuel(self.config.max_gas)
            .map_err(|e| format!("Fuel setting error: {}", e))?;

        let linker = self.build_linker()?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| "Module does not export 'memory'".to_string())?;

        // Helper: allocate via guest, fallback to hardcoded offset.
        // WASM only has i32/i64, so all pointers are i32.
        let allocate_guest = |store: &mut Store<HostContext>, size: i32| -> i32 {
            if let Ok(alloc_fn) = instance.get_typed_func::<i32, i32>(&mut *store, "allocate") {
                alloc_fn.call(&mut *store, size).unwrap_or(0x1000)
            } else {
                0x1000
            }
        };

        let deallocate_guest = |store: &mut Store<HostContext>, ptr: i32, size: i32| {
            if let Ok(dealloc_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut *store, "deallocate") {
                let _ = dealloc_fn.call(&mut *store, (ptr, size));
            }
        };

        // P2: Restore state via allocate()
        if !state.state_data.is_empty() {
            let state_ptr = allocate_guest(&mut store, state.state_data.len() as i32);
            memory
                .write(&mut store, state_ptr as usize, &state.state_data)
                .map_err(|e| format!("Failed to write state to guest memory: {}", e))?;

            if let Ok(restore_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut store, "restore_state") {
                restore_fn
                    .call(&mut store, (state_ptr, state.state_data.len() as i32))
                    .map_err(|e| format!("State restore error: {}", e))?;
            }

            deallocate_guest(&mut store, state_ptr, state.state_data.len() as i32);
        }

        // P2: Allocate for method name
        let mut method_bytes = method.as_bytes().to_vec();
        method_bytes.push(0); // null terminator
        let method_ptr = allocate_guest(&mut store, method_bytes.len() as i32);
        memory
            .write(&mut store, method_ptr as usize, &method_bytes)
            .map_err(|e| format!("Failed to write method to guest memory: {}", e))?;

        // P2: Allocate for args
        let args_ptr = if !args.is_empty() {
            let ptr = allocate_guest(&mut store, args.len() as i32);
            memory
                .write(&mut store, ptr as usize, args)
                .map_err(|e| format!("Failed to write args to guest memory: {}", e))?;
            ptr
        } else {
            0x1000 // dummy pointer for empty args
        };

        let mut return_code = 0;
        if let Ok(call_fn) = instance.get_typed_func::<(i32, i32, i32), i32>(&mut store, "call") {
            return_code = call_fn
                .call(
                    &mut store,
                    (method_ptr, args_ptr, args.len() as i32),
                )
                .map_err(|e| format!("Call execution trapped: {}", e))?;
        }

        // Deallocate method and args
        deallocate_guest(&mut store, method_ptr, method_bytes.len() as i32);
        if !args.is_empty() {
            deallocate_guest(&mut store, args_ptr, args.len() as i32);
        }

        // P2: Read state via allocated buffer
        let mut updated_state_data = state.state_data.clone();
        if let Ok(get_state_fn) = instance.get_typed_func::<i32, i32>(&mut store, "get_state") {
            let state_out_ptr = allocate_guest(&mut store, Self::GET_STATE_BUF_SIZE as i32);
            let len = get_state_fn
                .call(&mut store, state_out_ptr)
                .unwrap_or(0);
            if len > 0 {
                updated_state_data.resize(len as usize, 0);
                memory
                    .read(&store, state_out_ptr as usize, &mut updated_state_data)
                    .map_err(|e| format!("Memory read error: {}", e))?;
            }
            deallocate_guest(&mut store, state_out_ptr, Self::GET_STATE_BUF_SIZE as i32);
        }

        let fuel_left = store.get_fuel().unwrap_or(0);
        let gas_consumed = self.config.max_gas.saturating_sub(fuel_left);

        let data = store.into_data();
        Ok(ExecutionResult {
            gas_consumed,
            return_code,
            updated_state_data,
            events: data.events,
            created_objects: data.created_objects,
            stealth_settlements: data.stealth_settlements,
            mweb_peg_outs: data.mweb_peg_outs,
        })
    }
}
