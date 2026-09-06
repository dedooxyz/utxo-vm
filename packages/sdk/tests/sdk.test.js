const test = require("node:test");
const assert = require("node:assert");
const {
  SUPPORTED_CHAINS,
  getChainConfig,
  EnvelopeBuilder,
  SimpleUTXOWallet,
  UTXOClient,
  SOTClient,
  SONClient,
  UTX20Client,
  UTX721Client,
  NativeVaultClient,
  AtomicSwapClient,
  PSBTSwapBuilder,
  LightLineageVerifier,
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

test("Contract Clients instantiation (SOT, SON, UTX20, UTX721 aliases)", () => {
  const client = new UTXOClient("JKC", "http://127.0.0.1:9773");
  const sot = new SOTClient(client, "obj_sot_1");
  const son = new SONClient(client, "obj_son_1");
  const utx20 = new UTX20Client(client, "obj_utx20_1");
  const utx721 = new UTX721Client(client, "obj_utx721_1");
  const vault = new NativeVaultClient(client, "obj_vault_1");
  const swap = new AtomicSwapClient(client, "obj_swap_1");

  assert(sot instanceof SOTClient);
  assert(son instanceof SONClient);
  // Verify backward compatibility
  assert(utx20 instanceof SOTClient, "UTX20Client must extend/alias SOTClient");
  assert(utx721 instanceof SONClient, "UTX721Client must extend/alias SONClient");
  assert(vault instanceof NativeVaultClient);
  assert(swap instanceof AtomicSwapClient);
});

test("PSBTSwapBuilder: Create Order and Build 2-Party Atomic Fill Transaction", () => {
  // 1. Seller creates order for SON NFT
  const order = PSBTSwapBuilder.createSellOrder({
    chain: "JKC",
    sellObjectId: "obj_son_crypto_junk_1",
    sellSeal: "tx_nft_mint_01:0",
    assetType: "SON",
    priceSatoshis: BigInt(500000), // 0.005 JKC
    sellerAddress: "7AlicePayoutAddress",
    sellerPublicKey: "02alicepubkey",
    expiresInSeconds: 3600,
  });

  assert.strictEqual(order.chain, "JKC");
  assert.strictEqual(order.priceSatoshis, "500000");
  assert.strictEqual(order.sellSeal, "tx_nft_mint_01:0");
  assert(order.orderId.startsWith("order_"));

  // 2. Buyer fills order with payment UTXO
  const buyerPaymentUtxo = {
    txid: "tx_buyer_payment_utxo",
    vout: 1,
    satoshis: BigInt(1000000), // 0.01 JKC (enough for price 500k + dust 1000 + fee 500)
  };

  const atomicTx = PSBTSwapBuilder.buildAtomicFillTx({
    order,
    buyerAddress: "7BobAssetAddress",
    buyerPaymentUtxo,
    changeAddress: "7BobChangeAddress",
    dustSatoshis: BigInt(1000),
    estimatedFeeSatoshis: BigInt(500),
  });

  assert.strictEqual(atomicTx.orderId, order.orderId);
  // 2 inputs: Asset Seal (from Seller) + Payment UTXO (from Buyer)
  assert.strictEqual(atomicTx.inputs.length, 2);
  assert.strictEqual(atomicTx.inputs[0].txid, "tx_nft_mint_01");
  assert.strictEqual(atomicTx.inputs[0].vout, 0);
  assert.strictEqual(atomicTx.inputs[0].type, "ASSET_SEAL");
  assert.strictEqual(atomicTx.inputs[1].txid, "tx_buyer_payment_utxo");
  assert.strictEqual(atomicTx.inputs[1].type, "PAYMENT");

  // 3 outputs: Seller Payout (500k) + Buyer Asset (1000 dust) + Buyer Change (498500)
  assert.strictEqual(atomicTx.outputs.length, 3);
  assert.strictEqual(atomicTx.outputs[0].address, "7AlicePayoutAddress");
  assert.strictEqual(atomicTx.outputs[0].satoshis, BigInt(500000));
  assert.strictEqual(atomicTx.outputs[0].role, "SELLER_PAYOUT");

  assert.strictEqual(atomicTx.outputs[1].address, "7BobAssetAddress");
  assert.strictEqual(atomicTx.outputs[1].satoshis, BigInt(1000));
  assert.strictEqual(atomicTx.outputs[1].role, "BUYER_ASSET");

  assert.strictEqual(atomicTx.outputs[2].address, "7BobChangeAddress");
  assert.strictEqual(atomicTx.outputs[2].satoshis, BigInt(498500)); // 1000000 - 500000 - 1000 - 500
  assert.strictEqual(atomicTx.outputs[2].role, "BUYER_CHANGE");

  // Insufficient buyer UTXO test
  assert.throws(() => {
    PSBTSwapBuilder.buildAtomicFillTx({
      order,
      buyerAddress: "7BobAssetAddress",
      buyerPaymentUtxo: { txid: "poor_utxo", vout: 0, satoshis: BigInt(100) },
    });
  }, /Insufficient buyer payment UTXO/);
});

test("LightLineageVerifier: Verify Unbroken Single-Use Seal Lineage (Client-Side)", async () => {
  // Mock blockchain transactions
  const mockBlockchainTxs = {
    tx_deploy: {
      txid: "tx_deploy",
      vin: [{ txid: "coinbase", vout: 0 }],
      vout: [{ value: 1000, scriptpubkey_address: "alice" }],
    },
    tx_transfer_1: {
      txid: "tx_transfer_1",
      vin: [{ txid: "tx_deploy", vout: 0 }], // Spends genesis seal
      vout: [{ value: 1000, scriptpubkey_address: "bob" }],
    },
    tx_transfer_2: {
      txid: "tx_transfer_2",
      vin: [{ txid: "tx_transfer_1", vout: 0 }], // Spends seal 1
      vout: [{ value: 1000, scriptpubkey_address: "charlie" }],
    },
  };

  const mockFetchTx = async (txid) => {
    const tx = mockBlockchainTxs[txid];
    if (!tx) throw new Error(`Tx ${txid} not found on-chain`);
    return tx;
  };

  // Valid unbroken history
  const validHistory = [
    { txid: "tx_deploy", consumedSeal: "prev:0", newSeal: "tx_deploy:0", method: "deploy" },
    { txid: "tx_transfer_1", consumedSeal: "tx_deploy:0", newSeal: "tx_transfer_1:0", method: "transfer" },
    { txid: "tx_transfer_2", consumedSeal: "tx_transfer_1:0", newSeal: "tx_transfer_2:0", method: "transfer" },
  ];

  const validResult = await LightLineageVerifier.verifyObjectLineage("obj_sot_test", validHistory, undefined, mockFetchTx);
  assert.strictEqual(validResult.verified, true);
  assert.strictEqual(validResult.transitionsVerified, 3);
  assert.strictEqual(validResult.genesisSeal, "tx_deploy:0");
  assert.strictEqual(validResult.tipSeal, "tx_transfer_2:0");

  // Invalid: Discontinuous seal in history
  const brokenHistory = [
    { txid: "tx_deploy", consumedSeal: "prev:0", newSeal: "tx_deploy:0", method: "deploy" },
    { txid: "tx_transfer_2", consumedSeal: "tx_UNKNOWN:0", newSeal: "tx_transfer_2:0", method: "transfer" },
  ];
  const brokenResult = await LightLineageVerifier.verifyObjectLineage("obj_sot_test", brokenHistory, undefined, mockFetchTx);
  assert.strictEqual(brokenResult.verified, false);
  assert(brokenResult.error.includes("Seal discontinuity"));

  // Fraudulent: On-chain tx did not actually spend the claimed seal
  const fraudulentBlockchainTxs = {
    ...mockBlockchainTxs,
    tx_fake: {
      txid: "tx_fake",
      vin: [{ txid: "tx_unrelated", vout: 99 }], // Did NOT spend tx_transfer_1:0!
    },
  };
  const fraudFetchTx = async (txid) => fraudulentBlockchainTxs[txid];
  const fraudHistory = [
    { txid: "tx_deploy", consumedSeal: "prev:0", newSeal: "tx_deploy:0", method: "deploy" },
    { txid: "tx_transfer_1", consumedSeal: "tx_deploy:0", newSeal: "tx_transfer_1:0", method: "transfer" },
    { txid: "tx_fake", consumedSeal: "tx_transfer_1:0", newSeal: "tx_fake:0", method: "transfer" },
  ];
  const fraudResult = await LightLineageVerifier.verifyObjectLineage("obj_sot_test", fraudHistory, undefined, fraudFetchTx);
  assert.strictEqual(fraudResult.verified, false);
  assert(fraudResult.error.includes("Fraudulent seal"));
});
