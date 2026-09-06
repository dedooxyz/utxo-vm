import { HostContext } from "./env";

/**
 * Smart Object NFT (SON)
 * Canonical Non-Fungible Token standard for UTXO-VM
 */
export class SmartObjectNFT {
  collectionName: string;
  tokenId: u64;
  metadataUri: string;
  owner: string;

  constructor(collectionName: string, tokenId: u64, metadataUri: string, owner: string) {
    this.collectionName = collectionName;
    this.tokenId = tokenId;
    this.metadataUri = metadataUri;
    this.owner = owner;
  }

  transfer(to: string): void {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only current NFT owner can transfer");
    this.owner = to;
    HostContext.emitEvent("NFTTransfer", "Token " + this.tokenId.toString() + " transferred to " + to);
  }

  setMetadataUri(newUri: string): void {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only owner can update metadata");
    this.metadataUri = newUri;
    HostContext.emitEvent("NFTMetadataUpdate", "Token " + this.tokenId.toString() + " metadata updated");
  }

  burn(): void {
    let caller = HostContext.getCaller();
    assert(caller == this.owner, "Unauthorized: Only owner can burn");
    this.owner = "";
    HostContext.emitEvent("NFTBurn", "Token " + this.tokenId.toString() + " burned");
  }

  toJson(): string {
    return "{"
      + "\"collectionName\":\"" + this.collectionName + "\","
      + "\"tokenId\":\"" + this.tokenId.toString() + "\","
      + "\"metadataUri\":\"" + this.metadataUri + "\","
      + "\"owner\":\"" + this.owner + "\""
      + "}";
  }
}

export { SmartObjectNFT as SONNFT };
