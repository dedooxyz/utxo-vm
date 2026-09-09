import { HostContext } from "./env";
import { SmartObjectToken } from "./sot";
import { SmartObjectNFT } from "./son";
import { NativeVault } from "./native_vault";
import { AtomicSwapOrder } from "./atomic_swap";

// Global contract state as JSON string
let globalStateJson: string = "{}";
// Contract type determined at init
let contractType: string = "";

export function allocate(size: i32): usize {
  let buf = new ArrayBuffer(size);
  return changetype<usize>(buf);
}

export function deallocate(ptr: usize, size: i32): void {
  // AssemblyScript manages garbage collection automatically
}

// Simple JSON value extractor (for flat JSON objects)
function jsonGetString(json: string, key: string): string {
  let search = "\"" + key + "\":\"";
  let start = json.indexOf(search);
  if (start < 0) return "";
  start += search.length;
  let end = json.indexOf("\"", start);
  if (end < 0) return "";
  return json.substring(start, end);
}

function jsonGetInt(json: string, key: string): i64 {
  let search = "\"" + key + "\":";
  let start = json.indexOf(search);
  if (start < 0) return 0;
  let valueStart = start + search.length;
  // Skip whitespace
  while (valueStart < json.length && json.charCodeAt(valueStart) == 32) {
    valueStart++;
  }
  // Handle quoted numbers like "1000000"
  if (valueStart < json.length && json.charCodeAt(valueStart) == 34) {
    // Quoted number
    valueStart++;
    let end = valueStart;
    while (end < json.length && json.charCodeAt(end) != 34) {
      end++;
    }
    if (end == valueStart) return 0;
    return I64.parseInt(json.substring(valueStart, end));
  }
  // Handle unquoted numbers
  let end = valueStart;
  while (end < json.length && json.charCodeAt(end) >= 48 && json.charCodeAt(end) <= 57) {
    end++;
  }
  if (end == valueStart) return 0;
  return I64.parseInt(json.substring(valueStart, end));
}

function jsonGetBool(json: string, key: string): bool {
  let search = "\"" + key + "\":";
  let start = json.indexOf(search);
  if (start < 0) return false;
  start += search.length;
  // Skip whitespace
  while (start < json.length && json.charCodeAt(start) == 32) {
    start++;
  }
  let val = json.substring(start, start + 4);
  return val == "true";
}

export function init(args_ptr: usize, args_len: i32): i32 {
  let args = args_len > 0 ? String.UTF8.decodeUnsafe(args_ptr, args_len, true) : "{}";

  // Parse init args to determine contract type
  contractType = jsonGetString(args, "type");

  // Default to SOT if no type specified (backward compatibility)
  if (contractType.length == 0) {
    // Check if it looks like a token (has symbol field)
    if (jsonGetString(args, "symbol").length > 0) {
      contractType = "SOT";
    }
  }

  // Store initial state
  globalStateJson = args;

  HostContext.emitEvent("Initialized", "type=" + contractType + " state=" + args);
  return 0;
}

export function call(method_ptr: usize, args_ptr: usize, args_len: i32): i32 {
  let method = String.UTF8.decodeUnsafe(method_ptr, 64, true);
  let args = args_len > 0 ? String.UTF8.decodeUnsafe(args_ptr, args_len, true) : "{}";

  HostContext.emitEvent("CallInvoked", "method=" + method + " args=" + args);

  // Dispatch based on contract type and method
  if (contractType == "SOT" || contractType == "UTX20") {
    return callSOT(method, args);
  } else if (contractType == "SON" || contractType == "UTX721") {
    return callSON(method, args);
  } else if (contractType == "NativeVault") {
    return callNativeVault(method, args);
  } else if (contractType == "AtomicSwap") {
    return callAtomicSwap(method, args);
  } else {
    HostContext.emitEvent("Error", "Unknown contract type: " + contractType);
    return 1;
  }
}

function callSOT(method: string, args: string): i32 {
  let name = jsonGetString(globalStateJson, "name");
  let symbol = jsonGetString(globalStateJson, "symbol");
  let decimals = <u8>jsonGetInt(globalStateJson, "decimals");
  let totalSupply = <u64>jsonGetInt(globalStateJson, "totalSupply");
  let balance = <u64>jsonGetInt(globalStateJson, "balance");
  let owner = jsonGetString(globalStateJson, "owner");

  let token = new SmartObjectToken(name, symbol, decimals, totalSupply, owner);
  token.balance = balance;

  if (method == "transfer") {
    let to = jsonGetString(args, "to");
    let amount = <u64>jsonGetInt(args, "amount");

    let newToken = token.transfer(to, amount);
    globalStateJson = newToken.toJson();
    HostContext.emitEvent("TransferComplete", globalStateJson);
    return 0;
  } else if (method == "mint") {
    let to = jsonGetString(args, "to");
    let amount = <u64>jsonGetInt(args, "amount");

    let newToken = token.mint(to, amount);
    globalStateJson = newToken.toJson();
    HostContext.emitEvent("MintComplete", globalStateJson);
    return 0;
  } else if (method == "burn") {
    let amount = <u64>jsonGetInt(args, "amount");

    token.burn(amount);
    globalStateJson = token.toJson();
    HostContext.emitEvent("BurnComplete", globalStateJson);
    return 0;
  } else if (method == "balanceOf") {
    let addr = jsonGetString(args, "address");
    // Return balance for address (simplified - only owner for now)
    if (addr == owner) {
      HostContext.emitEvent("Balance", balance.toString());
    } else {
      HostContext.emitEvent("Balance", "0");
    }
    return 0;
  } else {
    HostContext.emitEvent("Error", "Unknown SOT method: " + method);
    return 1;
  }
}

