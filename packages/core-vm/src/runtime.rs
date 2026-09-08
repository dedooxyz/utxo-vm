use crate::host_functions::HostContext;
use crate::state::{CreatedObject, EventLog, SingleUseSeal, SmartObjectState, StealthSettlement};
use sha2::{Digest, Sha256};
use wasmtime::*;

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

impl VmRuntime {
    pub fn new(config: VmConfig) -> Self {
        let mut wasm_cfg = Config::new();
        wasm_cfg.consume_fuel(true);
        let engine = Engine::new(&wasm_cfg).expect("Failed to initialize Wasmtime engine");
        Self { engine, config }
    }

    pub fn calculate_code_hash(wasm_bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(wasm_bytes);
        hex::encode(hasher.finalize())
    }

    fn read_guest_string(caller: &Caller<'_, HostContext>, mem: &Memory, ptr: usize, max_len: usize) -> String {
        let data = mem.data(caller);
        if ptr >= data.len() {
            return String::new();
        }
        let end = (ptr + max_len).min(data.len());
        let slice = &data[ptr..end];
        // find null terminator if present
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(100)); }
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(100)); }
                caller.data().satoshis
            })
            .map_err(|e| e.to_string())?;

        // host_get_seal(out_ptr: i32) -> i32
        linker
            .func_wrap("env", "host_get_seal", |mut caller: Caller<'_, HostContext>, out_ptr: i32| -> i32 {
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(100)); }
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(500)); }
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(1000)); }
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(500)); }
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
                if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(500)); }
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

        // host_verify_groth16(vk_ptr: i32, vk_len: i32, proof_ptr: i32, proof_len: i32, inputs_ptr: i32, inputs_len: i32) -> i32
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
                    if let Ok(_fuel) = caller.get_fuel() { let _ = caller.set_fuel(_fuel.saturating_sub(10000)); }
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

        // Built-in abort handler for AssemblyScript
        linker
            .func_wrap("env", "abort", |_caller: Caller<'_, HostContext>, _msg: i32, _file: i32, _line: i32, _col: i32| {
                tracing::warn!("[VM] AssemblyScript abort called");
            })
            .map_err(|e| e.to_string())?;

        Ok(linker)
    }

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

        // If 'init' export exists, invoke it
        let mut return_code = 0;
        if let Ok(init_fn) = instance.get_typed_func::<(i32, i32), i32>(&mut store, "init") {
            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| "Memory not found".to_string())?;
            
            // Allocate guest memory or write to reserved buffer at 0x1000
            let args_ptr = 0x1000;
            if !init_args.is_empty() {
                memory
                    .write(&mut store, args_ptr, init_args)
                    .map_err(|e| format!("Memory write error: {}", e))?;
            }
            return_code = init_fn
                .call(&mut store, (args_ptr as i32, init_args.len() as i32))
                .map_err(|e| format!("Init execution error: {}", e))?;
        }

        // Read state if 'get_state' is exported
        let mut state_data = Vec::new();
        if let Ok(get_state_fn) = instance.get_typed_func::<i32, i32>(&mut store, "get_state") {
            let state_out_ptr = 0x2000;
            let len = get_state_fn
                .call(&mut store, state_out_ptr as i32)
                .unwrap_or(0);
            if len > 0 {
                let memory = instance
                    .get_memory(&mut store, "memory")
                    .ok_or_else(|| "Memory not found".to_string())?;
                state_data.resize(len as usize, 0);
                memory
                    .read(&store, state_out_ptr, &mut state_data)
                    .map_err(|e| format!("Memory read error: {}", e))?;
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

        // Write method name and args to guest memory
        let method_ptr = 0x0500;
        let args_ptr = 0x1000;

        let mut method_bytes = method.as_bytes().to_vec();
        method_bytes.push(0); // Null terminator
        memory
            .write(&mut store, method_ptr, &method_bytes)
            .map_err(|e| format!("Failed to write method to guest memory: {}", e))?;

        if !args.is_empty() {
            memory
                .write(&mut store, args_ptr, args)
                .map_err(|e| format!("Failed to write args to guest memory: {}", e))?;
        }

        let mut return_code = 0;
        if let Ok(call_fn) = instance.get_typed_func::<(i32, i32, i32), i32>(&mut store, "call") {
            return_code = call_fn
                .call(
                    &mut store,
                    (method_ptr as i32, args_ptr as i32, args.len() as i32),
                )
                .map_err(|e| format!("Call execution trapped: {}", e))?;
        }

        // Retrieve updated state
        let mut updated_state_data = state.state_data.clone();
        if let Ok(get_state_fn) = instance.get_typed_func::<i32, i32>(&mut store, "get_state") {
            let state_out_ptr = 0x2000;
            let len = get_state_fn
                .call(&mut store, state_out_ptr as i32)
                .unwrap_or(0);
            if len > 0 {
                updated_state_data.resize(len as usize, 0);
                memory
                    .read(&store, state_out_ptr, &mut updated_state_data)
                    .map_err(|e| format!("Memory read error: {}", e))?;
            }
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
