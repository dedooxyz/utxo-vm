# Rencana Implementasi: Konversi Indexer ke Rust Daemon (`utxo-vmd`), P2P Kademlia DHT, & Verifiable State Consensus

Dokumen ini memfinalisasi rencana teknis untuk mentransisikan komponen Indexer UTXO-VM (yang saat ini berbasis Node.js/TypeScript + SQLite) menjadi sebuah **Universal Rust Node Daemon (`packages/node` / `utxo-vmd`)**. 

Transformasi ini menyelesaikan masalah *trust & security gap* (ketergantungan pada single indexer operator) dengan menghadirkan:
1. **P2P Networking (`libp2p`)**: Kademlia DHT untuk penemuan peer & distribusi bytecode WASM, serta GossipSub untuk mempool & atestasi state.
2. **Kriptografi State Verifiable**: Mengganti hash SQLite sekuensial dengan **Sparse Merkle Tree (SMT)** di atas **RocksDB**, memungkinkan *Merkle Inclusion Proof* bagi klien ringan.
3. **Mekanisme Konsensus State Root**: Validator Quorum Attestation (Threshold Signature) yang terikat pada konsensus deterministik L1, dengan kesiapan integrasi ZK Proof.

---

## User Review Required

> [!IMPORTANT]
> **Keputusan Struktur Monorepo**:
> Komponen node baru akan ditempatkan di [`packages/node`](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node) sebagai crate workspace Cargo baru, dan didaftarkan ke root [`Cargo.toml`](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/Cargo.toml). Indexer TypeScript saat ini di [`packages/indexer`](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/indexer) akan tetap dipertahankan selama fase transisi sebagai pembanding (compatibility fallback) sebelum akhirnya didepresiasi.

> [!NOTE]
> **Database Storage Engine**:
> Menggunakan **RocksDB** (`rocksdb` crate) untuk penyimpanan state UTXO-VM kecepatan tinggi dan ACID durability, menggantikan `better-sqlite3`.

---

> [!NOTE]
> Parameter konfigurasi node:
> - Port P2P default ditetapkan ke **`2232`**.
> - Port RPC default ditetapkan ke **`9773`** (kompatibel dengan indexer saat ini).
> - Komponen dirancang modular sehingga parameter ini dapat dikonfigurasi melalui argumen CLI atau file `.env`.

---

## Model Ekonomi: Shared Security & Staking JKC Native untuk Ekosistem Multi-Chain (Doge, LTC, BEL)

Pertanyaan krusial: **Apakah koin native Junkcoin ($JKC) bisa menjadi koin staking utama, namun UTXO-VM melayani dan mengamankan sistem Dogecoin?**

### Jawabannya: BISA BANGET (Model Interchain Security / Shared Restaking).
Model ini mirip dengan cara kerja **Cosmos Interchain Security**, **EigenLayer**, atau **Babylon BTC Staking**, tetapi diterapkan pada rantai UTXO PoW:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    JUNKCOIN L1: THE SECURITY & STAKING HUB                  │
│                                                                             │
│  [ Validator Staking Covenant (Taproot + OP_CAT) ]                         │
│  - Operator mengunci jaminan $JKC native di Vault Junkcoin.                │
│  - Dev Fund JKC memberikan imbal hasil yield/subsidi staking awal.         │
│  - Slashing Rule: Jika validator curang di chain satelit (misal Doge),     │
│    Fraud Proof diserahkan ke L1 JKC via OP_CAT -> Koin $JKC disita/burn!   │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ Memberikan jaminan keamanan
                                       │ (Validator Quorum Attestation)
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                       DOGECOIN: THE HIGH-VOLUME ASSET SPOKE                 │
│                                                                             │
│  [ Smart Objects on Dogecoin ] (UTX20, Tokens, DEX, DeFi di Doge)           │
│  - Node operator `utxo-vmd` memproses transaksi dan State Root Doge.        │
│  - User Doge membayar fee transaksi dalam DOGE asli.                        │
│  - Fee DOGE mengalir langsung ke kantong operator node.                     │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Mengapa Desain Ini Jenius Secara Ekonomi & Teknis?
1. **Dogecoin L1 Tidak Memiliki Covenant/OP_CAT**:
   - Dogecoin tidak bisa melakukan slashing sendiri karena scriptnya terbatas.
   - Junkcoin (yang memiliki Taproot + OP_CAT + SegWit) bertindak sebagai **"Mahkamah Pengadilan / Slashing Engine"**.
2. **Flywheel Nilai untuk $JKC**:
   - Siapa pun di dunia yang ingin menjadi operator node/sequencer untuk meraup biaya gas dari ekosistem Dogecoin yang raksasa **WAJIB membeli dan mengunci koin native $JKC** di L1 Junkcoin sebagai jaminan!
   - Ini menyedot peredaran $JKC di pasar bebas (*supply shock*) dan mendongkrak kapitalisasi pasar JKC.
3. **Double-Incentive untuk Node Operator**:
   - Operator node mendapatkan **Yield Staking $JKC** (didukung Dev Fund di fase awal).
   - Ditambah **Cashflow Nyata dalam bentuk DOGE** dari aktivitas pengguna Dogecoin.


