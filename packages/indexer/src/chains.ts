import fs from "fs";
import path from "path";

const CHAIN_CONFIG_PATH = process.env.CHAIN_CONFIG_PATH || path.join(__dirname, "../../../chain.json");

export interface NetworkConfig {
  p2pkhPrefix: number;
  p2shPrefix: number;
  wifPrefix: number;
  bech32Prefix?: string;
}

export interface OpcodesConfig {
  OP_CREATE_SMART_OBJECT?: number;
  OP_CALL_SMART_OBJECT?: number;
  OP_UPDATE_STATE?: number;
  OP_SETTLE_CROSS_CHAIN?: number;
  OP_VERIFY_SEAL?: number;
}

export interface SettlementConfig {
  enabled: boolean;
  role: "settlement_layer" | "child_chain" | "none";
  supportedChains?: string[];
  settlementChain?: string;
}

export interface ChainConfig {
  name: string;
  ticker: string;
  electrsUrl: string;
  network: NetworkConfig;
  opcodesSupported: boolean;
  opcodes: OpcodesConfig;
  settlement: SettlementConfig;
  env: "testnet" | "mainnet" | "regtest";
}

export interface ChainConfigFile {
  chains: Record<string, ChainConfig>;
  defaults: {
    chain: string;
    electrsUrl: string;
  };
}

let configCache: ChainConfigFile | null = null;

function loadChainConfig(): ChainConfigFile {
  if (configCache) return configCache;

  try {
    const raw = fs.readFileSync(CHAIN_CONFIG_PATH, "utf8");
    configCache = JSON.parse(raw);
    console.log(`[Chains] Loaded ${Object.keys(configCache!.chains).length} chains from ${CHAIN_CONFIG_PATH}`);
    return configCache!;
  } catch (err: any) {
    console.warn(`[Chains] Failed to load chain.json: ${err.message}, using defaults`);
    configCache = {
      chains: {
        JKC_TESTNET: {
          name: "Junkcoin Testnet",
          ticker: "tJKC",
          electrsUrl: "https://jkc-testnet-api.s3na.xyz",
          network: { p2pkhPrefix: 0x6f, p2shPrefix: 0xc4, wifPrefix: 0xef, bech32Prefix: "tjkc" },
          opcodesSupported: true,
          opcodes: {
            OP_CREATE_SMART_OBJECT: 0xc0,
            OP_CALL_SMART_OBJECT: 0xc1,
            OP_UPDATE_STATE: 0xc2,
            OP_SETTLE_CROSS_CHAIN: 0xc3,
            OP_VERIFY_SEAL: 0xc4,
          },
          settlement: {
            enabled: true,
            role: "settlement_layer",
            supportedChains: ["BTC", "LTC", "DOGE", "BEL", "DINGO", "LKY", "SHIC", "TRMP", "B1T", "CRC", "PEP"],
          },
          env: "testnet",
        },
      },
      defaults: { chain: "JKC_TESTNET", electrsUrl: "https://jkc-testnet-api.s3na.xyz" },
    };
    return configCache!;
  }
}

/**
 * Get chain config by chain ID
 */
export function getChainConfig(chainId?: string): ChainConfig {
  const config = loadChainConfig();
  const id = chainId || config.defaults.chain;
  const chain = config.chains[id];

  if (!chain) {
    throw new Error(`Chain not found: ${id}. Available: ${Object.keys(config.chains).join(", ")}`);
  }

  return chain;
}

/**
 * Get all available chain IDs
 */
export function getAvailableChains(): string[] {
  const config = loadChainConfig();
  return Object.keys(config.chains);
}

/**
 * Check if chain supports opcodes
 */
export function isOpcodesSupported(chainId: string): boolean {
  const chain = getChainConfig(chainId);
  return chain.opcodesSupported;
}

/**
 * Get settlement layer chain for a child chain
 */
export function getSettlementChain(childChainId: string): string | null {
  const chain = getChainConfig(childChainId);
  return chain.settlement.settlementChain || null;
}

/**
 * Check if a chain is a settlement layer
 */
export function isSettlementLayer(chainId: string): boolean {
  const chain = getChainConfig(chainId);
  return chain.settlement.role === "settlement_layer";
}

/**
 * Get supported child chains for a settlement layer
 */
export function getSupportedChildChains(settlementChainId: string): string[] {
  const chain = getChainConfig(settlementChainId);
  return chain.settlement.supportedChains || [];
}

/**
 * Reload chain config from disk
 */
export function reloadChainConfig(): void {
  configCache = null;
  loadChainConfig();
}
