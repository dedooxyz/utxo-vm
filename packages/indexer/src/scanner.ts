import { execSync } from "child_process";
import path from "path";
import { MemoryStateDB, SmartObjectRecord, StateTransitionRecord, IStateDB } from "./state_db";
import { getChainConfig, isOpcodesSupported, isSettlementLayer, getSupportedChildChains } from "./chains";

const VM_BINARY = path.join(__dirname, "../../../target/release/utxo-core-vm-cli");

function executeVm(command: string, args: Record<string, any>): any {
  const input = JSON.stringify({ command, args });
  try {
    const result = execSync(VM_BINARY, {
      input,
      encoding: "utf8",
      timeout: 30000,
      maxBuffer: 10 * 1024 * 1024,
    });
    return JSON.parse(result);
  } catch (err: any) {
    console.error(`[Scanner] VM execution failed: ${err.message}`);
    return { success: false, error: err.message };
  }
}

function calculateCodeHash(wasmHex: string): string {
  const result = executeVm("code_hash", { wasm_hex: wasmHex });
  return result.success ? result.result.code_hash : "unknown";
}

// --- Opcode Detection ---
export interface OpcodeResult {
  opcode: number;
  data: Buffer;
  isOpcode: boolean;
}

/**
 * Detect if a scriptPubKey contains UTXO-VM opcodes
 */
export function detectOpcode(scriptPubKeyHex: string): OpcodeResult | null {
  const buf = Buffer.from(scriptPubKeyHex, "hex");
  const chainConfig = getChainConfig(process.env.CHAIN || "JKC_TESTNET");

  if (!chainConfig.opcodesSupported || !chainConfig.opcodes.OP_CREATE_SMART_OBJECT) {
    return null;
  }

  // Check for opcode prefix (0xc0-0xc4 range)
  if (buf.length >= 2 && buf[0] >= 0xc0 && buf[0] <= 0xc4) {
    const opcode = buf[0];
    const data = buf.slice(1);
    return { opcode, data, isOpcode: true };
  }

  return null;
}

/**
 * Process an opcode transaction
 */
export async function processOpcode(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  const chainConfig = getChainConfig(chain);

  switch (opcodeResult.opcode) {
    case chainConfig.opcodes.OP_CREATE_SMART_OBJECT:
      return await processCreateSmartObject(opcodeResult, tx, db, chain, blockHeight);

    case chainConfig.opcodes.OP_CALL_SMART_OBJECT:
      return await processCallSmartObject(opcodeResult, tx, db, chain, blockHeight);

    case chainConfig.opcodes.OP_UPDATE_STATE:
      return await processUpdateState(opcodeResult, tx, db, chain, blockHeight);

    case chainConfig.opcodes.OP_SETTLE_CROSS_CHAIN:
      return await processSettleCrossChain(opcodeResult, tx, db, chain, blockHeight);

    case chainConfig.opcodes.OP_VERIFY_SEAL:
      return await processVerifySeal(opcodeResult, tx, db, chain, blockHeight);

    default:
      console.warn(`[Scanner] Unknown opcode: ${opcodeResult.opcode}`);
      return false;
  }
}

async function processCreateSmartObject(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  try {
    const data = JSON.parse(opcodeResult.data.toString("utf8"));
    const seal = `${tx.txid}:0`;
    const txid = tx.txid;
    const objectId = `obj_${txid.slice(0, 16)}`;

    const record: SmartObjectRecord = {
      objectId,
      codeHash: data.codeHash || calculateCodeHash(data.wasmHex || ""),
      seal,
      satoshis: data.satoshis || "0",
      owner: data.owner || tx.vin?.[0]?.prevout?.scriptpubkey_address || "unknown",
      stateData: data.initialState || {},
      updatedAtBlock: blockHeight,
    };

    await db.saveObject(record, chain);
    console.log(`[Scanner] Opcode: Created Smart Object ${objectId} at Seal ${seal}`);
    return true;
  } catch (err: any) {
    console.error(`[Scanner] OP_CREATE_SMART_OBJECT failed: ${err.message}`);
    return false;
  }
}

