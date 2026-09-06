import { HostContext } from "./env";

export class UTX20Token {
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

  transfer(to: string, amount: u64): UTX20Token {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only token owner can transfer");
    assert(this.balance >= amount, "Insufficient balance");
    assert(amount > 0, "Amount must be greater than zero");

    this.balance -= amount;
    HostContext.emitEvent("Transfer", "Transferred " + amount.toString() + " " + this.symbol + " to " + to);

    return new UTX20Token(this.name, this.symbol, this.decimals, this.totalSupply, to);
  }

  mint(to: string, amount: u64): UTX20Token {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only minter/owner can mint");
    assert(amount > 0, "Mint amount must be greater than zero");

    this.totalSupply += amount;
    HostContext.emitEvent("Mint", "Minted " + amount.toString() + " " + this.symbol + " to " + to);

    return new UTX20Token(this.name, this.symbol, this.decimals, this.totalSupply, to);
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
