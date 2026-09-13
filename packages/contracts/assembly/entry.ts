import { HostContext } from "./env";
import { JsonParser, JsonValue, JsonType } from "./json";
import { SmartObjectToken } from "./sot";
import { SmartObjectNFT } from "./son";
import { NativeVault } from "./native_vault";
import { AtomicSwapOrder } from "./atomic_swap";

// Global contract state as JSON string
let globalStateJson: string = "{}";
// Contract type determined at init
let contractType: string = "";

export function allocate(size: i32): usize {
  return heap.alloc(size);
}

export function deallocate(ptr: usize, size: i32): i32 {
  heap.free(ptr);
  return 0;
}

// JSON value extractor helpers backed by robust JsonParser
export function jsonGetString(json: string, key: string): string {
  let parser = new JsonParser(json);
  let v = parser.parse();
  if (v == null) return "";
  return v.getString(key);
}

export function jsonGetInt(json: string, key: string): i64 {
  let parser = new JsonParser(json);
  let v = parser.parse();
  if (v == null) return 0;
  return v.getInt(key);
}

export function jsonGetBool(json: string, key: string): bool {
  let parser = new JsonParser(json);
  let v = parser.parse();
  if (v == null) return false;
  return v.getBool(key);
}

function readCString(ptr: usize, maxLen: i32 = 64): string {
  let len = 0;
  while (len < maxLen && load<u8>(ptr + len) != 0) {
    len++;
  }
  return len > 0 ? String.UTF8.decodeUnsafe(ptr, len, true) : "";
}

export function init(args_ptr: usize, args_len: i32): i32 {
  let args = args_len > 0 ? String.UTF8.decodeUnsafe(args_ptr, args_len, true) : "{}";

  let parser = new JsonParser(args);
  let parsed = parser.parse();
  if (parsed == null || parsed.type != JsonType.Object) {
    HostContext.emitEvent("Error", "Invalid init JSON: " + parser.errorMsg);
    return 1;
  }

  // Parse init args to determine contract type
  contractType = parsed.getString("type");

  // Default to SOT if no type specified (backward compatibility)
  if (contractType.length == 0) {
    // Check if it looks like a token (has symbol field)
    if (parsed.getString("symbol").length > 0) {
      contractType = "SOT";
    }
  }

  // Store initial state
  globalStateJson = args;

  HostContext.emitEvent("Initialized", "type=" + contractType + " state=" + args);
  return 0;
}

export function call(method_ptr: usize, args_ptr: usize, args_len: i32): i32 {
  let method = readCString(method_ptr, 64);
  let args = args_len > 0 ? String.UTF8.decodeUnsafe(args_ptr, args_len, true) : "{}";

  // Validate args JSON
  let argsParser = new JsonParser(args);
  let parsedArgs = argsParser.parse();
  if (parsedArgs == null || parsedArgs.type != JsonType.Object) {
    HostContext.emitEvent("Error", "Invalid call args JSON: " + argsParser.errorMsg);
    return 1;
  }

  // Validate global state JSON
  let stateParser = new JsonParser(globalStateJson);
  let parsedState = stateParser.parse();
  if (parsedState == null || parsedState.type != JsonType.Object) {
    HostContext.emitEvent("Error", "Invalid contract state JSON: " + stateParser.errorMsg);
    return 1;
  }

  HostContext.emitEvent("CallInvoked", "method=" + method + " args=" + args);

  // Dispatch based on contract type and method
  if (contractType == "SOT" || contractType == "UTX20") {
    return callSOT(method, parsedState, parsedArgs);
  } else if (contractType == "SON" || contractType == "UTX721") {
    return callSON(method, parsedState, parsedArgs);
  } else if (contractType == "NativeVault") {
    return callNativeVault(method, parsedState, parsedArgs);
  } else if (contractType == "AtomicSwap") {
    return callAtomicSwap(method, parsedState, parsedArgs);
  } else {
    HostContext.emitEvent("Error", "Unknown contract type: " + contractType);
    return 1;
  }
}

