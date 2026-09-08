export interface LineageTransition {
  txid: string;
  consumedSeal: string;
  newSeal: string;
  method?: string;
  blockHeight?: number;
}

export interface LineageVerificationResult {
  verified: boolean;
  objectId: string;
  genesisSeal: string;
  tipSeal: string;
  transitionsVerified: number;
  error?: string;
}

export interface SmtProofNode {
  position: "left" | "right";
  hash_hex: string;
}

export interface SmtInclusionProof {
  key_hex: string;
  value_hex: string;
  root_hex: string;
  proof_path: SmtProofNode[];
  verified: boolean;
}

export class LightMerkleVerifier {
  private static readonly SMT_LEAF_PREFIX = new TextEncoder().encode("SMT_LEAF");

  static async sha256(data: Uint8Array): Promise<Uint8Array> {
    const buffer = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
    const hashBuffer = await crypto.subtle.digest("SHA-256", buffer);
    return new Uint8Array(hashBuffer);
  }

  static async hashNode(left: Uint8Array, right: Uint8Array): Promise<Uint8Array> {
    const combined = new Uint8Array(left.length + right.length);
    combined.set(left, 0);
    combined.set(right, left.length);
    return this.sha256(combined);
  }

  static async hashLeaf(key: Uint8Array, value: Uint8Array): Promise<Uint8Array> {
    const combined = new Uint8Array(this.SMT_LEAF_PREFIX.length + key.length + value.length);
    combined.set(this.SMT_LEAF_PREFIX, 0);
    combined.set(key, this.SMT_LEAF_PREFIX.length);
    combined.set(value, this.SMT_LEAF_PREFIX.length + key.length);
    return this.sha256(combined);
  }

  static async verifySmtProof(
    keyHex: string,
    valueHex: string,
    rootHex: string,
    proofPath: SmtProofNode[]
  ): Promise<boolean> {
    const key = this.hexToBytes(keyHex);
    const value = this.hexToBytes(valueHex);
    const root = this.hexToBytes(rootHex);

    let current = await this.hashLeaf(key, value);

    for (const node of proofPath) {
      const sibling = this.hexToBytes(node.hash_hex);
      if (node.position === "left") {
        current = await this.hashNode(sibling, current);
      } else {
        current = await this.hashNode(current, sibling);
      }
    }

    return this.bytesToHex(current) === this.bytesToHex(root);
  }

  static hexToBytes(hex: string): Uint8Array {
    const cleanHex = hex.startsWith("0x") ? hex.slice(2) : hex;
    const bytes = new Uint8Array(cleanHex.length / 2);
    for (let i = 0; i < cleanHex.length; i += 2) {
      bytes[i / 2] = parseInt(cleanHex.substring(i, i + 2), 16);
    }
    return bytes;
  }

  static bytesToHex(bytes: Uint8Array): string {
    return Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }
}

export class LightLineageVerifier {
  /**
   * Verify an unbroken single-use seal lineage from genesis to current tip
   */
  static async verifyObjectLineage(
    objectId: string,
    history: LineageTransition[],
    electrsUrl?: string,
    fetchTxFn?: (txid: string) => Promise<any>
  ): Promise<LineageVerificationResult> {
    if (!history || history.length === 0) {
      return {
        verified: false,
        objectId,
        genesisSeal: "",
        tipSeal: "",
        transitionsVerified: 0,
        error: "Empty history",
      };
    }

    const fetchTx =
      fetchTxFn ||
      (async (txid: string) => {
        if (!electrsUrl) throw new Error("electrsUrl required if fetchTxFn is not provided");
        const res = await fetch(`${electrsUrl}/tx/${txid}`);
        if (!res.ok) throw new Error(`Electrs tx fetch failed: ${res.status}`);
        return await res.json();
      });

    const genesis = history[0];
    let expectedSeal = genesis.newSeal;

    for (let i = 1; i < history.length; i++) {
      const step = history[i];

      // Step 1: In-memory continuity check
      if (step.consumedSeal !== expectedSeal) {
        return {
          verified: false,
          objectId,
          genesisSeal: genesis.newSeal,
          tipSeal: expectedSeal,
          transitionsVerified: i,
          error: `Seal discontinuity at step ${i}: expected consumed seal ${expectedSeal}, got ${step.consumedSeal}`,
        };
      }

      // Step 2: Blockchain verification (if fetchTx is available)
      try {
        const txData = await fetchTx(step.txid);
        const [prevTxid, prevVoutStr] = step.consumedSeal.split(":");
        const prevVout = parseInt(prevVoutStr, 10);

        // Verify that txData.vin actually spent prevTxid:prevVout
        const spentInput = (txData.vin || []).find(
          (v: any) =>
            (v.txid === prevTxid || v.prevout?.txid === prevTxid) &&
            (v.vout === prevVout || v.prevout?.vout === prevVout)
        );

        if (!spentInput) {
          return {
            verified: false,
            objectId,
            genesisSeal: genesis.newSeal,
            tipSeal: expectedSeal,
            transitionsVerified: i,
            error: `Fraudulent seal: tx ${step.txid} did not spend seal ${step.consumedSeal} on-chain`,
          };
        }
      } catch (err: any) {
        return {
          verified: false,
          objectId,
          genesisSeal: genesis.newSeal,
          tipSeal: expectedSeal,
          transitionsVerified: i,
          error: `Blockchain verification failed for tx ${step.txid}: ${err.message}`,
        };
      }

      expectedSeal = step.newSeal;
    }

    return {
      verified: true,
      objectId,
      genesisSeal: genesis.newSeal,
      tipSeal: expectedSeal,
      transitionsVerified: history.length,
    };
  }
}