async function processCallSmartObject(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  try {
    const data = JSON.parse(opcodeResult.data.toString("utf8"));
    const targetSeal = data.targetSeal;
    const existingObj = await db.getObjectBySeal(targetSeal);

    if (!existingObj) {
      console.warn(`[Scanner] OP_CALL_SMART_OBJECT: Target seal ${targetSeal} not found`);
      return false;
    }

    const newSeal = `${tx.txid}:0`;
    const method = data.method || "call";

    const record: SmartObjectRecord = {
      objectId: existingObj.objectId,
      codeHash: existingObj.codeHash,
      seal: newSeal,
      satoshis: existingObj.satoshis,
      owner: data.caller || existingObj.owner,
      stateData: { ...existingObj.stateData, lastMethod: method, lastCaller: data.caller },
      updatedAtBlock: blockHeight,
    };

    await db.saveObject(record, chain);
    console.log(`[Scanner] Opcode: Called Smart Object ${existingObj.objectId} (${method})`);
    return true;
  } catch (err: any) {
    console.error(`[Scanner] OP_CALL_SMART_OBJECT failed: ${err.message}`);
    return false;
  }
}

async function processUpdateState(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  try {
    const data = JSON.parse(opcodeResult.data.toString("utf8"));
    const targetSeal = data.targetSeal;
    const existingObj = await db.getObjectBySeal(targetSeal);

    if (!existingObj) {
      console.warn(`[Scanner] OP_UPDATE_STATE: Target seal ${targetSeal} not found`);
      return false;
    }

    const newSeal = `${tx.txid}:0`;
    const newState = { ...existingObj.stateData, ...data.newState };

    const record: SmartObjectRecord = {
      objectId: existingObj.objectId,
      codeHash: existingObj.codeHash,
      seal: newSeal,
      satoshis: existingObj.satoshis,
      owner: existingObj.owner,
      stateData: newState,
      updatedAtBlock: blockHeight,
    };

    await db.saveObject(record, chain);
    console.log(`[Scanner] Opcode: Updated state for ${existingObj.objectId}`);
    return true;
  } catch (err: any) {
    console.error(`[Scanner] OP_UPDATE_STATE failed: ${err.message}`);
    return false;
  }
}

async function processSettleCrossChain(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  try {
    const data = JSON.parse(opcodeResult.data.toString("utf8"));
    const { sourceChain, sourceTxHash, stateRoot, proof } = data;

    console.log(`[Scanner] Opcode: Cross-chain settlement from ${sourceChain}`);
    console.log(`[Scanner] Source TX: ${sourceTxHash}`);
    console.log(`[Scanner] State Root: ${stateRoot}`);

    // Store settlement record
    const settlementId = `settlement_${tx.txid.slice(0, 16)}`;
    const seal = `${tx.txid}:0`;

    const record: SmartObjectRecord = {
      objectId: settlementId,
      codeHash: "settlement",
      seal,
      satoshis: "0",
      owner: sourceChain,
      stateData: {
        type: "cross_chain_settlement",
        sourceChain,
        sourceTxHash,
        stateRoot,
        proof,
        settledAtBlock: blockHeight,
      },
      updatedAtBlock: blockHeight,
    };

    await db.saveObject(record, chain);
    console.log(`[Scanner] Settlement ${settlementId} recorded`);
    return true;
  } catch (err: any) {
    console.error(`[Scanner] OP_SETTLE_CROSS_CHAIN failed: ${err.message}`);
    return false;
  }
}

