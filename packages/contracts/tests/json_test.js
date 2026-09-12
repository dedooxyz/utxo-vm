const fs = require("fs");
const path = require("path");
const assert = require("assert");

async function runTests() {
  const wasmPath = path.join(__dirname, "../build/release.wasm");
  const wasmBytes = fs.readFileSync(wasmPath);

  let caller = "03deadbeef";
  let events = [];

  let instance;

  const importObject = {
    env: {
      host_get_caller: (out_ptr) => {
        const bytes = new TextEncoder().encode(caller);
        new Uint8Array(instance.exports.memory.buffer, out_ptr, bytes.length).set(bytes);
        return bytes.length;
      },
      host_get_satoshis: () => 100000n,
      host_get_seal: (out_ptr) => {
        const bytes = new TextEncoder().encode("txid_test:0");
        new Uint8Array(instance.exports.memory.buffer, out_ptr, bytes.length).set(bytes);
        return bytes.length;
      },
      host_emit_event: (topic_ptr, data_ptr, len) => {
        const mem = new Uint8Array(instance.exports.memory.buffer);
        // read null-terminated topic
        let topicLen = 0;
        while (topicLen < 64 && mem[topic_ptr + topicLen] !== 0) {
          topicLen++;
        }
        const topic = new TextDecoder().decode(mem.subarray(topic_ptr, topic_ptr + topicLen));
        const data = new TextDecoder().decode(mem.subarray(data_ptr, data_ptr + len));
        events.push({ topic, data });
      },
      host_create_object: () => 1,
      host_stealth_settle: () => 1,
      host_mweb_peg_out: () => 1,
      abort: (msg, file, line, col) => {
        throw new Error(`AssemblyScript abort: ${file}:${line}:${col}`);
      },
    },
  };

  const module = await WebAssembly.instantiate(wasmBytes, importObject);
  instance = module.instance;

  function callInit(argsStr) {
    events = [];
    const bytes = new TextEncoder().encode(argsStr);
    const ptr = instance.exports.allocate(bytes.length);
    new Uint8Array(instance.exports.memory.buffer, ptr, bytes.length).set(bytes);
    const res = instance.exports.init(ptr, bytes.length);
    instance.exports.deallocate(ptr, bytes.length);
    return res;
  }

  function callMethod(method, argsStr) {
    events = [];
    const enc = new TextEncoder().encode(method);
    const mBytes = new Uint8Array(enc.length + 1);
    mBytes.set(enc);
    mBytes[enc.length] = 0;
    const mPtr = instance.exports.allocate(mBytes.length);
    new Uint8Array(instance.exports.memory.buffer, mPtr, mBytes.length).set(mBytes);

    const aBytes = new TextEncoder().encode(argsStr);
    const aPtr = instance.exports.allocate(aBytes.length);
    new Uint8Array(instance.exports.memory.buffer, aPtr, aBytes.length).set(aBytes);

    const res = instance.exports.call(mPtr, aPtr, aBytes.length);
    instance.exports.deallocate(mPtr, mBytes.length);
    instance.exports.deallocate(aPtr, aBytes.length);
    return res;
  }

  function getState() {
    const size = instance.exports.get_state_size();
    const ptr = instance.exports.allocate(size);
    const len = instance.exports.get_state(ptr);
    const bytes = new Uint8Array(instance.exports.memory.buffer, ptr, len);
    const stateStr = new TextDecoder().decode(bytes);
    instance.exports.deallocate(ptr, size);
    return stateStr;
  }

  console.log("Running Issue 2 Contract JSON Tests...");

  // Test 1: String value containing an escaped quote and backslash
  {
    console.log("  Test 1: Escaped quotes in JSON string values");
    const initJson = JSON.stringify({
      type: "SOT",
      name: 'Token "Escaped" Name',
      symbol: "TEN",
      decimals: 8,
      totalSupply: 1000000,
      balance: 1000000,
      owner: "03deadbeef",
    });
    const initRes = callInit(initJson);
    assert.strictEqual(initRes, 0, "Init with escaped quotes must succeed");

    const state = getState();
    assert(state.includes('Token "Escaped" Name') || state.includes('Token \\"Escaped\\" Name'), "State must preserve uncorrupted string");

    // Transfer with escaped chars in args
    const callRes = callMethod("transfer", JSON.stringify({
      to: "03deadbeef",
      amount: 1000000,
    }));
    assert.strictEqual(callRes, 0, "Call must succeed");
    console.log("  ✓ Test 1 passed");
  }

  // Test 2: Whitespace with \n and \t around punctuation
  {
    console.log("  Test 2: Arbitrary whitespace (newlines/tabs) around punctuation");
    const initWhitespace = '{\n\t"type":\t"SOT",\n\t"name":\t"WhitespaceToken",\n\t"symbol":\t"WST",\n\t"decimals":\t8,\n\t"totalSupply":\t"500000",\n\t"balance":\t"500000",\n\t"owner":\t"03deadbeef"\n}';
    const initRes = callInit(initWhitespace);
    assert.strictEqual(initRes, 0, "Init with whitespace formatting must succeed");

    const callWhitespace = '{\r\n\t"to":\t "03deadbeef" ,\r\n\t"amount":\t 500000\r\n}';
    const callRes = callMethod("transfer", callWhitespace);
    assert.strictEqual(callRes, 0, "Call with whitespace formatting must succeed");
    console.log("  ✓ Test 2 passed");
  }

  // Test 3: Intentionally malformed JSON returns explicit error code (non-zero)
  {
    console.log("  Test 3: Malformed JSON returns explicit error code, not default value");
    // Malformed init
    const badInitRes = callInit('{"name": "Bad", "unclosed": ');
    assert.notStrictEqual(badInitRes, 0, "Init with malformed JSON must return non-zero error code");

    // Re-init valid so call can be tested
    callInit(JSON.stringify({
      type: "SOT",
      name: "Good",
      symbol: "GD",
      decimals: 8,
      totalSupply: 100,
      balance: 100,
      owner: "03deadbeef",
    }));

    // Malformed call args
    const badCall1 = callMethod("transfer", '{"to": "03deadbeef", "amount": }');
    assert.notStrictEqual(badCall1, 0, "Malformed args with missing value must return non-zero error code");

    const badCall2 = callMethod("transfer", 'not even json');
    assert.notStrictEqual(badCall2, 0, "Malformed non-json args must return non-zero error code");

    const badCall3 = callMethod("transfer", '{"to": "03deadbeef"');
    assert.notStrictEqual(badCall3, 0, "Unclosed JSON args must return non-zero error code");

    // Check that error event was emitted
    const errorEvent = events.find((e) => e.topic.startsWith("Error"));
    assert(errorEvent, "Must emit an Error event on malformed JSON");
    console.log("  ✓ Test 3 passed");
  }

  console.log("All Issue 2 unit tests passed successfully!");
}

runTests().catch((err) => {
  console.error("Test failed:", err);
  process.exit(1);
});
