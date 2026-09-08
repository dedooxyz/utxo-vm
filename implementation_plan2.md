# Fix Audit Issues + Unify Docker Build

## Konteks

`docker-compose.yml` sudah benar: menggunakan single binary `utxo-vmd` dari Rust.
**Masalahnya**: `Dockerfile` (yang lama) masih build Node.js+TypeScript indexer, sedangkan `Dockerfile.vmd` hanya runtime image tanpa build stage.

Perlu: satu Dockerfile yang compile Rust lalu hasilkan image production kecil.

## Perubahan yang Akan Dilakukan

---

### 1. Fix `core-vm/src/runtime.rs` — C2: mweb_peg_outs hilang dari ExecutionResult

#### [MODIFY] [runtime.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/core-vm/src/runtime.rs)
- Tambah field `mweb_peg_outs: Vec<StealthSettlement>` ke `ExecutionResult`
- Ambil `data.mweb_peg_outs` di `deploy()` dan `execute()`

---

### 2. Fix `core-vm/src/lib.rs` — Ekspor `mweb_peg_outs` via state

#### [MODIFY] [lib.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/core-vm/src/lib.rs)
- Re-export `ExecutionResult` sudah via `pub use runtime::ExecutionResult` — tidak perlu perubahan setelah runtime.rs difix

---

### 3. Fix `core-vm/src/runtime.rs` — C1: Sambungkan GasMeter ke host function costs

#### [MODIFY] [runtime.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/core-vm/src/runtime.rs)
- Tambah inline gas charge di `host_emit_event` (per-byte) dan `host_create_object`, `host_stealth_settle`, `host_mweb_peg_out`
- Menggunakan `store.consume_fuel()` sebagai proxy (karena fuel already active) — deduct additional fuel per host call

---

### 4. Fix `core-vm/src/runtime.rs` — C3: Ganti println! dengan tracing

#### [MODIFY] [runtime.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/core-vm/src/runtime.rs)
- Ganti `println!("[VM] AssemblyScript abort called")` dengan `tracing::warn!`
- Tambah `tracing` ke dependencies core-vm

---

### 5. Fix `node/cross_chain/verifier.rs` — X1 (KRITIS): Tambah ECDSA verify

#### [MODIFY] [verifier.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/cross_chain/verifier.rs)
- `verify_attestation_signatures()` harus memanggil `ConsensusManager::verify_attestation()` untuk setiap attestasi
- Sekarang hanya cek string non-empty — tidak ada crypto!

---

### 6. Fix `node/p2p/node.rs` — P1: Gossipsub MessageId deterministic

#### [MODIFY] [node.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/p2p/node.rs)
- Ganti `DefaultHasher` dengan SHA256 dari `sha2` untuk message ID function

---

### 7. Fix `node/rpc/server.rs` — R1: Rate limiter gunakan AppState value

#### [MODIFY] [server.rs](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/packages/node/src/rpc/server.rs)
- Pindahkan `RateLimiter` ke dalam `AppState` sebagai `Arc<RateLimiter>`
- Buat rate limiter dari `state.rate_limit_rps` saat `create_router` dipanggil

---

### 8. Rebuild `Dockerfile.vmd` — Multi-stage Rust build

#### [MODIFY] [Dockerfile.vmd](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/Dockerfile.vmd)
- Stage 1: `rust:1.82-bookworm` — compile `utxo-vmd` release binary
- Stage 2: `debian:bookworm-slim` — runtime image minimal (hanya binary + ca-certs + curl)
- `docker-compose.yml` diubah untuk build dari `Dockerfile.vmd` alih-alih mount binary

---

### 9. Update `docker-compose.yml`

#### [MODIFY] [docker-compose.yml](file:///home/sena/Documents/DedooProjects/PSOBProjects/utxo-vm/docker-compose.yml)
- Ganti `image: debian:bookworm-slim` + mount binary dengan `build: { context: ., dockerfile: Dockerfile.vmd }`
- Tambah environment variable yang lengkap (DB_PATH, LOG_LEVEL, dll.)

## Verification Plan

### Automated
```bash
cargo check          # Zero errors
cargo test           # All tests pass
```

### Docker Build
```bash
docker build -f Dockerfile.vmd -t utxo-vmd:local .
docker-compose up --build
curl http://localhost:9773/health
```
