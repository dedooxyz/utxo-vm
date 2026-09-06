import { UTXOClient, BroadcastResult } from "../client";
import { UTXOWalletSigner } from "../wallet";
import { EnvelopeBuilder } from "../envelope";

/**
 * SOTClient - Client for Smart Object Token (SOT) standard contracts
 */
export class SOTClient {
  public client: UTXOClient;
  public tokenId: string;

  constructor(client: UTXOClient, tokenId: string) {
    this.client = client;
    this.tokenId = tokenId;
  }

  async getState(): Promise<any> {
    return await this.client.getObjectState(this.tokenId);
  }

  async transfer(to: string, amount: bigint, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "transfer",
      { to, amount: amount.toString() }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }

  async mint(to: string, amount: bigint, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "mint",
      { to, amount: amount.toString() }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }

  async burn(amount: bigint, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "burn",
      { amount: amount.toString() }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }
}

export { SOTClient as SmartObjectTokenClient };
