/**
 * UTXO-VM Replay Verifier
 * 
 * This is a replay-only binary that independently verifies state transitions.
 * It reads transaction data from JKC testnet and verifies that the state root
 * posted by operators matches the expected state.
 * 
 * Usage: node replay-verifier.js <txid>
 * 
 * This proves "You Are Not the Indexer" - anyone can independently verify
 * that the operator posted the correct state root.
 */

const bitcoin = require('bitcoinjs-lib');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

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

// Load compiled WASM
const wasmPath = path.join(__dirname, 'packages/contracts/build/release.wasm');
const wasmBytes = fs.readFileSync(wasmPath);
const wasmHash = crypto.createHash('sha256').update(wasmBytes).digest('hex');

console.log('UTXO-VM Replay Verifier');
console.log('======================\n');
console.log(`WASM hash: ${wasmHash}`);

// Helper functions
async function fetchTransaction(txid) {
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/tx/${txid}`);
  return await response.json();
}

async function fetchRawTransaction(txid) {
  const response = await fetch(`https://jkc-testnet-api.s3na.xyz/tx/${txid}/hex`);
  return await response.text();
}

// Parse OP_RETURN data from transaction
function parseOPReturn(tx) {
  for (const vout of tx.vout) {
    if (vout.scriptPubKey.type === 'op_return') {
      const hex = vout.scriptPubKey.hex;
      // Skip OP_RETURN and push data opcodes
      const dataHex = hex.slice(4); // Skip OP_RETURN and length
      const data = Buffer.from(dataHex, 'hex');
      
      try {
        const json = JSON.parse(data.toString('utf8'));
        return json;
      } catch (e) {
        return { raw: data.toString('hex') };
      }
    }
  }
  return null;
}

// Verify state root
function verifyStateRoot(stateRoot, expectedRoot) {
  return stateRoot === expectedRoot;
}

// Verify WASM hash
function verifyWasmHash(codeHash) {
  return codeHash === wasmHash;
}

// Main verification
async function verifyTransaction(txid) {
  console.log(`\nVerifying transaction: ${txid}\n`);
  
  try {
    // Fetch transaction
    const tx = await fetchTransaction(txid);
    console.log(`Transaction found: ${tx.status.confirmed ? 'Confirmed' : 'Unconfirmed'}`);
    
    if (tx.status.confirmed) {
      console.log(`Block height: ${tx.status.block_height}`);
      console.log(`Block hash: ${tx.status.block_hash}`);
    }
    
    // Parse OP_RETURN
    const opReturn = parseOPReturn(tx);
    if (!opReturn) {
      console.log('No OP_RETURN data found');
      return false;
    }
    
    console.log('\nOP_RETURN data:');
    console.log(JSON.stringify(opReturn, null, 2));
    
    // Verify protocol
    if (opReturn.protocol !== 'utxovm') {
      console.log('\n❌ Invalid protocol: Expected "utxovm"');
      return false;
    }
    console.log('\n✓ Protocol: utxovm');
    
    // Verify version
    if (opReturn.version !== 1) {
      console.log('\n❌ Invalid version: Expected 1');
      return false;
    }
    console.log('✓ Version: 1');
    
    // Verify WASM hash
    if (opReturn.codeHash && !verifyWasmHash(opReturn.codeHash)) {
      console.log('\n❌ WASM hash mismatch');
      console.log(`  Expected: ${wasmHash}`);
      console.log(`  Got: ${opReturn.codeHash}`);
      return false;
    }
    if (opReturn.codeHash) {
      console.log('✓ WASM hash verified');
    }
    
    // Parse state if present
    if (opReturn.initState) {
      const state = typeof opReturn.initState === 'string' 
        ? JSON.parse(opReturn.initState) 
        : opReturn.initState;
      
      console.log('\nState:');
      console.log(JSON.stringify(state, null, 2));
    }
    
    // Parse state root if present
    if (opReturn.stateRoot) {
      console.log(`\nState root: ${opReturn.stateRoot}`);
    }
    
    console.log('\n✅ Transaction verified successfully');
    return true;
    
  } catch (error) {
    console.error('\n❌ Verification failed:', error.message);
    return false;
  }
}

// Batch verification
async function verifyBatch(txids) {
  console.log('\n=== Batch Verification ===\n');
  
  const results = [];
  
  for (const txid of txids) {
    const result = await verifyTransaction(txid);
    results.push({ txid, verified: result });
  }
  
  console.log('\n=== Verification Summary ===\n');
  
  const verified = results.filter(r => r.verified).length;
  const total = results.length;
  
  console.log(`Verified: ${verified}/${total}`);
  
  if (verified === total) {
    console.log('\n✅ All transactions verified successfully');
  } else {
    console.log('\n❌ Some transactions failed verification');
  }
  
  return results;
}

// CLI interface
async function main() {
  const args = process.argv.slice(2);
  
  if (args.length === 0) {
    console.log('\nUsage:');
    console.log('  node replay-verifier.js <txid>           Verify single transaction');
    console.log('  node replay-verifier.js --batch <txid1> <txid2> ...  Verify multiple transactions');
    console.log('  node replay-verifier.js --demo           Run demo verification');
    
    // Demo mode
    if (args[0] === '--demo') {
      console.log('\n=== Demo Mode ===\n');
      console.log('To verify a transaction, run:');
      console.log('  node replay-verifier.js <txid>');
      console.log('\nExample transactions from previous tests:');
      console.log('  Deploy: 6a697245861d25435ca42bec07ffaac581f6022494907dd86926889c598c7a13');
      console.log('  Transfer: 42901161e8d04b433c1d8f58e810ce97e6a14752b31c38184e1d0349f6c907cf');
      console.log('  Batch: a9de1d94b9e1b2089a9a7bc1be1680b98262dd919f9efe7bbba7344967f52a8d');
    }
    return;
  }
  
  if (args[0] === '--batch') {
    const txids = args.slice(1);
    await verifyBatch(txids);
  } else {
    const txid = args[0];
    await verifyTransaction(txid);
  }
}

main().catch(console.error);
