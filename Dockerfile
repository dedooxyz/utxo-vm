FROM node:22-slim AS builder

WORKDIR /app

# Install Rust for core-vm
RUN apt-get update && apt-get install -y curl build-essential && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
    rm -rf /var/lib/apt/lists/*

ENV PATH="/root/.cargo/bin:${PATH}"

# Copy package files
COPY package.json package-lock.json ./
COPY packages/core-vm/Cargo.toml packages/core-vm/Cargo.toml
COPY packages/indexer/package.json packages/indexer/package.json

# Install dependencies
RUN npm ci

# Copy source
COPY packages/core-vm/ packages/core-vm/
COPY packages/indexer/ packages/indexer/
COPY chain.json chain.json

# Build core-vm
RUN cd packages/core-vm && cargo build --release

# Build indexer
RUN npm run build --workspace=packages/indexer

# Production stage
FROM node:22-slim

WORKDIR /app

# Copy built artifacts
COPY --from=builder /app/packages/indexer/dist/ packages/indexer/dist/
COPY --from=builder /app/packages/indexer/package.json packages/indexer/package.json
COPY --from=builder /app/packages/core-vm/target/release/utxo-core-vm-cli packages/core-vm/target/release/utxo-core-vm-cli
COPY --from=builder /app/node_modules/ node_modules/
COPY --from=builder /app/chain.json chain.json

# Create data directory
RUN mkdir -p /app/data

ENV NODE_ENV=production
ENV CHAIN=JKC_TESTNET
ENV CHAIN_CONFIG_PATH=/app/chain.json
ENV DB_PATH=/app/data/indexer.db
ENV SYNC_INTERVAL_MS=30000
ENV PORT=9773

EXPOSE 9773

CMD ["node", "packages/indexer/dist/index.js"]
