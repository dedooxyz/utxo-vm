import express from "express";
import { SqliteStateDB } from "./sqlite_db";
import { ChainBlockScanner, parseEnvelope } from "./scanner";
import { getChainConfig, getAvailableChains } from "./chains";

const app = express();
app.use(express.json());

// Load chain config from chain.json
const CHAIN = process.env.CHAIN || "JKC_TESTNET";
const chainConfig = getChainConfig(CHAIN);
const ELECTRS_URL = process.env.ELECTRS_URL || chainConfig.electrsUrl;
const DB_PATH = process.env.DB_PATH || undefined;
const SYNC_INTERVAL_MS = parseInt(process.env.SYNC_INTERVAL_MS || "30000", 10);
const db = new SqliteStateDB(DB_PATH);
const scanner = new ChainBlockScanner(db as any, CHAIN, ELECTRS_URL);

let isSyncing = false;
let lastSyncedBlock = 0;

// --- Continuous block sync ---
async function continuousSync() {
  if (isSyncing) return;
  isSyncing = true;
  try {
    const lastSync = await db.getLastSyncBlock(CHAIN);
    const startBlock = lastSync > 0 ? lastSync + 1 : undefined;
    const result = await scanner.syncFromElectrs(startBlock);
    if (result.indexed > 0) {
      console.log(`[Indexer] Synced ${result.indexed} envelopes from ${result.blocks} blocks (height: ${scanner.currentBlockHeight})`);
    }
    lastSyncedBlock = scanner.currentBlockHeight;
    await db.setLastSyncBlock(CHAIN, scanner.currentBlockHeight);

    // Also scan mempool every cycle
    const mempoolResult = await scanner.scanMempool();
    if (mempoolResult.indexed > 0) {
      console.log(`[Indexer] Mempool: ${mempoolResult.indexed} new envelopes (${mempoolResult.txids.join(", ")})`);
    }
  } catch (err: any) {
    console.error(`[Indexer] Sync error: ${err.message}`);
  } finally {
    isSyncing = false;
  }
}

// --- Auto-sync on startup ---
async function startup() {
  await continuousSync();
  setInterval(continuousSync, SYNC_INTERVAL_MS);
  console.log(`[Indexer] Continuous sync enabled (interval: ${SYNC_INTERVAL_MS}ms)`);
}
startup();

// --- API Routes ---

// 1. Get Chain Info
app.get("/api/v1/chain/info", async (_req, res) => {
  res.json({
    chain: scanner.chain,
    chainName: chainConfig.name,
    ticker: chainConfig.ticker,
    electrs: ELECTRS_URL,
    blockHeight: scanner.currentBlockHeight,
    protocolVersion: 1,
    vm: "utxo-core-vm",
    opcodesSupported: chainConfig.opcodesSupported,
    env: chainConfig.env,
    status: "synced",
  });
});

// 1b. Get Available Chains
app.get("/api/v1/chains", async (_req, res) => {
  res.json({
    chains: getAvailableChains(),
    current: CHAIN,
  });
});

// 2. Get Smart Object by ID or Seal
app.get("/api/v1/object/:id", async (req, res) => {
  let obj = await db.getObject(req.params.id);
  if (!obj) {
    obj = await db.getObjectBySeal(req.params.id);
  }
  if (!obj) {
    return res.status(404).json({ error: "Smart Object not found" });
  }
  res.json(obj);
});

// 3. Get Object State Transition History
app.get("/api/v1/object/:id/history", async (req, res) => {
  const history = await db.getTransitions(req.params.id);
  res.json(history);
});

// 4. Get Objects by Owner
app.get("/api/v1/objects/owner/:owner", async (req, res) => {
  const all = await db.getAllObjects();
  const objects = all.filter(o => o.owner === req.params.owner);
  res.json(objects);
});

// 5. Get All Objects
app.get("/api/v1/objects", async (_req, res) => {
  const all = await db.getAllObjects();
  res.json(all);
});

// 6. Get Stats
app.get("/api/v1/stats", async (_req, res) => {
  const stats = await db.getStats();
  res.json({
    ...stats,
    blockHeight: scanner.currentBlockHeight,
    chain: scanner.chain,
  });
});

