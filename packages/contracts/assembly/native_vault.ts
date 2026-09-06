import { HostContext } from "./env";

export class NativeVault {
  totalLockedSatoshis: u64;
  vaultOwner: string;

  constructor(owner: string) {
    this.totalLockedSatoshis = 0;
    this.vaultOwner = owner;
  }

  deposit(): u64 {
    let incomingSatoshis = HostContext.getSatoshis();
    assert(incomingSatoshis > 0, "No satoshis attached in transaction");
    this.totalLockedSatoshis += incomingSatoshis;
    HostContext.emitEvent("Deposit", "Locked " + incomingSatoshis.toString() + " base units");
    return incomingSatoshis;
  }

  withdrawToStealth(stealthAddress: string, amount: u64): bool {
    let caller = HostContext.getCaller();
    assert(caller == this.vaultOwner, "Unauthorized: Caller is not vault owner");
    assert(this.totalLockedSatoshis >= amount, "Insufficient vault balance");
    assert(amount > 0, "Amount must be greater than zero");

    this.totalLockedSatoshis -= amount;
    let ok = HostContext.stealthSettle(stealthAddress, amount);
    assert(ok, "Stealth settlement failed");

    HostContext.emitEvent("StealthWithdraw", "Sent " + amount.toString() + " units to stealth address: " + stealthAddress);
    return true;
  }

  toJson(): string {
    return "{"
      + "\"totalLockedSatoshis\":\"" + this.totalLockedSatoshis.toString() + "\","
      + "\"vaultOwner\":\"" + this.vaultOwner + "\""
      + "}";
  }
}
