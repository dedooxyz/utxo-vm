import { UTXOClient, EnvelopeBuilder, SimpleUTXOWallet } from "@utxo-vm/sdk";

export async function handleCall(targetSeal: string, method: string, rawArgs: string[], options: any): Promise<void> {
  const chain = options.chain || "BTC";
  const nodeUrl = options.nodeUrl || "http://127.0.0.1:9773";

  console.log(`\n⚡ [UTXO-VM Invoker] Calling method '${method}' on seal '${targetSeal}'...`);

  let argsObj: Record<string, any> = {};
  if (rawArgs.length > 0) {
    try {
      argsObj = JSON.parse(rawArgs.join(" "));
    } catch {
      argsObj = { raw: rawArgs.join(" ") };
    }
  }

  const wallet = SimpleUTXOWallet.createRandom();
  const client = new UTXOClient(chain, nodeUrl);

  const envelope = EnvelopeBuilder.buildCallEnvelope(targetSeal, method, argsObj);

  try {
    const result = await client.broadcastEnvelope(envelope, wallet);
    console.log(`\n✅ State Transition successfully submitted!`);
    console.log(`   - Transaction ID: ${result.txid}`);
    console.log(`   - Consumed Seal:  ${targetSeal}`);
    console.log(`   - New Seal:       ${result.seal}`);
    console.log(`   - Status:         ${result.status}`);
  } catch (err: any) {
    console.error(`❌ Call failed:`, err.message);
    process.exit(1);
  }
}
