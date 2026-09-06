import crypto from "crypto";

export interface SmartObjectRecord {
  objectId: string;
  codeHash: string;
  seal: string; // txid:vout
  satoshis: string;
  owner: string;
  stateData: any;
  updatedAtBlock: number;
}

export interface StateTransitionRecord {
  txid: string;
  consumedSeal: string;
  newSeal: string;
  method: string;
  events: Array<{ topic: string; data: string }>;
  blockHeight: number;
  timestamp: number;
}

export interface BlockHeaderRecord {
  chain: string;
  blockHeight: number;
  blockHash: string;
  prevHash?: string;
  stateRoot?: string;
  createdAt?: number;
}

export interface MempoolTransitionRecord {
  txid: string;
  chain: string;
  objectId: string;
  consumedSeal: string;
  newSeal: string;
  method: string;
  events: Array<{ topic: string; data: string }>;
  receivedAt: number;
}

export interface IStateDB {
  saveObject(record: SmartObjectRecord, chain?: string): Promise<void>;
  getObject(objectId: string): Promise<SmartObjectRecord | undefined>;
  getObjectBySeal(seal: string): Promise<SmartObjectRecord | undefined>;
  getObjectsByOwner(owner: string): Promise<SmartObjectRecord[]>;
  getAllObjects(): Promise<SmartObjectRecord[]>;
  recordTransition(objectId: string, transition: StateTransitionRecord): Promise<void>;
  getTransitions(objectId: string): Promise<StateTransitionRecord[]>;
  getHistory(objectId: string): Promise<StateTransitionRecord[]>;
  recordBlockHeader(chain: string, height: number, hash: string, prevHash?: string, stateRoot?: string): Promise<void>;
  getBlockHash(chain: string, height: number): Promise<string | undefined>;
  getBlockHeader(chain: string, height: number): Promise<BlockHeaderRecord | undefined>;
  rollbackToBlock(chain: string, targetHeight: number): Promise<{ rolledBackObjects: number; rolledBackTransitions: number }>;
  computeAndSaveStateRoot(chain: string, height: number): Promise<string>;
  getStateRoot(chain: string, height: number): Promise<string | undefined>;
  recordMempoolTransition?(transition: MempoolTransitionRecord): Promise<void>;
  getMempoolTransitions?(chain: string): Promise<MempoolTransitionRecord[]>;
  removeMempoolTransition?(txid: string): Promise<void>;
}

export class MemoryStateDB implements IStateDB {
  private objects: Map<string, SmartObjectRecord> = new Map();
  private sealIndex: Map<string, string> = new Map(); // seal -> objectId
  private history: Map<string, StateTransitionRecord[]> = new Map(); // objectId -> transitions
  private blockHeaders: Map<string, Map<number, BlockHeaderRecord>> = new Map(); // chain -> height -> header
  private undoLogs: Array<{
    chain: string;
    height: number;
    objectId: string;
    action: "CREATE" | "UPDATE";
    prevRecord?: SmartObjectRecord;
  }> = [];
  private mempool: Map<string, MempoolTransitionRecord> = new Map(); // txid -> record

  async saveObject(record: SmartObjectRecord, chain: string = "JKC"): Promise<void> {
    const existing = this.objects.get(record.objectId);
    if (existing) {
      this.undoLogs.push({
        chain,
        height: record.updatedAtBlock,
        objectId: record.objectId,
        action: "UPDATE",
        prevRecord: { ...existing },
      });
      this.sealIndex.delete(existing.seal);
    } else {
      this.undoLogs.push({
        chain,
        height: record.updatedAtBlock,
        objectId: record.objectId,
        action: "CREATE",
      });
    }

    this.objects.set(record.objectId, record);
    this.sealIndex.set(record.seal, record.objectId);
  }

  async getObject(objectId: string): Promise<SmartObjectRecord | undefined> {
    return this.objects.get(objectId);
  }

  async getObjectBySeal(seal: string): Promise<SmartObjectRecord | undefined> {
    const objectId = this.sealIndex.get(seal);
    if (!objectId) return undefined;
    return this.objects.get(objectId);
  }

  async getObjectsByOwner(owner: string): Promise<SmartObjectRecord[]> {
    const results: SmartObjectRecord[] = [];
    for (const obj of this.objects.values()) {
      if (obj.owner === owner) {
        results.push(obj);
      }
    }
    return results;
  }

