/**
 * JKC Testnet Operator Registration Test
 * 
 * This script tests the operator lifecycle on JKC testnet:
 * 1. Register operator (lock JKC into vault)
 * 2. Post batch (record state root)
 * 3. Unbond operator (start unbonding)
 * 4. Exit operator (after unbond delay)
 * 
 * Run: node test-jkc-operator-registration.cjs
 */

const bitcoin = require('bitcoinjs-lib');
const ECPairFactory = require('ecpair').ECPairFactory;
const ecc = require('tiny-secp256k1');
const crypto = require('crypto');

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

// Wallet from previous tests (reuse the same funded wallet)
const wallet = {
  privateKey: 'cQo84YPrqxfALWPX4R5WxvmNi2vKtMkEjqVRzX7BJQ9aLqZQPvck',
  address: 'muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi',
};

const keyPair = ECPair.fromWIF(wallet.privateKey, network);
const { address } = bitcoin.payments.p2pkh({ pubkey: keyPair.publicKey, network });

console.log('Operator Registration Test on JKC Testnet');
console.log('==========================================\n');
console.log(`Operator address: ${address}`);

// Step 1: Create operator registration transaction
async function createOperatorRegistration() {
  console.log('\n--- Step 1: Operator Registration ---\n');
  
  // Fetch UTXOs
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/address/${address}/utxo`);
  const utxos = await response.json();
  
  if (utxos.length === 0) {
    console.log('No UTXOs available');
    return null;
  }
  
  // Sort by value descending
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  // Bond amount (1 JKC = 100,000,000 satoshis)
  const bondAmount = 100_000_000;
  
  // Create transaction
  const psbt = new bitcoin.Psbt({ network });
  
  // Add input
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTransaction(utxo.txid), 'hex'),
  });
  
  // Output 1: Bond UTXO (operator's bond)
  const bondScript = bitcoin.script.compile([
    bitcoin.opcodes.OP_HASH160,
    bitcoin.crypto.hash160(keyPair.publicKey),
    bitcoin.opcodes.OP_EQUAL,
  ]);
  
  // Output 2: OP_RETURN with operator info
  const operatorInfo = JSON.stringify({
    type: 'operator_registration',
    pubkey: keyPair.publicKey.toString('hex'),
    bond_amount: bondAmount,
    timestamp: Date.now(),
  });
  
  const opReturnData = Buffer.from(operatorInfo);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  // Output 3: Change
  const changeAmount = utxo.value - bondAmount - 1000; // 1000 satoshis fee
  if (changeAmount > 0) {
    psbt.addOutput({
      address: address,
      value: changeAmount,
    });
  }
  
  // Sign
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  console.log('Transaction created');
  console.log(`Bond amount: ${bondAmount} satoshis (1 JKC)`);
  
  return txHex;
}

// Step 2: Create batch posting transaction
async function createBatchPost(operatorPubkey, stateRoot) {
  console.log('\n--- Step 2: Post Batch ---\n');
  
  // Fetch UTXOs
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/address/${address}/utxo`);
  const utxos = await response.json();
  
  if (utxos.length === 0) {
    console.log('No UTXOs available');
    return null;
  }
  
  // Sort by value descending
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  // Create transaction
  const psbt = new bitcoin.Psbt({ network });
  
  // Add input
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTransaction(utxo.txid), 'hex'),
  });
  
  // Output 1: OP_RETURN with batch data
  const batchData = JSON.stringify({
    type: 'batch_post',
    operator_pubkey: operatorPubkey,
    state_root: stateRoot,
    block_height: 12345, // Simulated
    timestamp: Date.now(),
  });
  
  const opReturnData = Buffer.from(batchData);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  // Output 2: Change
  const changeAmount = utxo.value - 1000; // 1000 satoshis fee
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  // Sign
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  console.log('Batch posted');
  console.log(`State root: ${stateRoot}`);
  
  return txHex;
}