async function processVerifySeal(
  opcodeResult: OpcodeResult,
  tx: any,
  db: IStateDB,
  chain: string,
  blockHeight: number
): Promise<boolean> {
  try {
    const data = JSON.parse(opcodeResult.data.toString("utf8"));
    const { seal, expectedOwner } = data;

    console.log(`[Scanner] Opcode: Verifying seal ${seal}`);

    const sealObj = await db.getObjectBySeal(seal);
    const isValid = !!(sealObj && sealObj.owner === expectedOwner);

    console.log(`[Scanner] Seal ${seal} valid: ${isValid}`);
    return isValid;
  } catch (err: any) {
    console.error(`[Scanner] OP_VERIFY_SEAL failed: ${err.message}`);
    return false;
  }
}

export interface TransactionInput {
  txid: string;
  vout: number;
  signature?: string;
}

export interface TransactionOutput {
  vout: number;
  satoshis: bigint;
  scriptPubKeyHex: string;
}

export interface BlockTransaction {
  txid: string;
  inputs: TransactionInput[];
  outputs: TransactionOutput[];
  envelope?: {
    protocol: string;
    version: number;
    contentType: string;
    payload: any;
    metadata?: Record<string, any>;
    status?: "confirmed" | "mempool";
  };
}

// --- Envelope Parser ---
export function parseEnvelope(scriptPubKeyHex: string): {
  protocol: string;
  version: number;
  contentType: string;
  payload: number[];
} | null {
  const buf = Buffer.from(scriptPubKeyHex, "hex");
  if (buf[0] !== 0x6a) return null; // OP_RETURN

  let offset = 1;
  const first = buf[offset++];
  let envelopeLen: number;
  if (first === 0x4c) {
    envelopeLen = buf[offset++];
  } else if (first === 0x4d) {
    envelopeLen = buf[offset] | (buf[offset + 1] << 8);
    offset += 2;
  } else if (first === 0x4e) {
    envelopeLen = buf[offset] | (buf[offset + 1] << 8) | (buf[offset + 2] << 16) | (buf[offset + 3] << 24);
    offset += 4;
  } else {
    envelopeLen = first;
  }

  const envelope = buf.slice(offset, offset + envelopeLen);
  if (envelope[0] !== 0x00 || envelope[1] !== 0x63) return null; // OP_FALSE OP_IF

  let envOff = 2;
  function readPush(): Buffer | null {
    if (envOff >= envelope.length) return null;
    const b = envelope[envOff++];
    if (b < 0x4c) {
      const d = envelope.slice(envOff, envOff + b);
      envOff += b;
      return d;
    }
    if (b === 0x4c) {
      const len = envelope[envOff++];
      const d = envelope.slice(envOff, envOff + len);
      envOff += len;
      return d;
    }
    if (b === 0x4d) {
      const len = envelope[envOff] | (envelope[envOff + 1] << 8);
      envOff += 2;
      const d = envelope.slice(envOff, envOff + len);
      envOff += len;
      return d;
    }
    if (b === 0x4e) {
      const len = envelope[envOff] | (envelope[envOff + 1] << 8) | (envelope[envOff + 2] << 16) | (envelope[envOff + 3] << 24);
      envOff += 4;
      const d = envelope.slice(envOff, envOff + len);
      envOff += len;
      return d;
    }
    return null;
  }

  const protocol = readPush()?.toString("utf8");
  if (protocol !== "utxovm") return null;

  const versionBuf = readPush();
  const version = versionBuf ? versionBuf[0] : 0;

  const contentType = readPush()?.toString("utf8") || "unknown";
  const payloadPush = readPush();
  const payload = payloadPush || Buffer.alloc(0);

  if (envelope[envOff] !== 0x68) return null; // OP_ENDIF

  return { protocol, version, contentType, payload: Array.from(payload) };
}

// --- Electrs Client ---
export class ElectrsClient {
  constructor(private baseUrl: string) {}

