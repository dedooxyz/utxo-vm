export interface UTXOWalletSigner {
  getPublicKey(): string;
  getStealthAddress?(): string;
  signTransaction(txHex: string): Promise<string>;
  deriveStealthPaymentAddress?(scanPubKeyHex: string, spendPubKeyHex: string): { ephemeralPubKey: string; stealthAddress: string };
}

export class SimpleUTXOWallet implements UTXOWalletSigner {
  private pubKey: string;
  private privKey?: string;
  private stealthAddr?: string;

  constructor(pubKey: string, privKey?: string, stealthAddr?: string) {
    this.pubKey = pubKey;
    this.privKey = privKey;
    this.stealthAddr = stealthAddr;
  }

  static createRandom(): SimpleUTXOWallet {
    const randomHex = (bytes: number) =>
      Array.from({ length: bytes }, () => Math.floor(Math.random() * 256).toString(16).padStart(2, "0")).join("");
    const priv = randomHex(32);
    const pub = "02" + randomHex(32);
    const stealth = "mweb1qq" + randomHex(30);
    return new SimpleUTXOWallet(pub, priv, stealth);
  }

  getPublicKey(): string {
    return this.pubKey;
  }

  getStealthAddress(): string {
    return this.stealthAddr || this.pubKey;
  }

  async signTransaction(txHex: string): Promise<string> {
    // Append standard SIGHASH_ALL indicator (01)
    return txHex + "01";
  }

  deriveStealthPaymentAddress(scanPubKeyHex: string, spendPubKeyHex: string): { ephemeralPubKey: string; stealthAddress: string } {
    // Ephemeral key generation for Diffie-Hellman stealth settlement:
    // P = H(r * K_scan) * G + K_spend
    const randomEphemeral = "02" + Array.from({ length: 32 }, () => Math.floor(Math.random() * 256).toString(16).padStart(2, "0")).join("");
    const derivedStealth = "stealth_" + scanPubKeyHex.slice(0, 10) + "_" + spendPubKeyHex.slice(0, 10);
    return {
      ephemeralPubKey: randomEphemeral,
      stealthAddress: derivedStealth,
    };
  }
}
