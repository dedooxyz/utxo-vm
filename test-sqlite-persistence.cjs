#!/usr/bin/env node

/**
 * SQLite Persistence Test
 * Verifies that the indexer's SQLite database correctly persists and retrieves data.
 */

const BASE = "http://localhost:9773";

async function fetchJSON(url, opts) {
  const res = await fetch(url, opts);
  return res.json();
}

async function runTests() {
  console.log("=== SQLite Persistence Test ===\n");
  let passed = 0;
  let failed = 0;

  function assert(label, condition, detail) {
    if (condition) {
      console.log(`  ✅ ${label}${detail ? ` — ${detail}` : ""}`);
      passed++;
    } else {
      console.log(`  ❌ ${label}${detail ? ` — ${detail}` : ""}`);
      failed++;
    }
  }

  // 1. Check stats endpoint
  const stats = await fetchJSON(`${BASE}/api/v1/stats`);
  assert("Stats endpoint returns data", stats.totalObjects > 0, `${stats.totalObjects} objects`);

  // 2. Get all objects
  const objects = await fetchJSON(`${BASE}/api/v1/objects`);
  assert("Objects endpoint returns array", Array.isArray(objects), `${objects.length} objects`);

  // 3. Check object structure
  if (objects.length > 0) {
    const obj = objects[0];
    assert("Object has objectId", !!obj.objectId);
    assert("Object has seal", !!obj.seal);
    assert("Object has owner", !!obj.owner);
    assert("Object has stateData", !!obj.stateData);
    assert("Object has updatedAtBlock", obj.updatedAtBlock > 0, `block ${obj.updatedAtBlock}`);
  }

  // 4. Get single object by ID
  if (objects.length > 0) {
    const objId = objects[0].objectId;
    const single = await fetchJSON(`${BASE}/api/v1/object/${objId}`);
    assert("Get object by ID works", single.objectId === objId);
  }

  // 5. Get object by seal
  if (objects.length > 0) {
    const seal = objects[0].seal;
    const bySeal = await fetchJSON(`${BASE}/api/v1/object/${seal}`);
    assert("Get object by seal works", bySeal.seal === seal);
  }

  // 6. Get state transitions
  if (objects.length > 0) {
    const objId = objects[0].objectId;
    const history = await fetchJSON(`${BASE}/api/v1/object/${objId}/history`);
    assert("State transitions endpoint returns array", Array.isArray(history));
  }

  // 7. Parse envelope endpoint — proper pushdata-encoded format
  // Each field needs its own pushdata prefix: <len> <data>
  const protocolField = "06" + "7574786f766d";           // "utxovm" (6 bytes)
  const versionField = "01" + "01";                       // version 1
  const contentTypeField = "10" + "6170706c69636174696f6e2f6a736f6e"; // "application/json" (16 bytes)
  const payloadField = "0f" + "7b226e616d65223a2254657374227d";       // {"name":"Test"} (15 bytes)
  const endifByte = "68";                                  // OP_ENDIF
  const innerData = protocolField + versionField + contentTypeField + payloadField + endifByte;
  const envelopeData = "0063" + innerData;                 // OP_FALSE OP_IF + data
  const pushLen = envelopeData.length / 2;
  let pushByte;
  if (pushLen < 0x4c) {
    pushByte = pushLen.toString(16).padStart(2, "0");
  } else {
    pushByte = "4c" + pushLen.toString(16).padStart(2, "0");
  }
  const scriptPubKey = "6a" + pushByte + envelopeData;
  const parseRes = await fetchJSON(`${BASE}/api/v1/parse-envelope`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ scriptPubKey })
  });
  assert("Parse envelope endpoint works", parseRes.protocol === "utxovm", parseRes.contentType);

  // 8. Stats has block height
  assert("Stats has blockHeight", typeof stats.blockHeight === "number" && stats.blockHeight > 0, `height ${stats.blockHeight}`);

  console.log(`\n=== Results: ${passed}/${passed + failed} passed ===`);
  process.exit(failed > 0 ? 1 : 0);
}

runTests().catch(err => {
  console.error("Test failed:", err.message);
  process.exit(1);
});
