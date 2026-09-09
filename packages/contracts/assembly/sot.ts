import { HostContext } from "./env";

/**
 * Smart Object Token (SOT)
 * Canonical Fungible Token standard for UTXO-VM
 */
export class SmartObjectToken {
  name: string;
  symbol: string;
  decimals: u8;
  totalSupply: u64;
  balance: u64;
  owner: string;

  constructor(name: string, symbol: string, decimals: u8, initialSupply: u64, owner: string) {
    this.name = name;
    this.symbol = symbol;
    this.decimals = decimals;
    this.totalSupply = initialSupply;
    this.balance = initialSupply;
    this.owner = owner;
  }

  transfer(to: string, amount: u64): SmartObjectToken {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only token owner can transfer");
    // UTXO model: each seal holds the entire balance. Full transfer only.
    assert(amount == this.balance, "UTXO transfer requires full balance: amount must equal balance");
    assert(amount > 0, "Amount must be greater than zero");

    this.balance = 0;
    HostContext.emitEvent("Transfer", "Transferred " + amount.toString() + " " + this.symbol + " to " + to);

    // Recipient gets a new token with balance == amount (not totalSupply).
    let newToken = new SmartObjectToken(this.name, this.symbol, this.decimals, this.totalSupply, to);
    newToken.balance = amount;
    return newToken;
  }

  mint(to: string, amount: u64): SmartObjectToken {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only minter/owner can mint");
    assert(amount > 0, "Mint amount must be greater than zero");

    this.totalSupply += amount;
    HostContext.emitEvent("Mint", "Minted " + amount.toString() + " " + this.symbol + " to " + to);

    // New token for recipient has balance == amount (the minted portion), not totalSupply
    let newToken = new SmartObjectToken(this.name, this.symbol, this.decimals, this.totalSupply, to);
    newToken.balance = amount;
    return newToken;
  }

  burn(amount: u64): void {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only token owner can burn");
    assert(this.balance >= amount, "Insufficient balance to burn");
    assert(amount > 0, "Burn amount must be greater than zero");

    this.balance -= amount;
    this.totalSupply -= amount;
    HostContext.emitEvent("Burn", "Burned " + amount.toString() + " " + this.symbol);
  }

  toJson(): string {
    return "{"
      + "\"name\":\"" + this.name + "\","
      + "\"symbol\":\"" + this.symbol + "\","
      + "\"decimals\":" + this.decimals.toString() + ","
      + "\"totalSupply\":\"" + this.totalSupply.toString() + "\","
      + "\"balance\":\"" + this.balance.toString() + "\","
      + "\"owner\":\"" + this.owner + "\""
      + "}";
  }
}

export { SmartObjectToken as SOTToken };
