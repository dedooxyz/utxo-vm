import { UTXOClient, BroadcastResult } from "../client";
import { UTXOWalletSigner } from "../wallet";
import { EnvelopeBuilder } from "../envelope";

export class NativeVaultClient {
  private client: UTXOClient;
  private vaultId: string;

  constructor(client: UTXOClient, vaultId: string) {
    this.client = client;
    this.vaultId = vaultId;
  }

  async getState(): Promise<any> {
    return await this.client.getObjectState(this.vaultId);
  }

  async deposit(satoshis: bigint, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.vaultId,
      "deposit",
      {}
    );
    return await this.client.broadcastEnvelope(envelope, wallet, satoshis);
  }

  async withdrawToStealth(stealthAddress: string, amount: bigint, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.vaultId,
      "withdrawToStealth",
      { stealthAddress, amount: amount.toString() }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }
}
