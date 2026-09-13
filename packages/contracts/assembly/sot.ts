import { HostContext } from "./env";
import { escapeJsonString } from "./json";

/**
 * Smart Object Token (SOT)
 * Canonical Fungible Token standard for UTXO-VM
 *
 * NOTE ON TOTAL SUPPLY (Issue 3 / Option (a) Design Specification):
 * In UTXO-VM's single-use-seal architecture, smart objects are bound to individual
 * unspent transaction outputs (`location = txid:vout`). There is NO global account
 * balance dictionary stored or mutated on L1.
 *
 * Consequently, `localSupply` (aliased as `totalSupply` for backward compatibility)
 * stored on an individual holder's seal represents the local issuance snapshot known
 * to that specific seal at the time of creation/minting. It is intentionally per-holder/local.
 * When other token holders mint or burn tokens in disjoint transactions, this holder's
 * seal state remains unchanged (L1 UTXOs cannot be mutated without a spend transaction).
 *
 * The canonical source of truth for global circulating supply is computed and indexed
 * by the aggregation layer / indexer (utxo-vmd attestation state tree) or maintained
 * on a dedicated genesis issuer seal. Callers and clients requiring network-wide
 * circulating supply must query the indexer/attestation state rather than relying
 * on individual holder seal state.
 */
export class SmartObjectToken {
  name: string;
  symbol: string;
  decimals: u8;
  localSupply: u64;
  balance: u64;
  owner: string;

  get totalSupply(): u64 {
    return this.localSupply;
  }
  set totalSupply(val: u64) {
    this.localSupply = val;
  }

  constructor(name: string, symbol: string, decimals: u8, initialSupply: u64, owner: string) {
    this.name = name;
    this.symbol = symbol;
    this.decimals = decimals;
    this.localSupply = initialSupply;
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

    // Recipient gets a new token with balance == amount (not localSupply).
    let newToken = new SmartObjectToken(this.name, this.symbol, this.decimals, this.localSupply, to);
    newToken.balance = amount;
    return newToken;
  }

  mint(to: string, amount: u64): SmartObjectToken {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only minter/owner can mint");
    assert(amount > 0, "Mint amount must be greater than zero");

    this.localSupply += amount;
    HostContext.emitEvent("Mint", "Minted " + amount.toString() + " " + this.symbol + " to " + to);

    // New token for recipient has balance == amount (the minted portion), not localSupply
    let newToken = new SmartObjectToken(this.name, this.symbol, this.decimals, this.localSupply, to);
    newToken.balance = amount;
    return newToken;
  }

  burn(amount: u64): void {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only token owner can burn");
    assert(this.balance >= amount, "Insufficient balance to burn");
    assert(amount > 0, "Burn amount must be greater than zero");

    this.balance -= amount;
    this.localSupply -= amount;
    HostContext.emitEvent("Burn", "Burned " + amount.toString() + " " + this.symbol);
  }

  toJson(): string {
    return "{"
      + "\"name\":\"" + escapeJsonString(this.name) + "\","
      + "\"symbol\":\"" + escapeJsonString(this.symbol) + "\","
      + "\"decimals\":" + this.decimals.toString() + ","
      + "\"localSupply\":\"" + this.localSupply.toString() + "\","
      + "\"totalSupply\":\"" + this.localSupply.toString() + "\","
      + "\"balance\":\"" + this.balance.toString() + "\","
      + "\"owner\":\"" + escapeJsonString(this.owner) + "\""
      + "}";
  }
}

export { SmartObjectToken as SOTToken };