  private async get(path: string): Promise<any> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`Electrs GET ${path}: ${res.status}`);
    return res;
  }

  async getJson(path: string): Promise<any> {
    return (await this.get(path)).json();
  }

  async getText(path: string): Promise<string> {
    const res = await this.get(path);
    return (await res.text()).trim();
  }

  async tipHeight(): Promise<number> {
    return parseInt(await this.getText("/blocks/tip/height"), 10);
  }

  async blockTxs(blockHash: string, startIdx = 0): Promise<any[]> {
    return this.getJson(`/block/${blockHash}/txs/${startIdx}`);
  }

  async blockHash(height: number): Promise<string> {
    return this.getText(`/block-height/${height}`);
  }

  async txInfo(txid: string): Promise<any> {
    return this.getJson(`/tx/${txid}`);
  }

  async mempoolRecent(): Promise<any[]> {
    try {
      return await this.getJson("/mempool/recent");
    } catch {
      return [];
    }
  }

  async txStatus(txid: string): Promise<string> {
    try {
      const info = await this.txInfo(txid);
      return info.status?.confirmed ? "confirmed" : "mempool";
    } catch {
      return "unknown";
    }
  }
}

// --- Scanner with Electrs ---
export class ChainBlockScanner {
  private db: IStateDB;
  public chain: string;
  public currentBlockHeight: number = 0;
  private isRunning: boolean = false;
  private electrs: ElectrsClient;
  private mempoolSeen: Set<string> = new Set();

  constructor(db: IStateDB, chain: string = "BTC", electrsUrl?: string) {
    this.db = db;
    this.chain = chain;
    this.electrs = new ElectrsClient(electrsUrl || "https://jkc-testnet-api.s3na.xyz");
  }

  async start(): Promise<void> {
    this.isRunning = true;
    console.log(`[Scanner] Started scanning ${this.chain} blockchain for utxovm envelopes...`);
  }

  async stop(): Promise<void> {
    this.isRunning = false;
    console.log(`[Scanner] Stopped scanning ${this.chain}.`);
  }

