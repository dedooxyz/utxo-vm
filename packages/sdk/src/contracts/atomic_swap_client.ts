import { UTXOClient, BroadcastResult } from "../client";
import { UTXOWalletSigner } from "../wallet";
import { EnvelopeBuilder } from "../envelope";

export class AtomicSwapClient {
  private client: UTXOClient;
  private orderId: string;

  constructor(client: UTXOClient, orderId: string) {
    this.client = client;
    this.orderId = orderId;
  }

  async getState(): Promise<any> {
    return await this.client.getObjectState(this.orderId);
  }

  async fill(takerWallet: UTXOWalletSigner, paymentSatoshis: bigint): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.orderId,
      "fill",
      { taker: takerWallet.getPublicKey() }
    );
    return await this.client.broadcastEnvelope(envelope, takerWallet, paymentSatoshis);
  }

  async cancel(makerWallet: UTXOWalletSigner): Promise<BroadcastResult> {
    const envelope = EnvelopeBuilder.buildCallEnvelope(
      this.orderId,
      "cancel",
      {}
    );
    return await this.client.broadcastEnvelope(envelope, makerWallet);
  }
}
