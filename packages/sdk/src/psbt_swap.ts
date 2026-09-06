export interface AtomicSellOrder {
  orderId: string;
  chain: string;
  sellObjectId: string;
  sellSeal: string; // txid:vout
  assetType: "SOT" | "SON" | "CUSTOM";
  priceSatoshis: string;
  sellerAddress: string;
  sellerPublicKey: string;
  createdAt: number;
  expiresAt?: number;
}

export interface BuyerPaymentInput {
  txid: string;
  vout: number;
  satoshis: bigint;
  scriptPubKeyHex?: string;
}

export interface AtomicSwapTransactionPayload {
  orderId: string;
  inputs: Array<{ txid: string; vout: number; type: "ASSET_SEAL" | "PAYMENT" }>;
  outputs: Array<{
    address: string;
    satoshis: bigint;
    role: "SELLER_PAYOUT" | "BUYER_ASSET" | "BUYER_CHANGE";
  }>;
  newAssetSeal: string;
}

export class PSBTSwapBuilder {
  /**
   * Create an atomic sell order offer
   */
  static createSellOrder(params: {
    chain?: string;
    sellObjectId: string;
    sellSeal: string;
    assetType?: "SOT" | "SON" | "CUSTOM";
    priceSatoshis: bigint | string;
    sellerAddress: string;
    sellerPublicKey: string;
    expiresInSeconds?: number;
  }): AtomicSellOrder {
    const now = Date.now();
    const orderId =
      "order_" + Math.random().toString(16).slice(2, 10) + Math.random().toString(16).slice(2, 10);
    return {
      orderId,
      chain: params.chain || "JKC",
      sellObjectId: params.sellObjectId,
      sellSeal: params.sellSeal,
      assetType: params.assetType || "SON",
      priceSatoshis: params.priceSatoshis.toString(),
      sellerAddress: params.sellerAddress,
      sellerPublicKey: params.sellerPublicKey,
      createdAt: now,
      expiresAt: params.expiresInSeconds ? now + params.expiresInSeconds * 1000 : undefined,
    };
  }

  /**
   * Assemble an atomic 2-party fill transaction matching the seller terms
   */
  static buildAtomicFillTx(params: {
    order: AtomicSellOrder;
    buyerAddress: string;
    buyerPaymentUtxo: BuyerPaymentInput;
    changeAddress?: string;
    dustSatoshis?: bigint;
    estimatedFeeSatoshis?: bigint;
  }): AtomicSwapTransactionPayload {
    const { order, buyerAddress, buyerPaymentUtxo } = params;
    const price = BigInt(order.priceSatoshis);
    const dust = params.dustSatoshis || BigInt(1000);
    const fee = params.estimatedFeeSatoshis || BigInt(500);

    const requiredSatoshis = price + dust + fee;
    if (buyerPaymentUtxo.satoshis < requiredSatoshis) {
      throw new Error(
        `Insufficient buyer payment UTXO. Has ${buyerPaymentUtxo.satoshis}, requires ${requiredSatoshis} (Price: ${price}, Dust: ${dust}, Fee: ${fee})`
      );
    }

    const [sealTxid, sealVoutStr] = order.sellSeal.split(":");
    const sealVout = parseInt(sealVoutStr, 10);

    const change = buyerPaymentUtxo.satoshis - requiredSatoshis;

    const inputs = [
      { txid: sealTxid, vout: sealVout, type: "ASSET_SEAL" as const },
      { txid: buyerPaymentUtxo.txid, vout: buyerPaymentUtxo.vout, type: "PAYMENT" as const },
    ];

    const outputs: Array<{
      address: string;
      satoshis: bigint;
      role: "SELLER_PAYOUT" | "BUYER_ASSET" | "BUYER_CHANGE";
    }> = [
      // Output 0: Payout to seller
      { address: order.sellerAddress, satoshis: price, role: "SELLER_PAYOUT" },
      // Output 1: Transferred Asset seal to buyer
      { address: buyerAddress, satoshis: dust, role: "BUYER_ASSET" },
    ];

    if (change > BigInt(0)) {
      outputs.push({
        address: params.changeAddress || buyerAddress,
        satoshis: change,
        role: "BUYER_CHANGE",
      });
    }

    return {
      orderId: order.orderId,
      inputs,
      outputs,
      newAssetSeal: "pending:1",
    };
  }

  /**
   * Verify an order against an Electrs endpoint to ensure the seal has not already been spent
   */
  static async verifyOrderValidity(
    order: AtomicSellOrder,
    electrsUrl: string
  ): Promise<{ valid: boolean; reason?: string }> {
    try {
      if (order.expiresAt && Date.now() > order.expiresAt) {
        return { valid: false, reason: "Order has expired" };
      }

      const [txid, voutStr] = order.sellSeal.split(":");
      const vout = parseInt(voutStr, 10);

      const resp = await fetch(`${electrsUrl}/tx/${txid}/outspend/${vout}`);
      if (!resp.ok) {
        return { valid: false, reason: `Electrs outspend check failed: ${resp.status}` };
      }
      const outspend = (await resp.json()) as any;
      if (outspend.spent) {
        return {
          valid: false,
          reason: `Sell seal ${order.sellSeal} has already been spent in tx ${outspend.txid}`,
        };
      }

      return { valid: true };
    } catch (err: any) {
      return { valid: false, reason: `Verification error: ${err.message}` };
    }
  }
}
