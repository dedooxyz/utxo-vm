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
