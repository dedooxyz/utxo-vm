import axios from "axios";
import { InscriptionEnvelope } from "./envelope";
import { UTXOWalletSigner } from "./wallet";
import { ChainConfig, getChainConfig, SUPPORTED_CHAINS } from "./chains";
import { SmtInclusionProof } from "./light_verifier";

export interface SmartObjectStateResponse {
  objectId: string;
  codeHash: string;
  seal: string; // txid:vout
  satoshis: string;
  owner: string;
  stateData: any;
  updatedAtBlock: number;
}

export interface StateTransitionRecord {
  txid: string;
  consumedSeal: string;
  newSeal: string;
  method: string;
  events: Array<{ topic: string; data: string }>;
  blockHeight: number;
  timestamp: number;
}

export interface BroadcastResult {
  txid: string;
  status: "mempool" | "confirmed";
  seal: string;
}

export interface SimulationResult {
  success: boolean;
  gasConsumed: number;
  returnCode: number;
  updatedState: any;
  events: Array<{ topic: string; data: string }>;
}

export interface ObjectProofResponse {
  object: SmartObjectStateResponse;
  merkleProof: SmtInclusionProof;
  verified: boolean;
}

export interface DhtContractResponse {
  codeHash: string;
  wasmHex: string;
}

export interface SlashingProof {
  chain: string;
  blockHeight: number;
  validatorPubkey: string;
  firstAttestation: any;
  secondAttestation: any;
  detectedAt: number;
}

export class UTXOClient {
  private nodeUrl: string;
  public chain: ChainConfig;

  constructor(chainTicker: string = "BTC", nodeUrl: string = "http://127.0.0.1:9773") {
    this.chain = getChainConfig(chainTicker);
    this.nodeUrl = nodeUrl;
  }

  async getObjectState(objectId: string): Promise<SmartObjectStateResponse> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/object/${objectId}`);
    return resp.data;
  }

  async getObjectHistory(objectId: string): Promise<StateTransitionRecord[]> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/object/${objectId}/history`);
    return resp.data;
  }

  async getObjectsByOwner(owner: string): Promise<SmartObjectStateResponse[]> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/objects/owner/${owner}`);
    return resp.data;
  }

  async simulateCall(targetSeal: string, method: string, args: Record<string, any>): Promise<SimulationResult> {
    const resp = await axios.post(`${this.nodeUrl}/api/v1/simulate`, {
      chain: this.chain.ticker,
      targetSeal,
      method,
      args,
    });
    return resp.data;
  }

  async broadcastEnvelope(
    envelope: InscriptionEnvelope,
    wallet: UTXOWalletSigner,
    attachedSatoshis: bigint = BigInt(1000)
  ): Promise<BroadcastResult> {
    const resp = await axios.post(`${this.nodeUrl}/api/v1/broadcast`, {
      chain: this.chain.ticker,
      envelope: {
        ...envelope,
        payload: Array.from(envelope.payload),
      },
      sender: wallet.getPublicKey(),
      attachedSatoshis: attachedSatoshis.toString(),
    });
    return resp.data;
  }

  async getChainInfo(): Promise<any> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/chain/info`);
    return resp.data;
  }

  async getObjectProof(objectId: string): Promise<ObjectProofResponse> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/object/${objectId}/proof`);
    return resp.data;
  }

  async publishContractDht(codeHash: string, wasmHex: string): Promise<{ status: string; codeHash: string }> {
    const resp = await axios.post(`${this.nodeUrl}/api/v1/dht/contract`, {
      code_hash: codeHash,
      wasm_hex: wasmHex,
    });
    return resp.data;
  }

  async getContractDht(codeHash: string): Promise<DhtContractResponse> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/dht/contract/${codeHash}`);
    return resp.data;
  }

  async getSlashingProofs(): Promise<{ count: number; proofs: SlashingProof[] }> {
    const resp = await axios.get(`${this.nodeUrl}/api/v1/consensus/slashing-proofs`);
    return resp.data;
  }
}
