const test = require("node:test");
const assert = require("node:assert");
const {
  SUPPORTED_CHAINS,
  getChainConfig,
  EnvelopeBuilder,
  SimpleUTXOWallet,
  UTXOClient,
  UTX20Client,
  UTX721Client,
  NativeVaultClient,
  AtomicSwapClient,
} = require("../dist/index.js");

test("SDK Multi-Chain Configuration", () => {
  assert.strictEqual(SUPPORTED_CHAINS.BTC.ticker, "BTC");
  assert.strictEqual(SUPPORTED_CHAINS.LTC.ticker, "LTC");
  assert.strictEqual(SUPPORTED_CHAINS.DOGE.ticker, "DOGE");
  assert.strictEqual(SUPPORTED_CHAINS.JKC.ticker, "JKC");
  assert.strictEqual(SUPPORTED_CHAINS.BEL.ticker, "BEL");

  const jkc = getChainConfig("JKC");
  assert.strictEqual(jkc.name, "Junkcoin");
  assert.strictEqual(jkc.p2pkhPrefix, 0x10);

  const ltc = getChainConfig("Litecoin");
  assert.strictEqual(ltc.ticker, "LTC");
});

test("EnvelopeBuilder: Deploy and Call Envelopes", () => {
  const dummyWasm = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
  const deployEnv = EnvelopeBuilder.buildDeployEnvelope(dummyWasm, { name: "TestToken" });

  assert.strictEqual(deployEnv.protocol, "utxovm");
  assert.strictEqual(deployEnv.version, 1);
  assert.strictEqual(deployEnv.contentType, "application/wasm");
  assert.deepStrictEqual(deployEnv.payload, dummyWasm);

  const callEnv = EnvelopeBuilder.buildCallEnvelope("tx_prev:0", "transfer", { to: "bob", amount: "50" });
  assert.strictEqual(callEnv.protocol, "utxovm");
  assert.strictEqual(callEnv.contentType, "application/json");

  // Serialization & Deserialization
  const serialized = EnvelopeBuilder.serializeTaprootEnvelope(callEnv);
  assert(serialized.length > 0);
  assert.strictEqual(serialized[0], 0x00); // OP_FALSE
  assert.strictEqual(serialized[1], 0x63); // OP_IF
  assert.strictEqual(serialized[serialized.length - 1], 0x68); // OP_ENDIF

  const deserialized = EnvelopeBuilder.deserializeTaprootEnvelope(serialized);
  assert(deserialized !== null);
  assert.strictEqual(deserialized.protocol, "utxovm");
});

test("Wallet: SimpleUTXOWallet & Stealth Derivation", () => {
  const wallet = SimpleUTXOWallet.createRandom();
  assert(wallet.getPublicKey().startsWith("02"));
  assert(wallet.getStealthAddress().startsWith("mweb1qq"));

  const stealthDerivation = wallet.deriveStealthPaymentAddress("02aabbcc", "02ddeeff");
  assert(stealthDerivation.ephemeralPubKey.startsWith("02"));
  assert(stealthDerivation.stealthAddress.startsWith("stealth_"));
});

test("Contract Clients instantiation", () => {
  const client = new UTXOClient("BTC", "http://127.0.0.1:9773");
  const utx20 = new UTX20Client(client, "obj_utx20_1");
  const utx721 = new UTX721Client(client, "obj_utx721_1");
  const vault = new NativeVaultClient(client, "obj_vault_1");
  const swap = new AtomicSwapClient(client, "obj_swap_1");

  assert(utx20 instanceof UTX20Client);
  assert(utx721 instanceof UTX721Client);
  assert(vault instanceof NativeVaultClient);
  assert(swap instanceof AtomicSwapClient);
});