function callSOT(method: string, state: JsonValue, args: JsonValue): i32 {
  let name = state.getString("name");
  let symbol = state.getString("symbol");
  let decimals = <u8>state.getInt("decimals");
  let totalSupply = state.has("localSupply") ? <u64>state.getInt("localSupply") : <u64>state.getInt("totalSupply");
  let balance = <u64>state.getInt("balance");
  let owner = state.getString("owner");

  let token = new SmartObjectToken(name, symbol, decimals, totalSupply, owner);
  token.balance = balance;

  if (method == "transfer") {
    let to = args.getString("to");
    let amount = <u64>args.getInt("amount");

    let newToken = token.transfer(to, amount);
    globalStateJson = newToken.toJson();
    HostContext.emitEvent("TransferComplete", globalStateJson);
    return 0;
  } else if (method == "mint") {
    let to = args.getString("to");
    let amount = <u64>args.getInt("amount");

    let newToken = token.mint(to, amount);
    globalStateJson = newToken.toJson();
    HostContext.emitEvent("MintComplete", globalStateJson);
    return 0;
  } else if (method == "burn") {
    let amount = <u64>args.getInt("amount");

    token.burn(amount);
    globalStateJson = token.toJson();
    HostContext.emitEvent("BurnComplete", globalStateJson);
    return 0;
  } else if (method == "balanceOf") {
    let addr = args.getString("address");
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

function callSON(method: string, state: JsonValue, args: JsonValue): i32 {
  let collectionName = state.getString("collectionName");
  let tokenId = <u64>state.getInt("tokenId");
  let metadataUri = state.getString("metadataUri");
  let owner = state.getString("owner");

  let nft = new SmartObjectNFT(collectionName, tokenId, metadataUri, owner);

  if (method == "transfer") {
    let to = args.getString("to");

    nft.transfer(to);
    globalStateJson = nft.toJson();
    HostContext.emitEvent("NFTTransferComplete", globalStateJson);
    return 0;
  } else if (method == "setMetadataUri") {
    let newUri = args.getString("uri");

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

function callNativeVault(method: string, state: JsonValue, args: JsonValue): i32 {
  let totalLockedSatoshis = <u64>state.getInt("totalLockedSatoshis");
  let vaultOwner = state.getString("vaultOwner");

  let vault = new NativeVault(vaultOwner);
  vault.totalLockedSatoshis = totalLockedSatoshis;

  if (method == "deposit") {
    vault.deposit();
    globalStateJson = vault.toJson();
    HostContext.emitEvent("DepositComplete", globalStateJson);
    return 0;
  } else if (method == "withdrawToStealth") {
    let stealthAddress = args.getString("stealthAddress");
    let amount = <u64>args.getInt("amount");

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

function callAtomicSwap(method: string, state: JsonValue, args: JsonValue): i32 {
  let maker = state.getString("maker");
  let offeredTokenId = <u64>state.getInt("offeredTokenId");
  let demandedSatoshis = <u64>state.getInt("demandedSatoshis");
  let isFilled = state.getBool("isFilled");
  let isCancelled = state.getBool("isCancelled");

  let order = new AtomicSwapOrder(maker, offeredTokenId, demandedSatoshis);
  order.isFilled = isFilled;
  order.isCancelled = isCancelled;

  if (method == "fill") {
    let taker = args.getString("taker");

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

export function get_state_size(): i32 {
  let buf = String.UTF8.encode(globalStateJson);
  return buf.byteLength;
}

export function restore_state(state_ptr: usize, state_len: i32): i32 {
  if (state_len <= 0) return 0;
  let stateStr = String.UTF8.decodeUnsafe(state_ptr, state_len, true);

  let parser = new JsonParser(stateStr);
  let parsed = parser.parse();
  if (parsed == null || parsed.type != JsonType.Object) {
    HostContext.emitEvent("Error", "Invalid restore_state JSON: " + parser.errorMsg);
    return 1;
  }

  globalStateJson = stateStr;

  // Re-detect contract type from restored state
  contractType = parsed.getString("type");
  if (contractType.length == 0) {
    if (parsed.getString("symbol").length > 0) {
      contractType = "SOT";
    }
  }
  HostContext.emitEvent("StateRestored", "type=" + contractType);
  return 0;
}
