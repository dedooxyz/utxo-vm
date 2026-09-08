# Rencana Implementasi: Eksekusi Penuh Layer Ekonomi UTXO-VM (Bertahap)

Rencana ini menguraikan tahapan konkret untuk mengeksekusi roadmap model ekonomi UTXO-VM secara modular dan sistematis, sesuai arahan user.

---

## Tahapan Eksekusi

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ FASE 1: Equivocation Detection & Slashing Engine (Taproot + OP_CAT)         │
│ - Deteksi otomatis double-signing di ConsensusManager                       │
│ - Bukti Kriptografis EquivocationProof                                      │
│ - Generator Script Taproot + OP_CAT Slashing Covenant di Junkcoin L1        │
│ - Endpoint RPC /api/v1/consensus/slashing-proofs                            │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ FASE 2: Progressive Gas Micro-Fee Enforcement (Spoke Cashflow)              │
│ - Flag CLI --min-execution-fee & --fee-collector di utxo-vmd               │
│ - Validasi output fee wajib di BlockProcessor saat eksekusi kontrak          │
│ - Proteksi spam transaksi & kompensasi riil operator node                   │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ FASE 3: TypeScript SDK Light Client Verification & DHT Contract Sharing     │
│ - LightMerkleVerifier: Verifikasi SMT Merkle proof mandiri di browser/wallet│
│ - UTXOClient: API getObjectProof, publishContractDht, getContractDht        │
│ - Envelope builder: Inklusi otomatis micro-fee output                       │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ FASE 4: Multi-Node Docker Cluster & Local Simulation                        │
│ - Dockerfile multi-stage build untuk utxo-vmd binary                        │
│ - docker-compose.yml: Node 1 (Bootstrap) + Node 2 (Validator) lokal         │
│ - Script simulasi siaran mempool & atestasi konsensus antar kontainer        │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## User Review Required

> [!IMPORTANT]
> **Struktur Slashing Covenant**:
> Script slashing akan memanfaatkan kapabilitas `OP_CAT` dan `OP_CHECKSIG` pada Taproot Junkcoin L1 untuk memeriksa apakah 2 tanda tangan dari pubkey yang sama menandatangani digest pesan yang berbeda pada tinggi blok yang sama.

> [!NOTE]
> **Progressive Fee Toggle**:
> `--min-execution-fee` default disetel ke `0` (Fase Genesis Adopsi), dan dapat diaktifkan menjadi nilai tertentu (misal `1000` satoshi) melalui konfigurasi node atau environment variable.

---

## Proposed Changes

### FASE 1: Slashing & Equivocation Engine di `packages/node`

#### [MODIFY] [packages/node/src/types.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/types.rs)
- Tambahkan struktur data `EquivocationProof`:
  ```rust
  pub struct EquivocationProof {
      pub chain: String,
      pub block_height: u64,
      pub validator_pubkey: String,
      pub first_attestation: StateAttestation,
      pub second_attestation: StateAttestation,
      pub detected_at: i64,
  }
  ```

#### [NEW] [packages/node/src/consensus/covenants.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/consensus/covenants.rs)
- Generator script Taproot + OP_CAT untuk:
  - `create_staking_script(validator_pubkey, lock_blocks)`
  - `create_slashing_script(validator_pubkey, whistleblower_pubkey)`
  - Verifikasi proof bukti equivocation siap tayang ke mempool L1 Junkcoin.

#### [MODIFY] [packages/node/src/consensus/attestation.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/consensus/attestation.rs)
- Update `ConsensusManager`:
  - Menyimpan koleksi `slashing_proofs: Arc<RwLock<Vec<EquivocationProof>>>`.
  - Pada `add_attestation`: jika pubkey yang sama telah menandatangani attestation pada tinggi blok yang sama dengan `state_root` yang berbeda, secara otomatis bentuk `EquivocationProof`, cetak log peringatan darurat, dan simpan untuk slashing L1.

#### [MODIFY] [packages/node/src/rpc/server.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/rpc/server.rs)
- Tambahkan endpoint:
  - `GET /api/v1/consensus/slashing-proofs`: Mengembalikan daftar bukti kecurangan validator.

---

### FASE 2: Progressive Gas Micro-Fee di `packages/node`

#### [MODIFY] [packages/node/src/main.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/main.rs)
- Tambahkan argumen CLI:
  - `--min-execution-fee <SATS>` (default: 0).
  - `--fee-collector <ADDRESS_OR_SCRIPT>`.

#### [MODIFY] [packages/node/src/scanner/processor.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/scanner/processor.rs)
- Di dalam `process_tx`:
  - Jika `min_execution_fee > 0`, periksa apakah `tx.vout` memiliki output yang membayar ke fee collector atau validator dengan nilai $\ge \text{min\_execution\_fee}$.
  - Jika fee tidak terpenuhi, tolak transisi state smart object (mencegah spam komputasi gratis pada fase berbayar).

---

### FASE 3: TypeScript SDK Light Client Verification & DHT Sharing

#### [MODIFY] [packages/sdk/src/light_verifier.ts](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/sdk/src/light_verifier.ts)
- Tambahkan `LightMerkleVerifier`:
  - Verifikasi deterministik 256-bit SMT Merkle Inclusion Proof langsung di browser / mobile wallet:
    `verifySmtProof(keyHex, valueHex, rootHex, proofPath): boolean`.

#### [MODIFY] [packages/sdk/src/client.ts](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/sdk/src/client.ts)
- Tambahkan method pada `UTXOClient`:
  - `getObjectProof(objectId: string): Promise<{ object: any, merkleProof: any, verified: boolean }>`
  - `publishContractDht(codeHash: string, wasmHex: string): Promise<any>`
  - `getContractDht(codeHash: string): Promise<string>`
  - `getSlashingProofs(): Promise<any[]>`

---

### FASE 4: Multi-Node Docker & Local Cluster Simulation

#### [NEW] [Dockerfile](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/Dockerfile)
- Multi-stage Docker build berbasis Alpine / Debian Slim yang mengompilasi `target/release/utxo-vmd`.

#### [NEW] [docker-compose.yml](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/docker-compose.yml)
- Menjalankan 2 kontainer node:
  - `utxo-node-1`: Seed node (P2P port 2232, RPC port 9773).
  - `utxo-node-2`: Validator node yang terhubung via `--bootstrap-peers /ip4/utxo-node-1/tcp/2232`.

---

## Verification Plan

### Automated Tests
1. **Consensus Equivocation Test (`tests/consensus_tests.rs`)**:
   - Mensimulasikan validator menandatangani 2 state root berbeda untuk blok yang sama.
   - Verifikasi bahwa `ConsensusManager` menangkap kecurangan, menghasilkan `EquivocationProof`, dan menyediakannya untuk klaim slashing.
2. **Covenants Taproot Script Test (`tests/covenants_tests.rs`)**:
   - Verifikasi script hex yang dihasilkan valid dan memenuhi opcode OP_CAT + OP_CHECKSIG.
3. **Fee Enforcement Test (`tests/processor_tests.rs`)**:
   - Menguji transaksi dengan dan tanpa micro-fee output saat `--min-execution-fee` aktif.
4. **SDK Unit Tests (`packages/sdk`)**:
   - Menjalankan `npm test` di `packages/sdk` untuk memvalidasi `LightMerkleVerifier`.

### Manual Verification
1. Jalankan `cargo test --workspace` untuk memverifikasi seluruh komponen Rust.
2. Build container Docker lokal: `docker compose build` atau `cargo test`.