function callSON(method: string, args: string): i32 {
  let collectionName = jsonGetString(globalStateJson, "collectionName");
  let tokenId = <u64>jsonGetInt(globalStateJson, "tokenId");
  let metadataUri = jsonGetString(globalStateJson, "metadataUri");
  let owner = jsonGetString(globalStateJson, "owner");

  let nft = new SmartObjectNFT(collectionName, tokenId, metadataUri, owner);

  if (method == "transfer") {
    let to = jsonGetString(args, "to");

    nft.transfer(to);
    globalStateJson = nft.toJson();
    HostContext.emitEvent("NFTTransferComplete", globalStateJson);
    return 0;
  } else if (method == "setMetadataUri") {
    let newUri = jsonGetString(args, "uri");

    nft.setMetadataUri(newUri);
    globalStateJson = nft.toJson();
    HostContext.emitEvent("NFTMetadataUpdateComplete", globalStateJson);
    return 0;
  } else if (method == "burn") {
    nft.burn();
    globalStateJson = nft.toJson();
    HostContext.emitEvent("NFTBurnComplete", globalStateJson);
    return 0;
  } else {
    HostContext.emitEvent("Error", "Unknown SON method: " + method);
    return 1;
  }
}

function callNativeVault(method: string, args: string): i32 {
  let totalLockedSatoshis = <u64>jsonGetInt(globalStateJson, "totalLockedSatoshis");
  let vaultOwner = jsonGetString(globalStateJson, "owner");

  let vault = new NativeVault(vaultOwner);
  vault.totalLockedSatoshis = totalLockedSatoshis;

  if (method == "deposit") {
    vault.deposit();
    globalStateJson = vault.toJson();
    HostContext.emitEvent("DepositComplete", globalStateJson);
    return 0;
  } else if (method == "withdrawToStealth") {
    let stealthAddress = jsonGetString(args, "stealthAddress");
    let amount = <u64>jsonGetInt(args, "amount");

    let ok = vault.withdrawToStealth(stealthAddress, amount);
    if (ok) {
      globalStateJson = vault.toJson();
      HostContext.emitEvent("WithdrawComplete", globalStateJson);
    }
    return ok ? 0 : 1;
  } else {
    HostContext.emitEvent("Error", "Unknown NativeVault method: " + method);
    return 1;
  }
}

function callAtomicSwap(method: string, args: string): i32 {
  let maker = jsonGetString(globalStateJson, "maker");
  let offeredTokenId = <u64>jsonGetInt(globalStateJson, "offeredTokenId");
  let demandedSatoshis = <u64>jsonGetInt(globalStateJson, "demandedSatoshis");
  let isFilled = jsonGetBool(globalStateJson, "isFilled");
  let isCancelled = jsonGetBool(globalStateJson, "isCancelled");

  let order = new AtomicSwapOrder(maker, offeredTokenId, demandedSatoshis);
  order.isFilled = isFilled;
  order.isCancelled = isCancelled;

  if (method == "fill") {
    let taker = jsonGetString(args, "taker");

    let ok = order.fill(taker);
    if (ok) {
      globalStateJson = order.toJson();
      HostContext.emitEvent("SwapFillComplete", globalStateJson);
    }
    return ok ? 0 : 1;
  } else if (method == "cancel") {
    let ok = order.cancel();
    if (ok) {
      globalStateJson = order.toJson();
      HostContext.emitEvent("SwapCancelComplete", globalStateJson);
    }
    return ok ? 0 : 1;
  } else {
    HostContext.emitEvent("Error", "Unknown AtomicSwap method: " + method);
    return 1;
  }
}

export function get_state(out_ptr: usize): i32 {
  let buf = String.UTF8.encode(globalStateJson);
  let len = buf.byteLength;
  memory.copy(out_ptr, changetype<usize>(buf), len);
  return len;
}

export function restore_state(state_ptr: usize, state_len: i32): i32 {
  if (state_len <= 0) return 0;
  globalStateJson = String.UTF8.decodeUnsafe(state_ptr, state_len, true);
  // Re-detect contract type from restored state
  contractType = jsonGetString(globalStateJson, "type");
  if (contractType.length == 0) {
    if (jsonGetString(globalStateJson, "symbol").length > 0) {
      contractType = "SOT";
    }
  }
  HostContext.emitEvent("StateRestored", "type=" + contractType);
  return 0;
}
