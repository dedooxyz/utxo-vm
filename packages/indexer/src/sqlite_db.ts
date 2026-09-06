import Database from "better-sqlite3";
import path from "path";
import { SmartObjectRecord, StateTransitionRecord } from "./state_db";

const DB_PATH = path.join(__dirname, "../data/indexer.db");

export class SqliteStateDB {
  private db: Database.Database;

  constructor(dbPath?: string) {
    const actualPath = dbPath || DB_PATH;
    this.db = new Database(actualPath);
    this.db.pragma("journal_mode = WAL");
    this.db.pragma("synchronous = NORMAL");
    this.init();
  }

  private init() {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS objects (
        object_id TEXT PRIMARY KEY,
        code_hash TEXT,
        seal TEXT UNIQUE NOT NULL,
        satoshis TEXT NOT NULL DEFAULT '0',
        owner TEXT NOT NULL,
        state_data TEXT NOT NULL DEFAULT '{}',
        updated_at_block INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
      );

      CREATE TABLE IF NOT EXISTS state_transitions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        object_id TEXT NOT NULL,
        txid TEXT NOT NULL,
        consumed_seal TEXT NOT NULL,
        new_seal TEXT NOT NULL,
        method TEXT NOT NULL,
        events TEXT NOT NULL DEFAULT '[]',
        block_height INTEGER NOT NULL,
        timestamp INTEGER NOT NULL,
        FOREIGN KEY (object_id) REFERENCES objects(object_id)
      );

      CREATE TABLE IF NOT EXISTS sync_state (
        chain TEXT PRIMARY KEY,
        last_block INTEGER NOT NULL DEFAULT 0,
        last_sync INTEGER NOT NULL DEFAULT (unixepoch())
      );

      CREATE INDEX IF NOT EXISTS idx_objects_seal ON objects(seal);
      CREATE INDEX IF NOT EXISTS idx_objects_owner ON objects(owner);
      CREATE INDEX IF NOT EXISTS idx_transitions_object ON state_transitions(object_id);
      CREATE INDEX IF NOT EXISTS idx_transitions_txid ON state_transitions(txid);
      CREATE INDEX IF NOT EXISTS idx_transitions_block ON state_transitions(block_height);
    `);
  }

  async saveObject(record: SmartObjectRecord): Promise<void> {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO objects (object_id, code_hash, seal, satoshis, owner, state_data, updated_at_block)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `);
    stmt.run(
      record.objectId,
      record.codeHash,
      record.seal,
      record.satoshis,
      record.owner,
      JSON.stringify(record.stateData),
      record.updatedAtBlock
    );
  }

  async getObject(objectId: string): Promise<SmartObjectRecord | null> {
    const stmt = this.db.prepare("SELECT * FROM objects WHERE object_id = ?");
    const row = stmt.get(objectId) as any;
    if (!row) return null;
    return {
      objectId: row.object_id,
      codeHash: row.code_hash,
      seal: row.seal,
      satoshis: row.satoshis,
      owner: row.owner,
      stateData: JSON.parse(row.state_data),
      updatedAtBlock: row.updated_at_block,
    };
  }

  async getObjectBySeal(seal: string): Promise<SmartObjectRecord | null> {
    const stmt = this.db.prepare("SELECT * FROM objects WHERE seal = ?");
    const row = stmt.get(seal) as any;
    if (!row) return null;
    return {
      objectId: row.object_id,
      codeHash: row.code_hash,
      seal: row.seal,
      satoshis: row.satoshis,
      owner: row.owner,
      stateData: JSON.parse(row.state_data),
      updatedAtBlock: row.updated_at_block,
    };
  }

  async getAllObjects(): Promise<SmartObjectRecord[]> {
    const stmt = this.db.prepare("SELECT * FROM objects ORDER BY updated_at_block DESC");
    const rows = stmt.all() as any[];
    return rows.map(row => ({
      objectId: row.object_id,
      codeHash: row.code_hash,
      seal: row.seal,
      satoshis: row.satoshis,
      owner: row.owner,
      stateData: JSON.parse(row.state_data),
      updatedAtBlock: row.updated_at_block,
    }));
  }

  async recordTransition(objectId: string, transition: StateTransitionRecord): Promise<void> {
    const stmt = this.db.prepare(`
      INSERT INTO state_transitions (object_id, txid, consumed_seal, new_seal, method, events, block_height, timestamp)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    `);
    stmt.run(
      objectId,
      transition.txid,
      transition.consumedSeal,
      transition.newSeal,
      transition.method,
      JSON.stringify(transition.events),
      transition.blockHeight,
      transition.timestamp
    );
  }

  async getTransitions(objectId: string): Promise<StateTransitionRecord[]> {
    const stmt = this.db.prepare(
      "SELECT * FROM state_transitions WHERE object_id = ? ORDER BY block_height ASC"
    );
    const rows = stmt.all(objectId) as any[];
    return rows.map(row => ({
      txid: row.txid,
      consumedSeal: row.consumed_seal,
      newSeal: row.new_seal,
      method: row.method,
      events: JSON.parse(row.events),
      blockHeight: row.block_height,
      timestamp: row.timestamp,
    }));
  }

  async getLastSyncBlock(chain: string): Promise<number> {
    const stmt = this.db.prepare("SELECT last_block FROM sync_state WHERE chain = ?");
    const row = stmt.get(chain) as any;
    return row ? row.last_block : 0;
  }

  async setLastSyncBlock(chain: string, block: number): Promise<void> {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO sync_state (chain, last_block, last_sync)
      VALUES (?, ?, unixepoch())
    `);
    stmt.run(chain, block);
  }

  async getStats(): Promise<{
    totalObjects: number;
    totalTransitions: number;
    lastSyncBlock: number;
  }> {
    const objectsCount = (this.db.prepare("SELECT COUNT(*) as count FROM objects").get() as any).count;
    const transitionsCount = (this.db.prepare("SELECT COUNT(*) as count FROM state_transitions").get() as any).count;
    const lastSync = await this.getLastSyncBlock("JKC");
    
    return {
      totalObjects: objectsCount,
      totalTransitions: transitionsCount,
      lastSyncBlock: lastSync,
    };
  }

  close() {
    this.db.close();
  }
}