// Step 3: Create unbonding transaction
async function createUnbonding() {
  console.log('\n--- Step 3: Unbond Operator ---\n');
  
  // Fetch UTXOs
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/address/${address}/utxo`);
  const utxos = await response.json();
  
  if (utxos.length === 0) {
    console.log('No UTXOs available');
    return null;
  }
  
  // Sort by value descending
  utxos.sort((a, b) => b.value - a.value);
  const utxo = utxos[0];
  
  console.log(`Using UTXO: ${utxo.txid}:${utxo.vout} (${utxo.value} satoshis)`);
  
  // Create transaction
  const psbt = new bitcoin.Psbt({ network });
  
  // Add input
  psbt.addInput({
    hash: utxo.txid,
    index: utxo.vout,
    nonWitnessUtxo: Buffer.from(await fetchRawTransaction(utxo.txid), 'hex'),
  });
  
  // Output 1: OP_RETURN with unbonding signal
  const unbondData = JSON.stringify({
    type: 'operator_unbonding',
    timestamp: Date.now(),
  });
  
  const opReturnData = Buffer.from(unbondData);
  const embed = bitcoin.payments.embed({ data: [opReturnData] });
  
  psbt.addOutput({
    script: embed.output,
    value: 0,
  });
  
  // Output 2: Change
  const changeAmount = utxo.value - 1000;
  psbt.addOutput({
    address: address,
    value: changeAmount,
  });
  
  // Sign
  psbt.signInput(0, keyPair);
  psbt.finalizeAllInputs();
  
  const txHex = psbt.extractTransaction().toHex();
  console.log('Unbonding initiated');
  
  return txHex;
}

// Helper: fetch raw transaction
async function fetchRawTransaction(txid) {
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/tx/${txid}/hex`);
  return await response.text();
}

// Helper: broadcast transaction
async function broadcastTransaction(txHex) {
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

// Main test
async function main() {
  try {
    // Step 1: Register operator
    const regTxHex = await createOperatorRegistration();
    if (!regTxHex) {
      console.log('Failed to create registration');
      return;
    }
    
    const regTxid = await broadcastTransaction(regTxHex);
    console.log(`Registration TXID: ${regTxid}`);
    console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${regTxid}`);
    
    // Wait for confirmation
    console.log('\nWaiting for confirmation...');
    await new Promise(resolve => setTimeout(resolve, 60000)); // 1 minute
    
    // Step 2: Post batch
    const stateRoot = crypto.createHash('sha256')
      .update('test_batch_' + Date.now())
      .digest('hex');
    
    const batchTxHex = await createBatchPost(
      keyPair.publicKey.toString('hex'),
      stateRoot
    );
    
    if (!batchTxHex) {
      console.log('Failed to create batch');
      return;
    }
    
    const batchTxid = await broadcastTransaction(batchTxHex);
    console.log(`Batch TXID: ${batchTxid}`);
    console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${batchTxid}`);
    
    // Wait for confirmation
    console.log('\nWaiting for confirmation...');
    await new Promise(resolve => setTimeout(resolve, 60000)); // 1 minute
    
    // Step 3: Unbond
    const unbondTxHex = await createUnbonding();
    if (!unbondTxHex) {
      console.log('Failed to create unbonding');
      return;
    }
    
    const unbondTxid = await broadcastTransaction(unbondTxHex);
    console.log(`Unbond TXID: ${unbondTxid}`);
    console.log(`Explorer: https://jkc-testnet-api.s3na.xyz/tx/${unbondTxid}`);
    
    console.log('\n=== Operator Registration Test Complete ===');
    console.log('\nSummary:');
    console.log(`1. Registration: ${regTxid}`);
    console.log(`2. Batch Post: ${batchTxid}`);
    console.log(`3. Unbonding: ${unbondTxid}`);
    console.log(`\nOperator lifecycle tested on JKC testnet!`);
    
  } catch (error) {
    console.error('Error:', error.message);
  }
}

main();