  async getAllObjects(): Promise<SmartObjectRecord[]> {
    return Array.from(this.objects.values());
  }

  async recordTransition(objectId: string, transition: StateTransitionRecord): Promise<void> {
    // Invalidate old seal in seal index
    this.sealIndex.delete(transition.consumedSeal);
    this.sealIndex.set(transition.newSeal, objectId);

    const hist = this.history.get(objectId) || [];
    hist.push(transition);
    this.history.set(objectId, hist);
  }

  async getTransitions(objectId: string): Promise<StateTransitionRecord[]> {
    return this.getHistory(objectId);
  }

  async getHistory(objectId: string): Promise<StateTransitionRecord[]> {
    return this.history.get(objectId) || [];
  }

  async recordBlockHeader(
    chain: string,
    height: number,
    hash: string,
    prevHash?: string,
    stateRoot?: string
  ): Promise<void> {
    if (!this.blockHeaders.has(chain)) {
      this.blockHeaders.set(chain, new Map());
    }
    this.blockHeaders.get(chain)!.set(height, {
      chain,
      blockHeight: height,
      blockHash: hash,
      prevHash,
      stateRoot,
      createdAt: Date.now(),
    });
  }

  async getBlockHash(chain: string, height: number): Promise<string | undefined> {
    const header = this.blockHeaders.get(chain)?.get(height);
    return header?.blockHash;
  }

  async getBlockHeader(chain: string, height: number): Promise<BlockHeaderRecord | undefined> {
    return this.blockHeaders.get(chain)?.get(height);
  }

  async computeAndSaveStateRoot(chain: string, height: number): Promise<string> {
    const sorted = Array.from(this.objects.values()).sort((a, b) => a.objectId.localeCompare(b.objectId));
    const hasher = crypto.createHash("sha256");
    for (const obj of sorted) {
      hasher.update(`${obj.objectId}:${obj.codeHash}:${obj.seal}:${obj.satoshis}:${obj.owner}:${JSON.stringify(obj.stateData)};`);
    }
    const stateRoot = hasher.digest("hex");
    const header = this.blockHeaders.get(chain)?.get(height);
    if (header) {
      header.stateRoot = stateRoot;
    }
    return stateRoot;
  }

  async getStateRoot(chain: string, height: number): Promise<string | undefined> {
    return this.blockHeaders.get(chain)?.get(height)?.stateRoot;
  }

  async rollbackToBlock(
    chain: string,
    targetHeight: number
  ): Promise<{ rolledBackObjects: number; rolledBackTransitions: number }> {
    let rolledBackObjects = 0;
    let rolledBackTransitions = 0;

    // Roll back undo logs in reverse order
    for (let i = this.undoLogs.length - 1; i >= 0; i--) {
      const entry = this.undoLogs[i];
      if (entry.chain === chain && entry.height > targetHeight) {
        if (entry.action === "CREATE") {
          const current = this.objects.get(entry.objectId);
          if (current) {
            this.sealIndex.delete(current.seal);
            this.objects.delete(entry.objectId);
          }
        } else if (entry.action === "UPDATE" && entry.prevRecord) {
          const current = this.objects.get(entry.objectId);
          if (current) {
            this.sealIndex.delete(current.seal);
          }
          this.objects.set(entry.objectId, entry.prevRecord);
          this.sealIndex.set(entry.prevRecord.seal, entry.objectId);
        }
        this.undoLogs.splice(i, 1);
        rolledBackObjects++;
      }
    }

    // Roll back transitions
    for (const [objectId, transitions] of this.history.entries()) {
      const filtered = transitions.filter(t => {
        if (t.blockHeight > targetHeight) {
          rolledBackTransitions++;
          return false;
        }
        return true;
      });
      this.history.set(objectId, filtered);
    }

    // Roll back block headers
    const headers = this.blockHeaders.get(chain);
    if (headers) {
      for (const h of Array.from(headers.keys())) {
        if (h > targetHeight) {
          headers.delete(h);
        }
      }
    }

    return { rolledBackObjects, rolledBackTransitions };
  }

  async recordMempoolTransition(transition: MempoolTransitionRecord): Promise<void> {
    this.mempool.set(transition.txid, transition);
  }

  async getMempoolTransitions(chain: string): Promise<MempoolTransitionRecord[]> {
    return Array.from(this.mempool.values()).filter(m => m.chain === chain);
  }

  async removeMempoolTransition(txid: string): Promise<void> {
    this.mempool.delete(txid);
  }
}
