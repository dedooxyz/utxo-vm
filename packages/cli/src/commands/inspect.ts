import { UTXOClient } from "@utxo-vm/sdk";

export async function handleInspect(objectIdOrSeal: string, options: any): Promise<void> {
  const chain = options.chain || "BTC";
  const nodeUrl = options.nodeUrl || "http://127.0.0.1:9773";

  console.log(`\n🔍 [UTXO-VM Inspector] Querying state for: ${objectIdOrSeal}...`);

  const client = new UTXOClient(chain, nodeUrl);

  try {
    const state = await client.getObjectState(objectIdOrSeal);
    console.log(`\n📊 Smart Object State:`);
    console.log(`   - Object ID:       ${state.objectId}`);
    console.log(`   - Current Seal:    ${state.seal}`);
    console.log(`   - Code Hash:       ${state.codeHash}`);
    console.log(`   - Locked Satoshis: ${state.satoshis}`);
    console.log(`   - Owner PubKey:    ${state.owner}`);
    console.log(`   - Updated Block:   ${state.updatedAtBlock}`);
    console.log(`   - State Payload:   ${JSON.stringify(state.stateData, null, 2)}`);

    try {
      const history = await client.getObjectHistory(state.objectId);
      if (history.length > 0) {
        console.log(`\n📜 State Transition History (${history.length} transitions):`);
        for (let i = 0; i < history.length; i++) {
          const h = history[i];
          console.log(`   [#${i + 1}] TxID: ${h.txid} | ${h.consumedSeal} -> ${h.newSeal} (Method: ${h.method})`);
        }
      }
    } catch {
      // History optional
    }
  } catch (err: any) {
    console.error(`❌ Inspection failed:`, err.response?.data?.error || err.message);
    process.exit(1);
  }
}
