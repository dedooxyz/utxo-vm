#!/usr/bin/env node
/**
 * JKC Testnet — Real Electrs Indexer
 *
 * Connects to real electrs, parses OP_RETURN utxovm envelopes,
 * and indexes smart objects + state transitions.
 */

const ELECTRS = "https://jkc-testnet-api.s3na.xyz";

// --- Envelope parser ---
function parseEnvelope(scriptPubKeyHex) {
  const buf = Buffer.from(scriptPubKeyHex, "hex");
  if (buf[0] !== 0x6a) return null;

  // Read outer push (OP_RETURN data)
  let offset = 1;
  const first = buf[offset++];
  let envelopeLen;
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

  // Check OP_FALSE OP_IF
  if (envelope[0] !== 0x00 || envelope[1] !== 0x63) return null;

  // Parse inner pushes
  let envOff = 2;
  function readPush() {
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

  const contentType = readPush()?.toString("utf8");

  // Remaining is payload (before final 0x68 OP_ENDIF)
  // The payload push may have its own OP_PUSHDATA prefix, so use readPush
  const payloadPush = readPush();
  const payload = payloadPush || Buffer.alloc(0);
  // Verify OP_ENDIF follows
  if (envelope[envOff] !== 0x68) return null;

  return { protocol, version, contentType, payload: Array.from(payload) };
}

// --- Fetch helpers ---
async function electrsGet(path) {
  const res = await fetch(`${ELECTRS}${path}`);
  if (!res.ok) throw new Error(`GET ${path}: ${res.status}`);
  return res;
}
async function electrsGetText(path) { return (await (await electrsGet(path)).text()).trim(); }
async function electrsGetJson(path) { return (await electrsGet(path)).json(); }

// --- State DB (in-memory) ---
const objects = new Map();   // objectId -> record
const sealIndex = new Map(); // seal -> objectId
const history = new Map();   // objectId -> transitions[]

function saveObject(record) {
  objects.set(record.objectId, record);
  sealIndex.set(record.seal, record.objectId);
}

function getObjectBySeal(seal) {
  const id = sealIndex.get(seal);
  return id ? objects.get(id) : undefined;
}

function recordTransition(objectId, transition) {
  sealIndex.delete(transition.consumedSeal);
  sealIndex.set(transition.newSeal, objectId);
  const hist = history.get(objectId) || [];
  hist.push(transition);
  history.set(objectId, hist);
}

// --- Process transaction ---
function processTx(tx, blockHeight) {
  if (!tx.envelope || tx.envelope.protocol !== "utxovm") return null;

  const { contentType, payload, metadata } = tx.envelope;

  // Deploy: application/wasm
  if (contentType === "application/wasm") {
    const objectId = "obj_" + tx.txid.slice(0, 16);
    const seal = `${tx.txid}:0`;
    const satoshis = String(tx.vout?.[0]?.value || 0);
    const owner = metadata?.owner || "03" + tx.txid.slice(0, 64);

    const record = {
      objectId,
      codeHash: "hash_" + tx.txid.slice(0, 16),
      seal,
      satoshis,
      owner,
      stateData: metadata?.initialState || { owner, initialized: true },
      updatedAtBlock: blockHeight,
    };
    saveObject(record);
    return { type: "deploy", objectId, seal };
  }

  // Application/octet-stream or other binary: register as raw inscription
  if (contentType === "application/octet-stream") {
    const objectId = "obj_" + tx.txid.slice(0, 16);
    const seal = `${tx.txid}:0`;
    const record = {
      objectId,
      codeHash: "raw_" + tx.txid.slice(0, 16),
      seal,
      satoshis: String(tx.vout?.[0]?.value || 0),
      owner: "03" + tx.txid.slice(0, 64),
      stateData: { type: "raw_inscription", payloadLen: payload.length },
      updatedAtBlock: blockHeight,
    };
    saveObject(record);
    return { type: "raw_inscription", objectId, seal };
  }

  // Method calls: application/json
  if (contentType === "application/json" && metadata?.targetSeal) {
    const targetSeal = metadata.targetSeal;
    const existing = getObjectBySeal(targetSeal);
    if (!existing) return { type: "orphan", targetSeal };

    const method = metadata.method || "call";
    const newSeal = `${tx.txid}:0`;
    const newState = { ...existing.stateData, lastMethod: method, lastCaller: metadata.caller || existing.owner };
    if (metadata.args?.to) newState.owner = metadata.args.to;

    const updated = { ...existing, seal: newSeal, owner: newState.owner || existing.owner, stateData: newState, updatedAtBlock: blockHeight };
    saveObject(updated);

    const transition = {
      txid: tx.txid,
      consumedSeal: targetSeal,
      newSeal,
      method,
      events: [{ topic: "StateTransition", data: `Invoked ${method}` }],
      blockHeight,
      timestamp: Date.now(),
    };
    recordTransition(existing.objectId, transition);
    return { type: "transition", objectId: existing.objectId, from: targetSeal, to: newSeal, method };
  }

  // Application/json without targetSeal: standalone call (e.g. batch, mint)
  if (contentType === "application/json") {
    const objectId = "obj_" + tx.txid.slice(0, 16);
    const seal = `${tx.txid}:0`;
    const record = {
      objectId,
      codeHash: "call_" + tx.txid.slice(0, 16),
      seal,
      satoshis: String(tx.vout?.[0]?.value || 0),
      owner: metadata?.caller || "03" + tx.txid.slice(0, 64),
      stateData: metadata || {},
      updatedAtBlock: blockHeight,
    };
    saveObject(record);
    return { type: "call", objectId, seal };
  }

  return null;
}

// --- Main ---
async function main() {
  console.log("=== JKC Testnet — Real Electrs Indexer ===\n");

  // 1. Get tip
  const tip = await electrsGetText("/blocks/tip/height");
  console.log(`Tip height: ${tip}`);

  // 2. Get the block containing our test txs
  // Our first tx was a7490625... Let's find which block it's in
  const testTxids = [
    "a74906252d73a1261338a356fd0e53061ad87ebc66f30c77ffc9adc09139d141",  // 1. Deploy WASM
    "7ebf3b55b73642bee6d55eaa23402c28a8578e1ae255870c1d62b746df9bcacb",  // 2. Transfer
    "fa02327b92b5dee23a362dbe248ac1a514ad67aa740ba6c9a7575bd160e5a2ad",  // 3. Stealth
    "d6f03aa5c3f0a629193a7f4beb8bc84af31751fdba5bea275b3b78aa87a02974",  // 4. State transition
    "92559b576c1d2ea7a6143f864b9355e2e9b5081c1c264d1401a5f6bbfd6d6d9e",  // 5. Batch
    "c3efc347a272b7bfb783443f75cb9836543f882ef36f4e8c2b005e0703c4c543",  // 6. Tiny
    "13b387c4c66205a10ab09b9ca109673f49ecf2f322c93d42fe5f0c733e805f5a",  // 7. Large
  ];

  console.log(`\nFetching ${testTxids.length} test transactions...`);

  let indexed = 0;
  let failed = 0;

  for (let i = 0; i < testTxids.length; i++) {
    const txid = testTxids[i];
    try {
      // Get full tx from electrs
      const txInfo = await electrsGetJson(`/tx/${txid}`);
      const blockHeight = txInfo.status?.block_height || 0;

      // Find ALL OP_RETURN outputs (batch: multiple envelopes in 1 tx)
      const opReturns = txInfo.vout.filter(o => o.scriptpubkey_type === "op_return");
      if (!opReturns.length) {
        console.log(`  [${i + 1}] ${txid.slice(0, 16)}... — no OP_RETURN`);
        failed++;
        continue;
      }

      let allParsed = true;
      const results = [];
      for (const opReturn of opReturns) {
        const envelope = parseEnvelope(opReturn.scriptpubkey);
        if (!envelope) {
          allParsed = false;
          break;
        }
        results.push(envelope);
      }

      if (!allParsed) {
        console.log(`  [${i + 1}] ${txid.slice(0, 16)}... — not a utxovm envelope`);
        failed++;
        continue;
      }

      // Process each envelope
      for (const envelope of results) {
        const blockTx = {
          txid,
          inputs: txInfo.vin.map(v => ({ txid: v.prevout?.txid || "", vout: v.prevout?.vout || 0 })),
          outputs: txInfo.vout.map((o, idx) => ({ vout: idx, satoshis: BigInt(o.value), scriptPubKeyHex: o.scriptpubkey })),
          envelope: {
            protocol: envelope.protocol,
            version: envelope.version,
            contentType: envelope.contentType,
            payload: envelope.payload,
            metadata: envelope.contentType === "application/json" ? JSON.parse(Buffer.from(envelope.payload).toString("utf8")) : {},
          },
        };

        const result = processTx(blockTx, blockHeight);
        if (result) {
          console.log(`  [${i + 1}] ${txid.slice(0, 16)}... — ${result.type} at block ${blockHeight}`);
          indexed++;
        }
      }
    } catch (err) {
      console.log(`  [${i + 1}] ${txid.slice(0, 16)}... — error: ${err.message}`);
      failed++;
    }
  }

  // Summary
  console.log(`\n${"=".repeat(60)}`);
  console.log(`INDEXING COMPLETE`);
  console.log(`${"=".repeat(60)}`);
  console.log(`Indexed: ${indexed} | Failed: ${failed}`);
  console.log(`Smart Objects: ${objects.size}`);
  console.log(`Seals indexed: ${sealIndex.size}`);

  // List all objects
  console.log(`\n--- Indexed Smart Objects ---`);
  for (const [id, obj] of objects) {
    console.log(`  ${id}`);
    console.log(`    Seal: ${obj.seal}`);
    console.log(`    Owner: ${obj.owner.slice(0, 20)}...`);
    console.log(`    Satoshis: ${obj.satoshis}`);
    console.log(`    State: ${JSON.stringify(obj.stateData).slice(0, 80)}`);
    console.log(`    Block: ${obj.updatedAtBlock}`);
  }

  // List all transitions
  console.log(`\n--- State Transitions ---`);
  for (const [id, trans] of history) {
    for (const t of trans) {
      console.log(`  ${id}: ${t.consumedSeal} → ${t.newSeal} (${t.method})`);
    }
  }

  // Demo API response
  console.log(`\n--- API Demo: GET /api/v1/object/:id ---`);
  for (const [id] of objects) {
    const obj = objects.get(id);
    console.log(JSON.stringify(obj, null, 2));
  }
}

main().catch(console.error);
