#!/usr/bin/env node
/**
 * Cross-Chain Verification Module
 * 
 * Simulates child chain verifying state on Junkcoin
 * Tests: seal verification, merkle proof, state consistency
 */

import * as crypto from "crypto";

const JUNKCOIN_INDEXER = "http://localhost:9773";

// =====================================================
// Merkle Tree Implementation
// =====================================================
class MerkleTree {
  private leaves: Buffer[];
  private tree: Buffer[][];

  constructor(leaves: string[]) {
    this.leaves = leaves.map(l => Buffer.from(l, "hex"));
    this.tree = this.buildTree(this.leaves);
  }

  private buildTree(leaves: Buffer[]): Buffer[][] {
    if (leaves.length === 0) return [];
    
    let level = leaves;
    const tree: Buffer[][] = [level];
    
    while (level.length > 1) {
      const nextLevel: Buffer[] = [];
      for (let i = 0; i < level.length; i += 2) {
        const left = level[i];
        const right = i + 1 < level.length ? level[i + 1] : left;
        const combined = Buffer.concat([left, right]);
        const hash = crypto.createHash("sha256").update(combined).digest();
        nextLevel.push(hash);
      }
      level = nextLevel;
      tree.push(level);
    }
    
    return tree;
  }

  getRoot(): Buffer {
    if (this.tree.length === 0) return Buffer.alloc(0);
    return this.tree[this.tree.length - 1][0];
  }

  getProof(index: number): Array<{hash: Buffer, position: string}> {
    const proof: Array<{hash: Buffer, position: string}> = [];
    let idx = index;
    
    for (let i = 0; i < this.tree.length - 1; i++) {
      const level = this.tree[i];
      const isRight = idx % 2 === 1;
      const siblingIdx = isRight ? idx - 1 : idx + 1;
      
      if (siblingIdx < level.length) {
        proof.push({
          hash: level[siblingIdx],
          position: isRight ? "left" : "right"
        });
      }
      
      idx = Math.floor(idx / 2);
    }
    
    return proof;
  }

  static verify(leaf: string, proof: Array<{hash: string, position: string}>, root: string): boolean {
    let current = Buffer.from(leaf, "hex");
    
    for (const step of proof) {
      const sibling = Buffer.from(step.hash, "hex");
      const combined = step.position === "left" 
        ? Buffer.concat([sibling, current])
        : Buffer.concat([current, sibling]);
      current = crypto.createHash("sha256").update(combined).digest();
    }
    
    return current.equals(Buffer.from(root, "hex"));
  }
}

// =====================================================
// Cross-Chain State Anchor
// =====================================================
function createStateAnchor(
  chainId: string,
  blockHeight: number,
  blockHash: string,
  stateRoot: string,
  prevSeal: string | null
) {
  return {
    type: "state_anchor",
    chainId,
    blockHeight,
    blockHash: "0x" + blockHash,
    stateRoot: "0x" + stateRoot,
    merkleRoot: "0x" + stateRoot,
    prevSeal: prevSeal || null,
    timestamp: Date.now(),
  };
}

// =====================================================
// Cross-Chain State Claim
// =====================================================
function createStateClaim(
  chainId: string,
  junkcoinSeal: string,
  merkleRoot: string,
  proof: Array<{hash: string, position: string}>,
  signatures: string[]
) {
  return {
    type: "state_claim",
    chainId,
    junkcoinSeal,
    merkleRoot: "0x" + merkleRoot,
    proof,
    signatures,
    timestamp: Date.now(),
  };
}

// =====================================================
// Seal Verification Client
// =====================================================
class SealVerifier {
  private indexerUrl: string;

  constructor(indexerUrl: string) {
    this.indexerUrl = indexerUrl;
  }

  async verifySeal(seal: string) {
    try {
      const [txid, vout] = seal.split(":");
      const response = await fetch(`${this.indexerUrl}/api/v1/object/obj_${txid.slice(0, 16)}`);
      
      if (!response.ok) {
        return { valid: false, error: "Seal not found" };
      }
      
      const object = await response.json();
      
      return {
        valid: object.seal === seal,
        object,
        seal: object.seal,
        stateData: object.stateData,
      };
    } catch (err: unknown) {
      return { valid: false, error: (err as Error).message };
    }
  }

  async verifyStateAnchor(anchor: { chainId: string, blockHeight: number }) {
    try {
      const response = await fetch(`${this.indexerUrl}/api/v1/objects`);
      const objects = await response.json();
      
      const matchingAnchor = objects.find((obj: any) => {
        const state = obj.stateData;
        return state.type === "state_anchor" &&
               state.chainId === anchor.chainId &&
               state.blockHeight === anchor.blockHeight;
      });
      
      if (!matchingAnchor) {
        return { valid: false, error: "Anchor not found" };
      }
      
      return {
        valid: true,
        seal: matchingAnchor.seal,
        object: matchingAnchor,
      };
    } catch (err: unknown) {
      return { valid: false, error: (err as Error).message };
    }
  }