---

## Arsitektur Sistem Baru (`utxo-vmd`)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          UTXO-VMD (RUST NODE DAEMON)                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  [ 1. P2P NETWORK LAYER: rust-libp2p ]                                      │
│  ├── Kademlia DHT: Peer routing, bootnodes, WASM bytecode CID lookup        │
│  ├── GossipSub: Broadcast mempool envelopes & State Root Attestations       │
│  └── Request-Response: State sync & Merkle proof querying                   │
│                                                                             │
│  [ 2. L1 ADAPTER & BLOCK SCANNER ]                                          │
│  ├── Client Electrs/RPC (async reqwest / jsonrpc)                           │
│  ├── Envelope Parser: Decode OP_FALSE OP_IF "utxovm" payloads               │
│  └── Reorg Handler: L1 block hash tracking & deterministic state rollbacks  │
│                                                                             │
│  [ 3. IN-MEMORY DETERMINISTIC RUNTIME ]                                     │
│  ├── Direct binding ke `packages/core-vm` (Wasmtime engine)                 │
│  ├── Single-Use Seal Manager (txid:vout ownership verification)             │
│  └── Gas & State Delta Calculator                                           │
│                                                                             │
│  [ 4. STORAGE & CRYPTOGRAPHIC PROOF ENGINE ]                                │
│  ├── Embedded RocksDB: Key-Value storage untuk smart objects & transitions   │
│  └── Sparse Merkle Tree (SMT): 256-bit Merkle roots & inclusion proofs      │
│                                                                             │
│  [ 5. CONSENSUS & ATTESTATION ENGINE ]                                      │
│  ├── State Root Signer: Schnorr/BLS threshold signing per block height      │
│  ├── Quorum Checker: Memverifikasi kesepakatan >= 2/3 node pada State Root │
│  └── Dispute / Divergence Alert: Mendeteksi jika ada node yang fork/curang  │
│                                                                             │
│  [ 6. RPC & CLIENT INTERFACE ]                                              │
│  ├── High-throughput Axum HTTP REST & JSON-RPC 2.0                          │
│  └── Light Client Proof API: Mengembalikan `(state_value, smt_proof)`       │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Proposed Changes

### Layer 1: Cargo Workspace & Crate Setup

#### [MODIFY] [Cargo.toml](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/Cargo.toml)
* Daftarkan `"packages/node"` ke dalam `members` workspace Cargo.

#### [NEW] [packages/node/Cargo.toml](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/Cargo.toml)
* Definisikan dependency Rust kelas industri:
  - `tokio`: Async runtime (multi-threaded).
  - `libp2p`: P2P networking dengan fitur `kad`, `gossipsub`, `noise`, `yamux`, `identify`, `ping`, `request-response`.
  - `rocksdb`: Engine database embedded.
  - `utxo-core-vm`: Path dependency ke `../core-vm` untuk eksekusi WASM in-memory.
  - `sparse-merkle-tree`: Struktur data SMT untuk kalkulasi Merkle path yang bisa diverifikasi kriptografis.
  - `axum` & `tower-http`: HTTP & JSON-RPC server berkecepatan tinggi.
  - `reqwest`: Client HTTP untuk berkomunikasi dengan L1 Electrs/RPC node.
  - `secp256k1` / `schnorrkel`: Penandatanganan State Root attestation.
  - `serde`, `serde_json`, `clap` (CLI args parser), `tracing` (structured logging).

---

### Layer 2: P2P Network Module (Kademlia DHT & GossipSub)

#### [NEW] [packages/node/src/p2p/behaviour.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/p2p/behaviour.rs)
* Menggabungkan protokol libp2p ke dalam composite `NetworkBehaviour`:
  - `Kademlia`: DHT untuk peer routing dan penyebaran contract WASM bytecode (berdasarkan CID/hash).
  - `Gossipsub`: Topik `utxovm/mempool/v1` (siaran transaksi unconfirmed) dan `utxovm/attestation/v1` (atestasi state root per blok).
  - `Identify` & `Ping`: Health-check dan pertukaran metadata antar node.

#### [NEW] [packages/node/src/p2p/node.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/p2p/node.rs)
* Event loop `P2pService` berbasis `tokio::select!`.
* Menangani koneksi bootstrap peers, penyebaran pesan gossip, dan penanganan permintaan DHT.

---

### Layer 3: Storage & Sparse Merkle Tree (SMT)

#### [NEW] [packages/node/src/storage/db.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/storage/db.rs)
* Wrapper untuk **RocksDB** dengan column families:
  - `cf_objects`: Menyimpan `object_id -> SmartObjectRecord`.
  - `cf_seals`: Menyimpan `seal (txid:vout) -> object_id`.
  - `cf_transitions`: Riwayat transisi state `txid -> StateTransitionRecord`.
  - `cf_blocks`: Informasi blok `height -> BlockRecord (hash, state_root)`.
  - `cf_undo`: Log rollback untuk menangani reorg L1 secara atomik.

