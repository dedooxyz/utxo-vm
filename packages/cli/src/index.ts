#!/usr/bin/env node
import { Command } from "commander";
import { handleCompile } from "./commands/compile.js";
import { handleDeploy } from "./commands/deploy.js";
import { handleCall } from "./commands/call.js";
import { handleInspect } from "./commands/inspect.js";

const program = new Command();

program
  .name("utxo-vm")
  .description("Universal UTXO Virtual Machine Developer CLI")
  .version("0.1.0");

program
  .command("compile <file>")
  .description("Compile AssemblyScript contract to WASM & WAT")
  .option("-o, --out-dir <dir>", "Output directory for compiled artifacts", "./build")
  .action((file, options) => handleCompile(file, options));

program
  .command("deploy <wasmFile>")
  .option("-c, --chain <chain>", "Target blockchain (BTC, LTC, DOGE, JKC, BEL, BCH)", "BTC")
  .option("-n, --node-url <url>", "Node API URL", "http://127.0.0.1:9773")
  .option("-s, --satoshis <amount>", "Satoshis to attach to initial UTXO seal", "10000")
  .description("Deploy WASM contract to any UTXO blockchain")
  .action((wasmFile, options) => handleDeploy(wasmFile, options));

program
  .command("call <targetSeal> <method> [args...]")
  .option("-c, --chain <chain>", "Target blockchain (BTC, LTC, DOGE, JKC, BEL, BCH)", "BTC")
  .option("-n, --node-url <url>", "Node API URL", "http://127.0.0.1:9773")
  .description("Execute a state transition method call on a smart object seal")
  .action((targetSeal, method, args, options) => handleCall(targetSeal, method, args, options));

program
  .command("inspect <objectIdOrSeal>")
  .option("-c, --chain <chain>", "Target blockchain (BTC, LTC, DOGE, JKC, BEL, BCH)", "BTC")
  .option("-n, --node-url <url>", "Node API URL", "http://127.0.0.1:9773")
  .description("Inspect on-chain state, current seal, and history of a smart object")
  .action((objectIdOrSeal, options) => handleInspect(objectIdOrSeal, options));

program.parse();