  /**
   * Sync from electrs: fetch recent blocks and index all utxovm envelopes and opcodes
   */
  async syncFromElectrs(sinceBlock?: number): Promise<{ indexed: number; blocks: number; opcodes: number }> {
    const tipHeight = await this.electrs.tipHeight();
    let startBlock = sinceBlock || (this.currentBlockHeight > 0 ? this.currentBlockHeight + 1 : Math.max(0, tipHeight - 10));
    let indexed = 0;
    let opcodes = 0;

    // Check for chain reorg at current tip
    if (this.currentBlockHeight > 0) {
      const storedTipHash = await this.db.getBlockHash(this.chain, this.currentBlockHeight);
      let canonTipHash: string | null = null;
      try {
        canonTipHash = await this.electrs.blockHash(this.currentBlockHeight);
      } catch {}

      if (storedTipHash && canonTipHash && storedTipHash !== canonTipHash) {
        console.warn(`[Scanner] Reorg detected at height ${this.currentBlockHeight}! Stored: ${storedTipHash}, Canon: ${canonTipHash}`);
        let ancestorHeight = this.currentBlockHeight - 1;
        let depth = 1;
        while (ancestorHeight > 0 && depth < 100) {
          const storedH = await this.db.getBlockHash(this.chain, ancestorHeight);
          let canonH: string | null = null;
          try {
            canonH = await this.electrs.blockHash(ancestorHeight);
          } catch {}
          if (storedH && canonH && storedH === canonH) {
            break;
          }
          ancestorHeight--;
          depth++;
        }
        console.warn(`[Scanner] Rolling back to common ancestor at height ${ancestorHeight}...`);
        await this.db.rollbackToBlock(this.chain, ancestorHeight);
        this.currentBlockHeight = ancestorHeight;
        startBlock = ancestorHeight + 1;
      }
    }

    console.log(`[Scanner] Syncing blocks ${startBlock} → ${tipHeight}`);

    for (let height = startBlock; height <= tipHeight; height++) {
      const blockHash = await this.electrs.blockHash(height);
      await this.db.recordBlockHeader(this.chain, height, blockHash);
      const txs = await this.electrs.blockTxs(blockHash);

      for (const tx of txs) {
        this.mempoolSeen.delete(tx.txid);
        if (this.db.removeMempoolTransition) {
          await this.db.removeMempoolTransition(tx.txid);
        }

        // Check for opcodes first (for chains that support them)
        const opcodeResult = detectOpcode(tx.vout?.[0]?.scriptpubkey || "");
        if (opcodeResult?.isOpcode) {
          const processed = await processOpcode(opcodeResult, tx, this.db, this.chain, height);
          if (processed) {
            opcodes++;
            continue; // Skip envelope processing for opcode transactions
          }
        }

        // Fallback to envelope processing
        const opReturns = (tx.vout || []).filter((o: any) => o.scriptpubkey_type === "op_return");

        for (const opReturn of opReturns) {
          const envelope = parseEnvelope(opReturn.scriptpubkey);
          if (!envelope) continue;

          let metadata: Record<string, any> = {};
          if (envelope.contentType === "application/json") {
            try {
              metadata = JSON.parse(Buffer.from(envelope.payload).toString("utf8"));
            } catch {}
          }

          const blockTx: BlockTransaction = {
            txid: tx.txid,
            inputs: (tx.vin || []).map((v: any) => ({
              txid: v.prevout?.txid || "",
              vout: v.prevout?.vout || 0,
            })),
            outputs: (tx.vout || []).map((o: any, idx: number) => ({
              vout: idx,
              satoshis: BigInt(o.value),
              scriptPubKeyHex: o.scriptpubkey,
            })),
            envelope: {
              protocol: envelope.protocol,
              version: envelope.version,
              contentType: envelope.contentType,
              payload: envelope.payload,
              metadata,
            },
          };

          await this.processTransaction(blockTx, height);
          indexed++;
        }
      }

      await this.db.computeAndSaveStateRoot(this.chain, height);
      this.currentBlockHeight = height;
    }

    console.log(`[Scanner] Synced to block ${tipHeight}, indexed ${indexed} envelopes, ${opcodes} opcodes`);
    return { indexed, blocks: tipHeight - startBlock + 1, opcodes };
  }

  /**
   * Scan mempool for unconfirmed transactions with utxovm envelopes or opcodes
   */
  async scanMempool(): Promise<{ indexed: number; opcodes: number; txids: string[] }> {
    const recent = await this.electrs.mempoolRecent();
    let indexed = 0;
    let opcodes = 0;
    const txids: string[] = [];

    for (const tx of recent) {
      if (this.mempoolSeen.has(tx.txid)) continue;
      this.mempoolSeen.add(tx.txid);

      try {
        const txInfo = await this.electrs.txInfo(tx.txid);

        // Check for opcodes first
        const opcodeResult = detectOpcode(txInfo.vout?.[0]?.scriptpubkey || "");
        if (opcodeResult?.isOpcode) {
          const processed = await processOpcode(opcodeResult, txInfo, this.db, this.chain, this.currentBlockHeight);
          if (processed) {
            opcodes++;
            txids.push(tx.txid);
            continue;
          }
        }

        // Fallback to envelope processing
        const opReturns = (txInfo.vout || []).filter((o: any) => o.scriptpubkey_type === "op_return");

        for (const opReturn of opReturns) {
          const envelope = parseEnvelope(opReturn.scriptpubkey);
          if (!envelope) continue;

          let metadata: Record<string, any> = {};
          if (envelope.contentType === "application/json") {
            try {
              metadata = JSON.parse(Buffer.from(envelope.payload).toString("utf8"));
            } catch {}
          }

          const mempoolTx: BlockTransaction = {
            txid: tx.txid,
            inputs: (txInfo.vin || []).map((v: any) => ({
              txid: v.prevout?.txid || "",
              vout: v.prevout?.vout || 0,
            })),
            outputs: (txInfo.vout || []).map((o: any, idx: number) => ({
              vout: idx,
              satoshis: BigInt(o.value),
              scriptPubKeyHex: o.scriptpubkey,
            })),
            envelope: {
              protocol: envelope.protocol,
              version: envelope.version,
              contentType: envelope.contentType,
              payload: envelope.payload,
              metadata,
              status: "mempool",
            },
          };

          await this.processTransaction(mempoolTx, this.currentBlockHeight);
          indexed++;
          txids.push(tx.txid);
        }
      } catch {}
    }

    return { indexed, opcodes, txids };
  }

