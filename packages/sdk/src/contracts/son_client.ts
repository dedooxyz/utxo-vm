import { UTXOClient, BroadcastResult } from "../client";
import { UTXOWalletSigner } from "../wallet";
import { EnvelopeBuilder } from "../envelope";

/**
 * SONClient - Client for Smart Object NFT (SON) standard contracts
 */
export class SONClient {
  public client: UTXOClient;
  public tokenId: string;

  constructor(client: UTXOClient, tokenId: string) {
    this.client = client;
    this.tokenId = tokenId;
  }

  async getState(): Promise<any> {
    return await this.client.getObjectState(this.tokenId);
  }

  async transfer(to: string, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "transfer",
      { to }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }

  async setMetadataUri(newUri: string, wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "setMetadataUri",
      { newUri }
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }

  async burn(wallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.tokenId,
      "burn",
      {}
    );
    return await this.client.broadcastEnvelope(envelope, wallet);
  }
}

export { SONClient as SmartObjectNFTClient };
