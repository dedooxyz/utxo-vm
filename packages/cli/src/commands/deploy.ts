import fs from "fs";
import { UTXOClient, EnvelopeBuilder, SimpleUTXOWallet } from "@utxo-vm/sdk";

export async function handleDeploy(wasmPath: string, options: any): Promise<void> {
  const chain = options.chain || "BTC";
  const nodeUrl = options.nodeUrl || "http://127.0.0.1:9773";
  const satoshis = BigInt(options.satoshis || "10000");

  console.log(`\n🚀 [UTXO-VM Deployer] Deploying contract on ${chain}...`);
  console.log(`   Node URL: ${nodeUrl}`);
  console.log(`   WASM File: ${wasmPath}`);

  if (!fs.existsSync(wasmPath)) {
    console.error(`❌ Error: WASM file not found: ${wasmPath}`);
    process.exit(1);
  }

  const wasmBytes = new Uint8Array(fs.readFileSync(wasmPath));
  const wallet = SimpleUTXOWallet.createRandom();
  const client = new UTXOClient(chain, nodeUrl);

  const envelope = EnvelopeBuilder.buildDeployEnvelope(wasmBytes, {
    owner: wallet.getPublicKey(),
    deployTimestamp: Date.now(),
  });

  try {
    const result = await client.broadcastEnvelope(envelope, wallet, satoshis);
    console.log(`\n✅ Smart Object successfully deployed!`);
    console.log(`   - Transaction ID: ${result.txid}`);
    console.log(`   - Initial Seal:   ${result.seal}`);
    console.log(`   - Owner PubKey:   ${wallet.getPublicKey()}`);
    console.log(`   - Status:         ${result.status}`);
  } catch (err: any) {
    console.error(`❌ Deployment failed:`, err.message);
    process.exit(1);
  }
}
