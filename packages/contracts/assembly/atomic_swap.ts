import { HostContext } from "./env";

export class AtomicSwapOrder {
  maker: string;
  offeredTokenId: u64;
  demandedSatoshis: u64;
  isFilled: bool;
  isCancelled: bool;

  constructor(maker: string, offeredTokenId: u64, demandedSatoshis: u64) {
    this.maker = maker;
    this.offeredTokenId = offeredTokenId;
    this.demandedSatoshis = demandedSatoshis;
    this.isFilled = false;
    this.isCancelled = false;
  }

  fill(taker: string): bool {
    assert(!this.isFilled, "Order already filled");
    assert(!this.isCancelled, "Order is cancelled");
    let paidSatoshis = HostContext.getSatoshis();
    assert(paidSatoshis >= this.demandedSatoshis, "Insufficient payment in satoshis");

    this.isFilled = true;
    HostContext.emitEvent("SwapFilled", "Order filled by " + taker + " for " + this.demandedSatoshis.toString() + " satoshis");
    return true;
  }

  cancel(): bool {
    let caller = HostContext.getCaller();
    assert(caller == this.maker, "Unauthorized: Only order maker can cancel");
    assert(!this.isFilled, "Cannot cancel: order already filled");
    assert(!this.isCancelled, "Order already cancelled");

    this.isCancelled = true;
    HostContext.emitEvent("SwapCancelled", "Order for token " + this.offeredTokenId.toString() + " cancelled by maker");
    return true;
  }

  toJson(): string {
    return "{"
      + "\"maker\":\"" + this.maker + "\","
      + "\"offeredTokenId\":\"" + this.offeredTokenId.toString() + "\","
      + "\"demandedSatoshis\":\"" + this.demandedSatoshis.toString() + "\","
      + "\"isFilled\":" + (this.isFilled ? "true" : "false") + ","
      + "\"isCancelled\":" + (this.isCancelled ? "true" : "false")
      + "}";
  }
}
