#!/usr/bin/env node
/**
 * Core-VM CLI Wrapper
 * 
 * Calls the Rust core-vm binary and returns JSON results
 * Used by the indexer for WASM execution
 */

const { execSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const VM_BINARY = path.join(__dirname, "target/release/utxo-core-vm-cli");

function ensureBinary() {
  if (!fs.existsSync(VM_BINARY)) {
    console.error("Core-VM CLI not found. Building...");
    execSync("cargo build --release --bin utxo-core-vm-cli", {
      cwd: path.join(__dirname),
      stdio: "inherit",
    });
  }
}

function execute(command, args) {
  ensureBinary();

  const input = JSON.stringify({ command, args });
  
  try {
    const result = execSync(VM_BINARY, {
      input,
      encoding: "utf8",
      timeout: 30000,
      maxBuffer: 10 * 1024 * 1024,
    });
    
    return JSON.parse(result);
  } catch (err) {
    throw new Error(`VM execution failed: ${err.message}`);
  }
}

function deploy(wasmHex, caller, seal, satoshis, initArgs) {
  return execute("deploy", {
    wasm_hex: wasmHex,
    caller,
    seal_txid: seal.split(":")[0],
    seal_vout: parseInt(seal.split(":")[1]) || 0,
    satoshis,
    init_args_hex: initArgs || "",
  });
}

function executeContract(wasmHex, stateHex, caller, method, args) {
  return execute("execute", {
    wasm_hex: wasmHex,
    state_hex: stateHex,
    caller,
    method,
    args_hex: args || "",
  });
}

function calculateCodeHash(wasmHex) {
  return execute("code_hash", { wasm_hex: wasmHex });
}

module.exports = { deploy, executeContract, calculateCodeHash, execute };

if (require.main === module) {
  const [command, ...args] = process.argv.slice(2);
  
  const commands = {
    deploy: () => {
      const [wasmFile, caller, seal, satoshis, initArgs] = args;
      const wasmHex = fs.readFileSync(wasmFile, "hex");
      const result = deploy(wasmHex, caller, seal, parseInt(satoshis), initArgs);
      console.log(JSON.stringify(result, null, 2));
    },
    execute: () => {
      const [wasmFile, stateHex, caller, method, argsFile] = args;
      const wasmHex = fs.readFileSync(wasmFile, "hex");
      const methodArgs = argsFile ? fs.readFileSync(argsFile, "hex") : "";
      const result = executeContract(wasmHex, stateHex, caller, method, methodArgs);
      console.log(JSON.stringify(result, null, 2));
    },
    code_hash: () => {
      const [wasmFile] = args;
      const wasmHex = fs.readFileSync(wasmFile, "hex");
      const result = calculateCodeHash(wasmHex);
      console.log(JSON.stringify(result, null, 2));
    },
  };
  
  if (commands[command]) {
    commands[command]();
  } else {
    console.error("Usage: node cli.js <deploy|execute|code_hash> [args...]");
    process.exit(1);
  }
}
