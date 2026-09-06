import { HostContext } from "./env";
import { UTX20Token } from "./utx20";
import { UTX721NFT } from "./utx721";
import { NativeVault } from "./native_vault";
import { AtomicSwapOrder } from "./atomic_swap";

// Global contract instance slot for WASM module execution
let globalStateJson: string = "{}";

export function allocate(size: i32): usize {
  let buf = new ArrayBuffer(size);
  return changetype<usize>(buf);
}

export function deallocate(ptr: usize, size: i32): void {
  // AssemblyScript manages garbage collection automatically
}

export function init(args_ptr: usize, args_len: i32): i32 {
  if (args_len <= 0) {
    let caller = HostContext.getCaller();
    globalStateJson = "{\"owner\":\"" + caller + "\"}";
    return 0;
  }
  let str = String.UTF8.decodeUnsafe(args_ptr, args_len, true);
  globalStateJson = str;
  HostContext.emitEvent("Initialized", str);
  return 0;
}

export function call(method_ptr: usize, args_ptr: usize, args_len: i32): i32 {
  let method = String.UTF8.decodeUnsafe(method_ptr, 64, true);
  let args = args_len > 0 ? String.UTF8.decodeUnsafe(args_ptr, args_len, true) : "";

  HostContext.emitEvent("CallInvoked", "Method: " + method);
  return 0;
}

export function get_state(out_ptr: usize): i32 {
  let buf = String.UTF8.encode(globalStateJson);
  let len = buf.byteLength;
  memory.copy(out_ptr, changetype<usize>(buf), len);
  return len;
}