// 7. Simulate Execution
app.post("/api/v1/simulate", async (req, res) => {
  const { targetSeal, method, args } = req.body;
  const obj = await db.getObjectBySeal(targetSeal);

  if (!obj) {
    return res.status(404).json({ success: false, error: "Target seal not found" });
  }

  const simulatedState = { ...obj.stateData, simulatedMethod: method };
  if (args?.to) {
    simulatedState.owner = args.to;
  }

  res.json({
    success: true,
    gasConsumed: 25_420,
    returnCode: 0,
    updatedState: simulatedState,
    events: [{ topic: "SimulatedExecution", data: `Method ${method} executed successfully` }],
  });
});

// 8. Broadcast Transaction & Inscription
app.post("/api/v1/broadcast", async (req, res) => {
  const { chain, envelope, sender, attachedSatoshis } = req.body;
  const txid = "tx_" + Math.random().toString(16).slice(2) + Math.random().toString(16).slice(2);
  const seal = `${txid}:0`;

  const blockTx = {
    txid,
    inputs: [{ txid: "prev_tx", vout: 0 }],
    outputs: [{ vout: 0, satoshis: BigInt(attachedSatoshis || 1000), scriptPubKeyHex: "5120..." }],
    envelope: {
      protocol: envelope.protocol || "utxovm",
      version: envelope.version || 1,
      contentType: envelope.contentType || "application/json",
      payload: envelope.payload,
      metadata: {
        ...envelope.metadata,
        caller: sender,
        owner: sender,
      },
    },
  };

  await scanner.processTransaction(blockTx);

  res.json({
    txid,
    status: "mempool",
    seal,
  });
});

// 9. Re-sync from electrs
app.post("/api/v1/sync", async (req, res) => {
  const { sinceBlock } = req.body;
  try {
    const result = await scanner.syncFromElectrs(sinceBlock);
    await db.setLastSyncBlock(CHAIN, scanner.currentBlockHeight);
    res.json({ success: true, ...result });
  } catch (err: any) {
    res.status(500).json({ success: false, error: err.message });
  }
});

// 10. Sync status
app.get("/api/v1/sync/status", async (_req, res) => {
  const lastSync = await db.getLastSyncBlock(CHAIN);
  res.json({
    isSyncing,
    lastSyncedBlock,
    lastPersistedBlock: lastSync,
    currentHeight: scanner.currentBlockHeight,
    intervalMs: SYNC_INTERVAL_MS,
  });
});

// 11. Scan mempool
app.post("/api/v1/mempool/scan", async (_req, res) => {
  try {
    const result = await scanner.scanMempool();
    res.json({ success: true, ...result });
  } catch (err: any) {
    res.status(500).json({ success: false, error: err.message });
  }
});

// 12. Parse envelope utility
app.post("/api/v1/parse-envelope", async (req, res) => {
  const { scriptPubKey } = req.body;
  if (!scriptPubKey) {
    return res.status(400).json({ error: "scriptPubKey required" });
  }
  const envelope = parseEnvelope(scriptPubKey);
  if (!envelope) {
    return res.status(400).json({ error: "Not a valid utxovm envelope" });
  }
  // Parse JSON payload if applicable
  let payload: any = envelope.payload;
  if (envelope.contentType === "application/json") {
    try {
      payload = JSON.parse(Buffer.from(envelope.payload).toString("utf8"));
    } catch {}
  }
  res.json({ ...envelope, payload });
});


// 13. State Root endpoints
app.get("/api/v1/state-root", async (_req, res) => {
  const height = scanner.currentBlockHeight;
  const stateRoot = await db.getStateRoot(CHAIN, height);
  res.json({
    chain: CHAIN,
    blockHeight: height,
    stateRoot: stateRoot || null,
  });
});

app.get("/api/v1/state-root/:height", async (req, res) => {
  const height = parseInt(req.params.height, 10);
  const stateRoot = await db.getStateRoot(CHAIN, height);
  if (!stateRoot) {
    return res.status(404).json({ error: `State root not found for height ${height}` });
  }
  res.json({
    chain: CHAIN,
    blockHeight: height,
    stateRoot,
  });
});

// 14. Unconfirmed mempool transitions
app.get("/api/v1/mempool/transitions", async (_req, res) => {
  const transitions = db.getMempoolTransitions ? await db.getMempoolTransitions(CHAIN) : [];
  res.json(transitions);
});

const PORT = process.env.PORT || 9773;
app.listen(PORT, () => {
  console.log(`[UTXO-VM Indexer] Server listening on port ${PORT}`);
  console.log(`[UTXO-VM Indexer] Electrs: ${ELECTRS_URL}`);
});

export { app, db, scanner };