  async verifyMerkleRoot(chainId: string, blockHeight: number, merkleRoot: string) {
    try {
      const response = await fetch(`${this.indexerUrl}/api/v1/objects`);
      const objects = await response.json();
      
      const matchingAnchor = objects.find((obj: any) => {
        const state = obj.stateData;
        return state.type === "state_anchor" &&
               state.chainId === chainId &&
               state.blockHeight === blockHeight &&
               state.merkleRoot === "0x" + merkleRoot;
      });
      
      return { valid: !!matchingAnchor, seal: matchingAnchor?.seal };
    } catch (err: unknown) {
      return { valid: false, error: (err as Error).message };
    }
  }
}

// =====================================================
// Export for testing
// =====================================================
module.exports = {
  MerkleTree,
  createStateAnchor,
  createStateClaim,
  SealVerifier,
};

// =====================================================
// Run tests if executed directly
// =====================================================
if (require.main === module) {
  async function main() {
    console.log("╔══════════════════════════════════════════════════════╗");
    console.log("║  Cross-Chain Verification Module Test               ║");
    console.log("╚══════════════════════════════════════════════════════╝\n");

    const verifier = new SealVerifier(JUNKCOIN_INDEXER);

    // Test 1: Merkle Tree
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("TEST 1: Merkle Tree Construction & Verification");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    const leaves = [
      crypto.randomBytes(32).toString("hex"),
      crypto.randomBytes(32).toString("hex"),
      crypto.randomBytes(32).toString("hex"),
      crypto.randomBytes(32).toString("hex"),
    ];

    const tree = new MerkleTree(leaves);
    const root = tree.getRoot().toString("hex");
    console.log(`  Root: ${root}`);

    const proof = tree.getProof(0);
    console.log(`  Proof for leaf 0: ${proof.length} steps`);

    const proofStrings = proof.map(p => ({
      hash: p.hash.toString("hex"),
      position: p.position,
    }));
    const valid = MerkleTree.verify(leaves[0], proofStrings, root);
    console.log(`  Verification: ${valid ? "✅ PASS" : "❌ FAIL"}`);

    // Test 2: State Anchor Creation
    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("TEST 2: State Anchor Creation");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    const anchor = createStateAnchor(
      "chain_a_testnet",
      12345,
      crypto.randomBytes(32).toString("hex"),
      root,
      null
    );
    console.log(`  Chain: ${anchor.chainId}`);
    console.log(`  Block: ${anchor.blockHeight}`);
    console.log(`  Merkle Root: ${anchor.merkleRoot}`);

    // Test 3: Seal Verification
    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("TEST 3: Seal Verification Against Junkcoin");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    const objects = await (await fetch(`${JUNKCOIN_INDEXER}/api/v1/objects`)).json();
    console.log(`  Objects on Junkcoin: ${objects.length}`);

    if (objects.length > 0) {
      const testSeal = objects[0].seal;
      const result = await verifier.verifySeal(testSeal);
      console.log(`  Verifying seal: ${testSeal}`);
      console.log(`  Result: ${result.valid ? "✅ VALID" : "❌ INVALID"}`);
    }

    // Test 4: State Claim Verification
    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("TEST 4: State Claim Verification");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    if (objects.length > 0) {
      const claim = createStateClaim(
        "chain_a_testnet",
        objects[0].seal,
        root,
        proof.map(p => ({
          hash: p.hash.toString("hex"),
          position: p.position,
        })),
        ["0x" + crypto.randomBytes(64).toString("hex")]
      );
      console.log(`  Claim created for seal: ${claim.junkcoinSeal}`);
      console.log(`  Proof steps: ${claim.proof.length}`);
      console.log(`  Signatures: ${claim.signatures.length}`);
    }

    // Test 5: Multi-Chain State Consistency
    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("TEST 5: Multi-Chain State Consistency");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    const chains = ["chain_a", "chain_b", "chain_c"];
    for (const chainId of chains) {
      const chainAnchor = objects.find((obj: any) => 
        obj.stateData?.chainId === chainId
      );
      console.log(`  ${chainId}: ${chainAnchor ? "✅ anchored" : "❌ not anchored"}`);
    }

    console.log("\n=== Cross-Chain Verification Module Test Complete ===");
  }

  main().catch(console.error);
}
