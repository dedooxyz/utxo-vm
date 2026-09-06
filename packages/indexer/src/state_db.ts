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

export class MemoryStateDB {
  private objects: Map<string, SmartObjectRecord> = new Map();
  private sealIndex: Map<string, string> = new Map(); // seal -> objectId
  private history: Map<string, StateTransitionRecord[]> = new Map(); // objectId -> transitions

  async saveObject(record: SmartObjectRecord): Promise<void> {
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

  async getHistory(objectId: string): Promise<StateTransitionRecord[]> {
    return this.history.get(objectId) || [];
  }
}
