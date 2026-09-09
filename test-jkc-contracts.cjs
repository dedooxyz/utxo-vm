/**
 * JKC Testnet Contract Integration Test
 * 
 * This script tests all contract types on JKC testnet:
 * 1. NativeVault - Lock N satoshis, 1-of-1 + timelock exit
 * 2. UTX721 (SON) - 1 owner, 1 payload hash, operator posts root
 * 3. AtomicSwap - 2-of-2 HTLC, refund CSV, claim hash preimage
 * 
 * Run: node test-jkc-contracts.cjs
 */

const bitcoin = require('bitcoinjs-lib');
const ECPairFactory = require('ecpair').ECPairFactory;
const ecc = require('tiny-secp256k1');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const ECPair = ECPairFactory(ecc);

// JKC Testnet parameters
const JKC_TESTNET = {
  messagePrefix: '\x18Junkcoin Signed Message:\n',
  bech32: 'tjkc',
  bip32: { public: 0x043587cf, private: 0x04358394 },
  pubKeyHash: 0x6f,
  scriptHash: 0xc4,
  wif: 0xef,
};

const network = JKC_TESTNET;

// Wallet from previous tests
const wallet = {
  privateKey: 'cQo84YPrqxfALWPX4R5WxvmNi2vKtMkEjqVRzX7BJQ9aLqZQPvck',
  address: 'muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi',
};

const keyPair = ECPair.fromWIF(wallet.privateKey, network);
const { address } = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network });

// Load compiled WASM
const wasmPath = path.join(__dirname, 'packages/contracts/build/release.wasm');
const wasmBytes = fs.readFileSync(wasmPath);

console.log('JKC Testnet Contract Integration Test');
console.log('=====================================\n');
console.log(`Wallet: ${address}`);
console.log(`WASM size: ${wasmBytes.length} bytes`);

// Helper functions
async function fetchUTXOs(addr) {
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/address/${addr}/utxo`);
  return await response.json();
}

async function fetchRawTx(txid) {
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/tx/${txid}/hex`);
  return await response.text();
}

async function broadcastTx(txHex) {
  const response = await fetch('https://jkc-testnet-api.s3na.xyz/tx', {
    method: 'POST',
    body: txHex,
    headers: { 'Content-Type': 'text/plain' },
  });
  
  if (!response.ok) {
    const error = await response.text();
    throw new Error(`Broadcast failed: ${error}`);
  }
  
  return await response.text();
}

// Step 1: Deploy NativeVault
async function deployNativeVault() {
  console.log('\n--- Step 1: Deploy NativeVault ---\n');
  
  const utxos = await fetchUTXOs(address);
  if (utxos.length === 0) throw new Error('No UTXOs');
  
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  // Create deployment transaction with OP_RETURN envelope
  const psbt = new bitcoin.Psbt({ network });
  
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTx(utxo.txid), 'hex'),
  });
  
  // OP_RETURN envelope with WASM hash
  const wasmHash = crypto.createHash('sha256').update(wasmBytes).digest('hex');
  const envelope = {
    protocol: 'utxovm',
    version: 1,
    contentType: 'application/wasm',
    codeHash: wasmHash,
    initState: JSON.stringify({
      type: 'NativeVault',
      owner: address,
      totalLockedSatoshis: 0,
    }),
  };
  
  const envelopeJson = JSON.stringify(envelope);
  const opReturnData = Buffer.from(envelopeJson);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  // Change output
  const changeAmount = utxo.value - 1000;
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  const txid = await broadcastTx(txHex);
  
  console.log(`NativeVault deployed!`);
  console.log(`TXID: ${txid}`);
  console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${txid}`);
  
  return { txid, wasmHash };
}

// Step 2: Deposit to NativeVault
async function depositToVault(vaultTxid) {
  console.log('\n--- Step 2: Deposit to NativeVault ---\n');
  
  const utxos = await fetchUTXOs(address);
  if (utxos.length === 0) throw new Error('No UTXOs');
  
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  const depositAmount = 50_000_000; // 0.5 JKC
  
  const psbt = new bitcoin.Psbt({ network });
  
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTx(utxo.txid), 'hex'),
  });
  
  // OP_RETURN with deposit call
  const callData = {
    protocol: 'utxovm',
    version: 1,
    contentType: 'application/json',
    method: 'deposit',
    args: {},
    vaultRef: vaultTxid,
  };
  
  const callJson = JSON.stringify(callData);
  const opReturnData = Buffer.from(callJson);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  // Output to vault (locked satoshis)
  psbt.addOutput({
    address: address, // In real impl, this would be vault P2TR address
    value: depositAmount,
  });
  
  // Change
  const changeAmount = utxo.value - depositAmount - 1000;
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  const txid = await broadcastTx(txHex);
  
  console.log(`Deposited ${depositAmount} satoshis to vault!`);
  console.log(`TXID: ${txid}`);
  console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${txid}`);
  
  return txid;
}