  /**
   * Process a confirmed block containing transactions
   */
  async processBlock(blockHeight: number, txs: BlockTransaction[], blockHash?: string): Promise<void> {
    this.currentBlockHeight = blockHeight;
    if (blockHash) {
      await this.db.recordBlockHeader(this.chain, blockHeight, blockHash);
    }
    for (const tx of txs) {
      if (this.db.removeMempoolTransition) {
        await this.db.removeMempoolTransition(tx.txid);
      }
      await this.processTransaction(tx, blockHeight);
    }
    await this.db.computeAndSaveStateRoot(this.chain, blockHeight);
  }

  /**
   * Process a single transaction and update Single-Use Seals
   */
  async processTransaction(tx: BlockTransaction, blockHeight: number = this.currentBlockHeight): Promise<void> {
    if (!tx.envelope || tx.envelope.protocol !== "utxovm") {
      return;
    }

    const { contentType, metadata } = tx.envelope;

    // Case 1: Deploy Contract -> Create initial Smart Object Seal
    if (contentType === "application/wasm") {
      const objectId = "obj_" + tx.txid.slice(0, 16);
      const seal = `${tx.txid}:0`;
      const satoshis = tx.outputs[0]?.satoshis?.toString() || "1000";
      const owner = metadata?.owner || "03" + tx.txid.slice(0, 64);

      // Execute WASM contract via core-vm
      const wasmHex = Buffer.from(tx.envelope.payload).toString("hex");
      const initArgs = metadata?.init ? JSON.stringify(metadata.init) : "";
      const initArgsHex = Buffer.from(initArgs).toString("hex");
      
      const vmResult = executeVm("deploy", {
        wasm_hex: wasmHex,
        caller: owner,
        seal_txid: tx.txid,
        seal_vout: 0,
        satoshis: parseInt(satoshis),
        init_args_hex: initArgsHex,
      });

      const codeHash = vmResult.success ? vmResult.result.code_hash : calculateCodeHash(wasmHex);
      const stateData = vmResult.success && vmResult.result.updated_state_hex
        ? JSON.parse(Buffer.from(vmResult.result.updated_state_hex, "hex").toString("utf8") || "{}")
        : metadata?.initialState || { owner, initialized: true };

      const record: SmartObjectRecord = {
        objectId,
        codeHash,
        seal,
        satoshis,
        owner,
        stateData: {
          ...stateData,
          vmResult: vmResult.success ? {
            gasConsumed: vmResult.result.gas_consumed,
            returnCode: vmResult.result.return_code,
          } : null,
        },
        updatedAtBlock: blockHeight,
      };

      await this.db.saveObject(record, this.chain);
      console.log(`[Scanner] Deployed Smart Object ${objectId} at Seal ${seal} (VM: ${vmResult.success ? "ok" : "fallback"})`);
    }

    // Case 2: Raw inscription (binary payload)
    else if (contentType === "application/octet-stream") {
      const objectId = "obj_" + tx.txid.slice(0, 16);
      const seal = `${tx.txid}:0`;
      const record: SmartObjectRecord = {
        objectId,
        codeHash: "raw_" + tx.txid.slice(0, 16),
        seal,
        satoshis: tx.outputs[0]?.satoshis?.toString() || "0",
        owner: "03" + tx.txid.slice(0, 64),
        stateData: { type: "raw_inscription", payloadLen: tx.envelope.payload.length },
        updatedAtBlock: blockHeight,
      };
      await this.db.saveObject(record, this.chain);
      console.log(`[Scanner] Raw Inscription ${objectId} at Seal ${seal}`);
    }

    // Case 3: Method Call with targetSeal -> Spend Input Seal, Create Output Seal
    else if (contentType === "application/json" && metadata?.targetSeal) {
      const targetSeal = metadata.targetSeal;
      const existingObj = await this.db.getObjectBySeal(targetSeal);

      if (!existingObj) {
        console.warn(`[Scanner] Warning: Seal ${targetSeal} not found or already spent.`);
        return;
      }

      const method = metadata.method || "call";
      const newSeal = `${tx.txid}:0`;
      const newSatoshis = tx.outputs[0]?.satoshis?.toString() || existingObj.satoshis;

      // Execute WASM contract if we have the code
      let updatedState = { ...existingObj.stateData, lastMethod: method, lastCaller: metadata.caller || existingObj.owner };
      let vmResult: any = null;

      if (existingObj.codeHash && existingObj.codeHash.startsWith("0x")) {
        // We have a real code hash, try to execute via core-vm
        const args = metadata.args ? JSON.stringify(metadata.args) : "";
        const argsHex = Buffer.from(args).toString("hex");
        
        vmResult = executeVm("execute", {
          wasm_hex: existingObj.codeHash, // This is actually the WASM hex stored in codeHash
          state_hex: Buffer.from(JSON.stringify(existingObj.stateData)).toString("hex"),
          caller: metadata.caller || existingObj.owner,
          method,
          args_hex: argsHex,
          seal_txid: tx.txid,
          seal_vout: 0,
        });

        if (vmResult.success && vmResult.result.updated_state_hex) {
          try {
            const newState = JSON.parse(Buffer.from(vmResult.result.updated_state_hex, "hex").toString("utf8"));
            updatedState = { ...updatedState, ...newState };
          } catch {}
        }
      } else {
        // Fallback: simple state update
        if (metadata.args?.to) {
          updatedState.owner = metadata.args.to;
        }
      }

      const updatedRecord: SmartObjectRecord = {
        ...existingObj,
        seal: newSeal,
        satoshis: newSatoshis,
        owner: updatedState.owner || existingObj.owner,
        stateData: {
          ...updatedState,
          vmResult: vmResult?.success ? {
            gasConsumed: vmResult.result.gas_consumed,
            returnCode: vmResult.result.return_code,
          } : null,
        },
        updatedAtBlock: blockHeight,
      };

      await this.db.saveObject(updatedRecord, this.chain);

      const transition: StateTransitionRecord = {
        txid: tx.txid,
        consumedSeal: targetSeal,
        newSeal,
        method,
        events: [
          { topic: "StateTransition", data: `Invoked ${method} by ${metadata.caller || "caller"}` },
        ],
        blockHeight,
        timestamp: Date.now(),
      };

      await this.db.recordTransition(existingObj.objectId, transition);
      console.log(`[Scanner] State Transition for ${existingObj.objectId}: ${targetSeal} -> ${newSeal}`);
    }

    // Case 4: Standalone JSON call (no targetSeal)
    else if (contentType === "application/json") {
      const objectId = "obj_" + tx.txid.slice(0, 16);
      const seal = `${tx.txid}:0`;
      const record: SmartObjectRecord = {
        objectId,
        codeHash: "call_" + tx.txid.slice(0, 16),
        seal,
        satoshis: tx.outputs[0]?.satoshis?.toString() || "0",
        owner: metadata?.caller || "03" + tx.txid.slice(0, 16),
        stateData: metadata || {},
        updatedAtBlock: blockHeight,
      };
      await this.db.saveObject(record, this.chain);
      console.log(`[Scanner] Call ${objectId} at Seal ${seal}`);
    }
  }
}
