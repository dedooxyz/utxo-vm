import Database from "better-sqlite3";
import path from "path";
import crypto from "crypto";
import {
  SmartObjectRecord,
  StateTransitionRecord,
  BlockHeaderRecord,
  MempoolTransitionRecord,
  IStateDB,
} from "./state_db";

const DB_PATH = path.join(__dirname, "../data/indexer.db");

export class SqliteStateDB implements IStateDB {
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

      CREATE TABLE IF NOT EXISTS block_hashes (
        chain TEXT NOT NULL,
        block_height INTEGER NOT NULL,
        block_hash TEXT NOT NULL,
        prev_hash TEXT,
        state_root TEXT,
        created_at INTEGER NOT NULL DEFAULT (unixepoch()),
        PRIMARY KEY (chain, block_height)
      );

      CREATE TABLE IF NOT EXISTS state_undo_logs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        chain TEXT NOT NULL,
        block_height INTEGER NOT NULL,
        object_id TEXT NOT NULL,
        action TEXT NOT NULL,
        prev_code_hash TEXT,
        prev_seal TEXT,
        prev_satoshis TEXT,
        prev_owner TEXT,
        prev_state_data TEXT,
        prev_updated_at_block INTEGER,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
      );

      CREATE TABLE IF NOT EXISTS mempool_transitions (
        txid TEXT PRIMARY KEY,
        chain TEXT NOT NULL,
        object_id TEXT NOT NULL,
        consumed_seal TEXT NOT NULL,
        new_seal TEXT NOT NULL,
        method TEXT NOT NULL,
        events TEXT NOT NULL DEFAULT '[]',
        received_at INTEGER NOT NULL DEFAULT (unixepoch())
      );

      CREATE INDEX IF NOT EXISTS idx_objects_seal ON objects(seal);
      CREATE INDEX IF NOT EXISTS idx_objects_owner ON objects(owner);
      CREATE INDEX IF NOT EXISTS idx_transitions_object ON state_transitions(object_id);
      CREATE INDEX IF NOT EXISTS idx_transitions_txid ON state_transitions(txid);
      CREATE INDEX IF NOT EXISTS idx_transitions_block ON state_transitions(block_height);
      CREATE INDEX IF NOT EXISTS idx_undo_chain_height ON state_undo_logs(chain, block_height);
      CREATE INDEX IF NOT EXISTS idx_block_hashes_chain ON block_hashes(chain, block_height);
    `);
  }

  async saveObject(record: SmartObjectRecord, chain: string = "JKC"): Promise<void> {
    const existing = this.db
      .prepare("SELECT * FROM objects WHERE object_id = ?")
      .get(record.objectId) as any;

    if (existing) {
      this.db
        .prepare(`
          INSERT INTO state_undo_logs (
            chain, block_height, object_id, action,
            prev_code_hash, prev_seal, prev_satoshis, prev_owner, prev_state_data, prev_updated_at_block
          )
          VALUES (?, ?, ?, 'UPDATE', ?, ?, ?, ?, ?, ?)
        `)
        .run(
          chain,
          record.updatedAtBlock,
          record.objectId,
          existing.code_hash,
          existing.seal,
          existing.satoshis,
          existing.owner,
          existing.state_data,
          existing.updated_at_block
        );
    } else {
      this.db
        .prepare(`
          INSERT INTO state_undo_logs (chain, block_height, object_id, action)
          VALUES (?, ?, ?, 'CREATE')
        `)
        .run(chain, record.updatedAtBlock, record.objectId);
    }

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

  async getObject(objectId: string): Promise<SmartObjectRecord | undefined> {
    const stmt = this.db.prepare("SELECT * FROM objects WHERE object_id = ?");
    const row = stmt.get(objectId) as any;
    if (!row) return undefined;
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

  async getObjectBySeal(seal: string): Promise<SmartObjectRecord | undefined> {
    const stmt = this.db.prepare("SELECT * FROM objects WHERE seal = ?");
    const row = stmt.get(seal) as any;
    if (!row) return undefined;
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

  async getObjectsByOwner(owner: string): Promise<SmartObjectRecord[]> {
    const stmt = this.db.prepare("SELECT * FROM objects WHERE owner = ? ORDER BY updated_at_block DESC");
    const rows = stmt.all(owner) as any[];
    return rows.map((row) => ({
      objectId: row.object_id,
      codeHash: row.code_hash,
      seal: row.seal,
      satoshis: row.satoshis,
      owner: row.owner,
      stateData: JSON.parse(row.state_data),
      updatedAtBlock: row.updated_at_block,
    }));
  }

  async getAllObjects(): Promise<SmartObjectRecord[]> {
    const stmt = this.db.prepare("SELECT * FROM objects ORDER BY updated_at_block DESC");
    const rows = stmt.all() as any[];
    return rows.map((row) => ({
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
    return rows.map((row) => ({
      txid: row.txid,
      consumedSeal: row.consumed_seal,
      newSeal: row.new_seal,
      method: row.method,
      events: JSON.parse(row.events),
      blockHeight: row.block_height,
      timestamp: row.timestamp,
    }));
  }

  async getHistory(objectId: string): Promise<StateTransitionRecord[]> {
    return this.getTransitions(objectId);
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

  async recordBlockHeader(
    chain: string,
    height: number,
    hash: string,
    prevHash?: string,
    stateRoot?: string
  ): Promise<void> {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO block_hashes (chain, block_height, block_hash, prev_hash, state_root, created_at)
      VALUES (?, ?, ?, ?, ?, unixepoch())
    `);
    stmt.run(chain, height, hash, prevHash || null, stateRoot || null);
  }

  async getBlockHash(chain: string, height: number): Promise<string | undefined> {
    const stmt = this.db.prepare("SELECT block_hash FROM block_hashes WHERE chain = ? AND block_height = ?");
    const row = stmt.get(chain, height) as any;
    return row ? row.block_hash : undefined;
  }

  async getBlockHeader(chain: string, height: number): Promise<BlockHeaderRecord | undefined> {
    const stmt = this.db.prepare("SELECT * FROM block_hashes WHERE chain = ? AND block_height = ?");
    const row = stmt.get(chain, height) as any;
    if (!row) return undefined;
    return {
      chain: row.chain,
      blockHeight: row.block_height,
      blockHash: row.block_hash,
      prevHash: row.prev_hash || undefined,
      stateRoot: row.state_root || undefined,
      createdAt: row.created_at,
    };
  }

  async computeAndSaveStateRoot(chain: string, height: number): Promise<string> {
    const rows = this.db
      .prepare("SELECT object_id, code_hash, seal, satoshis, owner, state_data FROM objects ORDER BY object_id ASC")
      .all() as any[];

    const hasher = crypto.createHash("sha256");
    for (const row of rows) {
      hasher.update(
        `${row.object_id}:${row.code_hash}:${row.seal}:${row.satoshis}:${row.owner}:${row.state_data};`
      );
    }
    const stateRoot = hasher.digest("hex");

    this.db
      .prepare("UPDATE block_hashes SET state_root = ? WHERE chain = ? AND block_height = ?")
      .run(stateRoot, chain, height);

    return stateRoot;
  }

  async getStateRoot(chain: string, height: number): Promise<string | undefined> {
    const stmt = this.db.prepare("SELECT state_root FROM block_hashes WHERE chain = ? AND block_height = ?");
    const row = stmt.get(chain, height) as any;
    return row ? row.state_root || undefined : undefined;
  }

  async rollbackToBlock(
    chain: string,
    targetHeight: number
  ): Promise<{ rolledBackObjects: number; rolledBackTransitions: number }> {
    const rollback = this.db.transaction(() => {
      const undoRows = this.db
        .prepare(
          "SELECT * FROM state_undo_logs WHERE chain = ? AND block_height > ? ORDER BY id DESC"
        )
        .all(chain, targetHeight) as any[];

      let rolledBackObjects = 0;

      for (const undo of undoRows) {
        if (undo.action === "CREATE") {
          this.db.prepare("DELETE FROM objects WHERE object_id = ?").run(undo.object_id);
        } else if (undo.action === "UPDATE") {
          this.db
            .prepare(`
              UPDATE objects
              SET code_hash = ?, seal = ?, satoshis = ?, owner = ?, state_data = ?, updated_at_block = ?
              WHERE object_id = ?
            `)
            .run(
              undo.prev_code_hash,
              undo.prev_seal,
              undo.prev_satoshis,
              undo.prev_owner,
              undo.prev_state_data,
              undo.prev_updated_at_block,
              undo.object_id
            );
        }
        rolledBackObjects++;
      }

      const transCountRow = this.db
        .prepare("SELECT COUNT(*) as count FROM state_transitions WHERE block_height > ?")
        .get(targetHeight) as any;
      const rolledBackTransitions = transCountRow ? transCountRow.count : 0;

      this.db.prepare("DELETE FROM state_transitions WHERE block_height > ?").run(targetHeight);
      this.db.prepare("DELETE FROM state_undo_logs WHERE chain = ? AND block_height > ?").run(chain, targetHeight);
      this.db.prepare("DELETE FROM block_hashes WHERE chain = ? AND block_height > ?").run(chain, targetHeight);
      this.db.prepare("UPDATE sync_state SET last_block = ? WHERE chain = ?").run(targetHeight, chain);

      return { rolledBackObjects, rolledBackTransitions };
    });

    return rollback();
  }

  async recordMempoolTransition(transition: MempoolTransitionRecord): Promise<void> {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO mempool_transitions (txid, chain, object_id, consumed_seal, new_seal, method, events, received_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, unixepoch())
    `);
    stmt.run(
      transition.txid,
      transition.chain,
      transition.objectId,
      transition.consumedSeal,
      transition.newSeal,
      transition.method,
      JSON.stringify(transition.events)
    );
  }

  async getMempoolTransitions(chain: string): Promise<MempoolTransitionRecord[]> {
    const rows = this.db
      .prepare("SELECT * FROM mempool_transitions WHERE chain = ? ORDER BY received_at DESC")
      .all(chain) as any[];

    return rows.map((r) => ({
      txid: r.txid,
      chain: r.chain,
      objectId: r.object_id,
      consumedSeal: r.consumed_seal,
      newSeal: r.new_seal,
      method: r.method,
      events: JSON.parse(r.events),
      receivedAt: r.received_at,
    }));
  }

  async removeMempoolTransition(txid: string): Promise<void> {
    this.db.prepare("DELETE FROM mempool_transitions WHERE txid = ?").run(txid);
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