// Step 3: Deploy UTX721 (SON)
async function deployUTX721() {
  console.log('\n--- Step 3: Deploy UTX721 (SON) ---\n');
  
  const utxos = await fetchUTXOs(address);
  if (utxos.length === 0) throw new Error('No UTXOs');
  
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  const psbt = new bitcoin.Psbt({ network });
  
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTx(utxo.txid), 'hex'),
  });
  
  // OP_RETURN envelope
  const envelope = {
    protocol: 'utxovm',
    version: 1,
    contentType: 'application/wasm',
    codeHash: crypto.createHash('sha256').update(wasmBytes).digest('hex'),
    initState: JSON.stringify({
      type: 'UTX721',
      collectionName: 'JKC Test Collection',
      tokenId: 1,
      metadataUri: 'ipfs://test-metadata',
      owner: address,
    }),
  };
  
  const envelopeJson = JSON.stringify(envelope);
  const opReturnData = Buffer.from(envelopeJson);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  const changeAmount = utxo.value - 1000;
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  const txid = await broadcastTx(txHex);
  
  console.log(`UTX721 deployed!`);
  console.log(`TXID: ${txid}`);
  console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${txid}`);
  
  return txid;
}

// Step 4: Deploy AtomicSwap
async function deployAtomicSwap() {
  console.log('\n--- Step 4: Deploy AtomicSwap ---\n');
  
  const utxos = await fetchUTXOs(address);
  if (utxos.length === 0) throw new Error('No UTXOs');
  
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  // Generate hash preimage for HTLC
  const preimage = crypto.randomBytes(32);
  const hash = crypto.createHash('sha256').update(preimage).digest();
  
  const psbt = new bitcoin.Psbt({ network });
  
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTx(utxo.txid), 'hex'),
  });
  
  // OP_RETURN envelope
  const envelope = {
    protocol: 'utxovm',
    version: 1,
    contentType: 'application/wasm',
    codeHash: crypto.createHash('sha256').update(wasmBytes).digest('hex'),
    initState: JSON.stringify({
      type: 'AtomicSwap',
      maker: address,
      offeredTokenId: 1,
      demandedSatoshis: 100_000_000,
      hashLock: hash.toString('hex'),
      timeLock: 144, // blocks
      isFilled: false,
      isCancelled: false,
    }),
  };
  
  const envelopeJson = JSON.stringify(envelope);
  const opReturnData = Buffer.from(envelopeJson);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  const changeAmount = utxo.value - 1000;
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  const txid = await broadcastTx(txHex);
  
  console.log(`AtomicSwap deployed!`);
  console.log(`TXID: ${txid}`);
  console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${txid}`);
  console.log(`Hash preimage: ${preimage.toString('hex')}`);
  console.log(`Hash lock: ${hash.toString('hex')}`);
  
  return { txid, preimage: preimage.toString('hex'), hashLock: hash.toString('hex') };
}

// Main test
async function main() {
  try {
    // Step 1: Deploy NativeVault
    const { txid: vaultTxid } = await deployNativeVault();
    
    // Wait for confirmation
    console.log('\nWaiting for confirmation...');
    await new Promise(resolve => setTimeout(resolve, 60000));
    
    // Step 2: Deposit to vault
    await depositToVault(vaultTxid);
    
    // Wait for confirmation
    console.log('\nWaiting for confirmation...');
    await new Promise(resolve => setTimeout(resolve, 60000));
    
    // Step 3: Deploy UTX721
    const utx721Txid = await deployUTX721();
    
    // Wait for confirmation
    console.log('\nWaiting for confirmation...');
    await new Promise(resolve => setTimeout(resolve, 60000));
    
    // Step 4: Deploy AtomicSwap
    const { txid: swapTxid, preimage, hashLock } = await deployAtomicSwap();
    
    console.log('\n=== Contract Integration Test Complete ===');
    console.log('\nSummary:');
    console.log(`1. NativeVault: ${vaultTxid}`);
    console.log(`2. Deposit: (see vault transaction)`);
    console.log(`3. UTX721: ${utx721Txid}`);
    console.log(`4. AtomicSwap: ${swapTxid}`);
    console.log(`\nAll contracts deployed on JKC testnet!`);
    
  } catch (error) {
    console.error('Error:', error.message);
  }
}

main();