#### [NEW] [packages/node/src/storage/smt.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/storage/smt.rs)
* Implementasi **Sparse Merkle Tree (SMT)** 256-bit.
* Setiap kali ada update smart object:
  - Key: `hash256(object_id)`
  - Value: `hash256(code_hash || seal || satoshis || owner || state_data)`
* Menghasilkan root deterministik `StateRoot` di setiap blok, serta menghasilkan **Inclusion/Exclusion Proofs** untuk dibagikan ke klien ringan.

---

### Layer 4: L1 Scanner & Deterministic Execution

#### [NEW] [packages/node/src/scanner/electrs.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/scanner/electrs.rs)
* Async scanner yang membaca tip blok L1 (Junkcoin, Bitcoin, Doge, dsb.) via Electrs REST API / RPC.
* Deteksi reorg: Memeriksa header hash blok sebelumnya; jika terjadi *fork*, memicu rollback state atomik di RocksDB.

#### [NEW] [packages/node/src/scanner/parser.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/scanner/parser.rs)
* Rust port dari parser envelope `utxovm`:
  - Mengekstrak script witness: `OP_FALSE OP_IF "utxovm" <ver> <contentType> <payload> OP_ENDIF`.
  - Validasi Single-Use Seal: Memastikan input UTXO yang di-spend benar-benar valid menurut transaksi L1.

#### [NEW] [packages/node/src/scanner/processor.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/scanner/processor.rs)
* Penghubung antara scanner L1 dan `utxo-core-vm`:
  - Menyiapkan context pemanggilan WASM (`HostContext`).
  - Mengeksekusi bytecode WASM via `utxo-core-vm::Runtime`.
  - Mengupdate state di RocksDB & memperbarui SMT.

---

### Layer 5: Consensus & State Root Attestation

#### [NEW] [packages/node/src/consensus/attestation.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/consensus/attestation.rs)
* Struktur pesan atestasi:
  ```rust
  pub struct StateAttestation {
      pub chain: String,
      pub block_height: u64,
      pub block_hash: String,
      pub state_root: [u8; 32],
      pub validator_pubkey: String,
      pub signature: Vec<u8>,
  }
  ```
* Validasi tanda tangan dan pengelompokan (*aggregation*) atestasi per tinggi blok.
* Quorum Rules: Ketika sebuah `state_root` memperoleh tanda tangan $\ge 2/3$ validator terdaftar, state root tersebut ditandai sebagai **Canonical Verified State**.

---

### Layer 6: JSON-RPC & Axum Web Interface

#### [NEW] [packages/node/src/rpc/server.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/rpc/server.rs)
* Implementasi Axum router:
  - `GET /api/v1/chain/info`: Info sinkronisasi rantai & peers P2P yang terhubung.
  - `GET /api/v1/object/:id`: Mengambil data objek pintar.
  - `GET /api/v1/object/:id/proof`: Mengembalikan `(object_data, smt_merkle_proof)` untuk verifikasi trustless pada klien.
  - `GET /api/v1/state-root/:height`: Mengambil canonical state root beserta kumpulan tanda tangan atestasi validator.
  - `POST /api/v1/broadcast`: Menerima transaksi UTXO-VM dari user dan menyiarkannya via P2P GossipSub.

#### [NEW] [packages/node/src/main.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/main.rs)
* Entry-point utama binary `utxo-vmd`:
  - Inisialisasi CLI (`--chain`, `--rpc-port`, `--p2p-port`, `--electrs-url`, `--validator-key`).
  - Bootstrapping modul: Storage, P2P swarm, Scanner loop, dan RPC server berjalan bersamaan secara asinkronus menggunakan `tokio`.

---

## Verification Plan

### Automated Tests
1. **SMT Unit Tests**:
   - Menambahkan 10.000 objek acak, mengupdate state, dan memvalidasi bahwa Merkle root konsisten dan deterministik.
   - Menguji keabsahan *Inclusion Proof* dan *Exclusion Proof* (membuktikan bahwa objek palsu tidak ada di state tree).
2. **Deterministic VM Execution Test**:
   - Menjalankan kontrak token `UTX20` WASM in-memory di Rust dan memverifikasi transisi state sesuai spesifikasi.
3. **P2P Swarm Integration Test**:
   - Menjalankan dua instance `utxo-vmd` lokal di memori/test harness.
   - Node 1 menyiarkan transaksi ke GossipSub; verifikasi Node 2 menerima dan memprosesnya.
   - Node 1 meng-upload bytecode WASM ke Kademlia DHT; verifikasi Node 2 dapat mengunduhnya via `code_hash`.
4. **L1 Sync & Rollback Test**:
   - Mensimulasikan data blok Junkcoin/Bitcoin; verifikasi pembentukan State Root per blok.
   - Mensimulasikan reorg 2 blok; verifikasi rollback SMT dan RocksDB kembali ke state semula.

### Manual Verification
1. Build binary: `cargo build --package utxo-vmd --release`.
2. Jalankan satu node terhadap Junkcoin Testnet Electrs (`https://jkc-testnet-api.s3na.xyz`).
3. Query endpoint RPC `/api/v1/object/:id/proof` menggunakan `curl` dan verifikasi bahwa Merkle proof lolos verifikasi secara matematis.
