export interface ChainConfig {
  name: string;
  ticker: string;
  p2pkhPrefix: number;
  p2shPrefix: number;
  wifPrefix: number;
  bech32Prefix?: string;
  rpcDefaultPort: number;
  minRelayFee: number;
  dustLimit: number;
}

export const SUPPORTED_CHAINS: Record<string, ChainConfig> = {
  JKC_TESTNET: {
    name: "Junkcoin Testnet",
    ticker: "tJKC",
    p2pkhPrefix: 0x6f,
    p2shPrefix: 0xc4,
    wifPrefix: 0xef,
    bech32Prefix: "tjkc",
    rpcDefaultPort: 9772,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  JKC: {
    name: "Junkcoin",
    ticker: "JKC",
    p2pkhPrefix: 0x10,
    p2shPrefix: 0x05,
    wifPrefix: 0x90,
    bech32Prefix: "jkc",
    rpcDefaultPort: 9772,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  BTC: {
    name: "Bitcoin",
    ticker: "BTC",
    p2pkhPrefix: 0x00,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    bech32Prefix: "bc",
    rpcDefaultPort: 8332,
    minRelayFee: 1000,
    dustLimit: 546,
  },
  LTC: {
    name: "Litecoin",
    ticker: "LTC",
    p2pkhPrefix: 0x30,
    p2shPrefix: 0x32,
    wifPrefix: 0xb0,
    bech32Prefix: "ltc",
    rpcDefaultPort: 9332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  DOGE: {
    name: "Dogecoin",
    ticker: "DOGE",
    p2pkhPrefix: 0x1e,
    p2shPrefix: 0x16,
    wifPrefix: 0x9e,
    rpcDefaultPort: 22555,
    minRelayFee: 100000,
    dustLimit: 100000,
  },
  BEL: {
    name: "Bells",
    ticker: "BEL",
    p2pkhPrefix: 0x19,
    p2shPrefix: 0x1e,
    wifPrefix: 0x99,
    rpcDefaultPort: 19918,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  DINGO: {
    name: "Dingocoin",
    ticker: "DINGO",
    p2pkhPrefix: 0x1e,
    p2shPrefix: 0x16,
    wifPrefix: 0x9e,
    rpcDefaultPort: 33982,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  SHIC: {
    name: "Shiba Inu Coin",
    ticker: "SHIC",
    p2pkhPrefix: 0x3b,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    rpcDefaultPort: 8332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  TRMP: {
    name: "Trumpcoin",
    ticker: "TRMP",
    p2pkhPrefix: 0x3b,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    rpcDefaultPort: 8332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  B1T: {
    name: "Bean Cash",
    ticker: "B1T",
    p2pkhPrefix: 0x3b,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    rpcDefaultPort: 8332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  CRC: {
    name: "Crocodile Cash",
    ticker: "CRC",
    p2pkhPrefix: 0x3b,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    rpcDefaultPort: 8332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  PEP: {
    name: "Pepecoin",
    ticker: "PEP",
    p2pkhPrefix: 0x3b,
    p2shPrefix: 0x05,
    wifPrefix: 0x80,
    rpcDefaultPort: 42069,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
  LKY: {
    name: "Luckycoin",
    ticker: "LKY",
    p2pkhPrefix: 0x30,
    p2shPrefix: 0x05,
    wifPrefix: 0xb0,
    rpcDefaultPort: 9332,
    minRelayFee: 1000,
    dustLimit: 1000,
  },
};

export function getChainConfig(tickerOrName: string): ChainConfig {
  const upper = tickerOrName.toUpperCase();
  if (SUPPORTED_CHAINS[upper]) {
    return SUPPORTED_CHAINS[upper];
  }
  for (const key of Object.keys(SUPPORTED_CHAINS)) {
    if (SUPPORTED_CHAINS[key].name.toUpperCase() === upper) {
      return SUPPORTED_CHAINS[key];
    }
  }
  // Default to JKC_TESTNET (per AGENTS.md: JKC_TESTNET is the default settlement layer)
  return SUPPORTED_CHAINS.JKC_TESTNET;
}
