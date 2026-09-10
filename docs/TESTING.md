# UTXO-VM Test Suite

> **Read this before running or modifying tests.**
>
> This document describes every test file, what it covers, how to run it,
> and the live testnet results as of the last run.

---

## 1. How to Run Tests

### Unit + Integration Tests (offline, no testnet required)

```bash
# Full workspace (all crates, all default tests)
cargo test --workspace

# With experimental-scripts feature (OP_CAT covenants)
cargo test --workspace --features experimental-scripts

# With experimental-zk feature (mock ZK proofs, test-only)
cargo test --workspace --features experimental-zk

# Individual crates
cargo test -p utxo-core-vm          # Core VM runtime
cargo test -p utxo-vmd --lib        # Node library (consensus, scanner, storage, p2p)
cargo test -p utxo-vmd --test consensus_tests
cargo test -p utxo-vmd --test processor_tests
cargo test -p utxo-vmd --test rpc_tests
cargo test -p utxo-vmd --test smt_tests
cargo test -p utxo-vmd --test storage_tests
cargo test -p utxo-vmd --test p2p_tests
cargo test -p utxo-vmd --test cross_chain_tests
```

### Live Testnet Tests (spend real tJKC, require network)

```bash
# All live tests are marked #[ignore] — must pass --ignored flag
cargo test -p utxo-vmd --test live_testnet -- --nocapture --ignored
cargo test -p utxo-vmd --test live_write_testnet -- --nocapture --ignored
cargo test -p utxo-vmd --test live_broadcast -- --nocapture --ignored
cargo test -p utxo-vmd --test live_mint_call -- --nocapture --ignored
cargo test -p utxo-vmd --test live_contracts -- --nocapture --ignored
cargo test -p utxo-vmd --test live_quorum_equivocation -- --nocapture --ignored
cargo test -p utxo-vmd --test live_vault_challenge -- --nocapture --ignored
```

### Scanner Daemon (live, manual)

```bash
cargo run -p utxo-vmd -- \
  --chain JKC_TESTNET \
  --electrs-url https://jkc-testnet-api.s3na.xyz \
  --db-path /tmp/utxovm-test/test.redb \
  --start-height 177600 \
  --sync-interval-secs 10 \
  --p2p-port 2299 \
  --rpc-port 9778 \
  --log-level info
```

---

## 2. Test Files

### Offline Unit / Integration Tests

| File | Tests | What It Covers |
|:--|:--|:--|
| `packages/core-vm/tests/runtime_tests.rs` | 12 | WASM execution, fuel metering, abort traps, code hash validation, deterministic replay, SIMD rejection, AssemblyScript contract execution |
| `packages/node/src/consensus/challenge.rs` (inline) | 6 | Challenge tx construction: proof validation, watcher threshold, evidence output, tx serialization, bond/fee checks |
| `packages/node/src/consensus/l1_scripts.rs` (inline) | 14 | Script builders: vault tree, challenge leaf (BIP-342), operator unbond leaf, silence escape, softfork status, tagged hashes, push encoding |
| `packages/node/src/consensus/attestation.rs` (inline) | 4 | Attestation signing, verification, equivocation detection, fail-closed on no validators |
| `packages/node/src/consensus/covenants.rs` (inline) | 2 | Staking script, slashing script (experimental-scripts feature) |
| `packages/node/src/consensus/fee_distribution.rs` (inline) | 5 | Fee calculation, indexer fee, operator shares, challenged operator, summary |
| `packages/node/tests/consensus_tests.rs` | 2 | End-to-end consensus: attestation + equivocation |
| `packages/node/tests/processor_tests.rs` | 3 | Block processor: envelope parsing, state transitions, reorg handling |
| `packages/node/tests/rpc_tests.rs` | 1 | RPC server: chain info endpoint |
| `packages/node/tests/smt_tests.rs` | 2 | Sparse Merkle Tree: insert, proof, verify |
| `packages/node/tests/storage_tests.rs` | 2 | Redb storage: put/get, rollback |
| `packages/node/tests/p2p_tests.rs` | 1 | P2P node: identity, listen |
| `packages/node/tests/cross_chain_tests.rs` | 2 | Bridge: lock/mint, relay verify |

### Live Testnet Tests (all `#[ignore]` by default)

| File | Tests | What It Covers | Spends tJKC? |
|:--|:--|:--|:--|
| `live_testnet.rs` | 8 | Read-only: tip height, block hash, historical tx fetch, block scan, mempool, softfork status | No |
| `live_write_testnet.rs` | 3 | Build (unsigned) deploy tx, build challenge tx from equivocation, verify historical envelope | No |
| `live_broadcast.rs` | 1 | Build + sign + broadcast a real inscription tx to testnet | Yes (~300 sats) |
| `live_mint_call.rs` | 5 | Mint SOT token, call transfer, post attestation, equivocation + challenge build, block scan | Yes (~1200 sats) |
| `live_contracts.rs` | 4 | Mint NFT (UTX721), NativeVault (lock sats), AtomicSwap order, verify contract txs | Yes (~1100 sats) |
| `live_quorum_equivocation.rs` | 2 | 3-of-3 validator quorum + post on-chain, equivocation double-sign + challenge build | Yes (~600 sats) |
| `live_vault_challenge.rs` | 1 | Deploy P2TR vault with bond, build + broadcast challenge slash tx | Yes (~100,000 sats bond) |

---

## 3. Test Categories

### 3.1 Core VM Tests (`utxo-core-vm`)

Tests in `packages/core-vm/tests/runtime_tests.rs`:

| Test | What It Verifies |
|:--|:--|
| `test_runtime_initialization` | VmRuntime can be created with default config |
| `test_runtime_hash_incorporates_wasmtime_version` | Runtime hash includes Wasmtime version pin |
| `test_code_hash_calculation` | SHA-256 of WASM bytes is deterministic |
| `test_runtime_version_uses_pinned_wasmtime` | Version string matches pinned Wasmtime 18.0.4 |
| `test_bad_wasm_hash_rejected` | Mismatched code hash is rejected before execution |
| `test_simd_rejected_by_deterministic_config` | SIMD instructions rejected in deterministic mode |
| `test_fuel_gas_metering_out_of_gas` | Fuel exhaustion traps and reverts state |
| `test_fuel_exhaustion_no_state_write` | No state write occurs when fuel runs out |
| `test_wat_host_functions_execution` | Host functions (`env` namespace) are callable |
| `test_abort_traps` | AssemblyScript `abort()` traps the instance |
| `test_deterministic_replay_identical_root` | Two independent runs produce identical state root |
| `test_compiled_assemblyscript_wasm_execution` | Compiled SOT contract deploys + transfers correctly |

### 3.2 Consensus Tests

**Challenge module** (`packages/node/src/consensus/challenge.rs`):

| Test | What It Verifies |
|:--|:--|
| `test_pad_to_32` | Byte padding to 32 bytes for root hashing |
| `test_build_evidence_output` | Evidence OP_RETURN contains `utxovm:challenge` tag |
| `test_build_challenge_transaction_success` | Full challenge tx builds from valid equivocation proof |
| `test_build_challenge_transaction_rejects_invalid_proof` | Invalid proof is rejected |
| `test_build_challenge_transaction_rejects_insufficient_watchers` | Below-threshold watcher sigs rejected |
| `test_build_challenge_transaction_rejects_bond_below_fee` | Bond < fee is rejected |

**Attestation module** (`packages/node/src/consensus/attestation.rs`):

| Test | What It Verifies |
|:--|:--|
| `test_sign_and_verify_attestation` | ECDSA sign + verify roundtrip |
| `test_equivocation_detection_and_slashing_proof` | Double-sign detected, proof stored |
| `test_fail_closed_no_validators_rejects_attestation` | No validators → attestation rejected |

**L1 Scripts** (`packages/node/src/consensus/l1_scripts.rs`):

| Test | What It Verifies |
|:--|:--|
| `test_build_vault_script_tree_model_b` | Vault tree: operator leaf + challenge leaf, tagged hashes |
| `test_build_committee_challenge_leaf` | BIP-342: individual CHECKSIG (not CHECKMULTISIG), x-only pubkeys |
| `test_build_operator_unbond_leaf` | CSV + CHECKSIG, no CLTV |
| `test_build_challenge_claim_script_tree` | Claim leaf (CSV) + rebut leaf (immediate) |
| `test_build_silence_escape_script` | CSV + CHECKSIG escape path |
| `test_build_seal_spend_output` | OP_RETURN seal-spend envelope |
| `test_build_batch_commitment_output` | OP_RETURN batch commitment |
| `test_assert_chain_supports_bonding_pass` | CSV + Taproot → bonding OK |
| `test_assert_chain_supports_bonding_fails_without_csv` | No CSV → bonding rejected |
| `test_assert_chain_supports_bonding_fails_without_taproot` | No Taproot → bonding rejected |
| `test_no_cltv_in_any_script` | No CLTV in any bonding script (testnet) |
| `test_no_op_cat_in_bonding_scripts` | No OP_CAT in bonding scripts (default feature) |
| `test_tagged_hash` | BIP-341 tagged hash differs from plain SHA-256 |
| `test_push_minimal_uint` | Minimal uint encoding for script numbers |

### 3.3 Live Testnet Tests

All live tests connect to `https://jkc-testnet-api.s3na.xyz` (JKC testnet).

**Read-only tests** (`live_testnet.rs`):

| Test | What It Verifies |
|:--|:--|
| `live_testnet_tip_height` | Tip > 177,000 (chain is live) |
| `live_testnet_block_hash` | Block hash fetch works |
| `live_testnet_historical_deploy_tx_exists` | Historical deploy tx `6a69...` has 3 outputs |
| `live_testnet_historical_transfer_tx_exists` | Historical transfer tx `4290...` has 3 outputs |
| `live_testnet_historical_batch_tx_exists` | Historical batch tx `a9de...` has 2 outputs |
| `live_testnet_scan_recent_blocks_for_utxovm_envelopes` | Scans last 10 blocks for utxovm tags |
| `live_testnet_softfork_status_via_rpc` | Queries softfork status from JKC RPC |
| `live_testnet_mempool_not_empty` | Mempool endpoint returns HTTP 200 |

**Write tests** (broadcast real transactions):

| Test | TXID | Operation |
|:--|:--|:--|
| `live_broadcast_inscription_tx` | `1a2969aa25acc042...` | Deploy counter inscription |
| `live_mint_sot_token` | `a7936cc74a6ed94c...` | Mint SOT token (TestJKC, 1M supply) |
| `live_call_transfer` | `a37f057fe82971aa...` | Call transfer to mscKWCFZ |
| `live_post_attestation` | `0652f163e9c3b581...` | Post signed attestation on-chain |
| `live_mint_nft_utx721` | `87ec51546e8fcf36...` | Mint NFT (UTX721 #1) |
| `live_native_vault_lock_sats` | `cc5e52c29a2db224...` | NativeVault lock 0.01 tJKC |
| `live_atomic_swap_order` | `c5eabe16b452f6dc...` | AtomicSwap order (NFT for 1 tJKC) |
| `live_quorum_3_validators_same_height` | `ebf72d8ddf6a017b...` | 3-of-3 quorum attestation posted |
| `live_deploy_p2tr_vault_and_challenge` | `4cba01ccfd6c4aa2...` (vault) | Deploy P2TR vault with 100k sat bond |
| | `76ee0e2f78ea0e7b...` (slash) | **Challenge slash — bond slashed!** |

---

## 4. Live Testnet Results (as of 2026-09-10)

### Network Status
- **Chain**: JKC Testnet
- **Tip**: ~177,650
- **Electrs**: `https://jkc-testnet-api.s3na.xyz`
- **Softforks**: CSV active (h=120k), SegWit active (h=140k), Taproot active (h=160k), OP_CAT active, CLTV NOT active on testnet, MWEB not active

### Wallet
- **Address**: `muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi`
- **Private Key (hex)**: stored in `/home/sena/Documents/DedooProjects/PSOBProjects/jkc-testnet-wallet.md`
- **Balance**: ~50 tJKC (4,999,000,000 sats)
- Full wallet details: see `jkc-testnet-wallet.md` (parent directory)

### Broadcast Transactions (10 confirmed on testnet)

| # | Type | Tag | TXID |
|:--|:--|:--|:--|
| 1 | Deploy | `utxovm:deploy` | `1a2969aa25acc042902fa3c87223f5bb693c220c6f37a5414128421560498fd2` |
| 2 | Mint SOT | `utxovm:deploy` | `a7936cc74a6ed94c2656c3cf35d1150f6e123d550734b5531a03656e86039a95` |
| 3 | Call transfer | `utxovm:call` | `a37f057fe82971aa89f355f7f5f4600429af3b2278adbb0b4f21f462cefd7844` |
| 4 | Attestation | `utxovm:batch` | `0652f163e9c3b581ff852a4eb0fe2430f14b960285ca155ded766e519dfcec97` |
| 5 | Mint NFT | `utxovm:deploy` | `87ec51546e8fcf36c66d71f78d623294fa14cb0b960e354c2ab0f3c7ca6db06a` |
| 6 | NativeVault | `utxovm:deploy` | `cc5e52c29a2db2248b62f58bb1da00c2c4ceb249806230fcb889f8706c220f8d` |
| 7 | AtomicSwap | `utxovm:deploy` | `c5eabe16b452f6dc3bf337354b9433f00a478f91bb5f38019fc4d9064fe8f51c` |
| 8 | Quorum | `utxovm:quorum` | `ebf72d8ddf6a017b74bc77cefaad40d395503c02794b05b59f732a3b8a225a8e` |
| 9 | P2TR Vault | `utxovm:vault` | `4cba01ccfd6c4aa203faf8c0e93df2cece8b94846059efcd10696dc97baeab10` |
| 10 | **Challenge Slash** | `utxovm:challenge` | `76ee0e2f78ea0e7b1fe4782abdba3ee0f6f19620f52c248a35cb5061bb97181f` |

All txs verifiable at: `https://jkc-testnet-api.s3na.xyz/tx/<txid>`

### Scanner Daemon Test

- Started `utxo-vmd` against live testnet
- Scanned blocks 177,599 → 177,646
- RPC endpoints verified: `/api/v1/chain/info`, `/api/v1/state-root`, `/api/v1/objects`, `/api/v1/consensus/slashing-proofs`
- P2P identity loaded: `12D3KooWE2eX822K2gRu74yjosXYYeVykuiQHsrJEjVC6ruehrhH`
- State root: `bbde6793dc0d7247bd110d9c38cee4c05698923175cf59fa3dda499da7f167b0`
- Note: scanner's envelope parser expects `OP_FALSE OP_IF "utxovm" ...` format; live test txs used simplified `OP_RETURN "utxovm:deploy"` format

---

## 5. Production Code Fixes from Live Testing

Live testing on JKC testnet revealed three BIP-342 compliance issues in `l1_scripts.rs`:

| Issue | Root Cause | Fix |
|:--|:--|:--|
| `OP_CHECKMULTISIG` in tapscript | BIP-342 disables CHECKMULTISIG in tapscript | Replaced with individual `OP_CHECKSIG` + `OP_ADD` + `OP_EQUAL` for M-of-N threshold |
| 33-byte compressed pubkeys | BIP-342 tapscript CHECKSIG expects 32-byte x-only pubkeys | Strip parity prefix: `pubkey[1..]` (32 bytes) |
| `TapSighashType::Default` for script-path | Default (0x00) is for key-path; script-path needs explicit type | Use `TapSighashType::All` (0x01) for script-path spends |

These fixes are in `packages/node/src/consensus/l1_scripts.rs` and are covered by unit tests.

---

## 6. Test Count Summary

| Category | Files | Tests | Status |
|:--|:--|:--|:--|
| Core VM | 1 | 12 | ✅ All pass |
| Node lib (inline) | 6 modules | 36 | ✅ All pass |
| Node integration | 6 files | 12 | ✅ All pass |
| Live testnet (ignored) | 7 files | 24 | ✅ All pass |
| **Total** | **20** | **84** | **0 failed** |

With `--features experimental-scripts`: +3 covenant tests (54 total node lib tests).
With `--features experimental-zk`: mock ZK tests (test-only).

---

## 7. Key Test Invariants (must always hold)

1. **Deterministic replay**: Same WASM + witness → identical state root (2 independent runs)
2. **Fuel exhaustion reverts**: No state write when fuel runs out
3. **Abort traps**: `env.abort` traps the instance via `Err(...)`
4. **Bad WASM hash rejected**: Mismatched code hash → execution rejected
5. **Equivocation detection**: Same validator, same height, different roots → slashing proof
6. **Equivocation proof verification**: Cryptographic signature verification on both attestations
7. **Fail-closed**: No registered validators → attestation rejected
8. **BIP-342 compliance**: No `OP_CHECKMULTISIG` in tapscript leaves; x-only pubkeys only
9. **No CLTV on testnet**: All timelocks use CSV (BIP-68), not CLTV (BIP-65)
10. **No OP_CAT in default bonding scripts**: OP_CAT only behind `experimental-scripts` feature
11. **Challenge slash works on testnet**: P2TR script-path spend with Schnorr signature confirmed on JKC testnet

---

## 8. Adding New Tests

When adding new tests:

1. **Unit tests**: Add `#[cfg(test)] mod tests` block in the source file
2. **Integration tests**: Add a new file in `packages/node/tests/`
3. **Live testnet tests**: Add `#[ignore]` attribute — never run live tests in CI
4. **Update this document**: Add the new test file and what it covers
5. **Update test count**: Adjust the counts in section 6

### Live test boilerplate

```rust
use utxo_vmd::scanner::electrs::ElectrsClient;

const ELECTRS_URL: &str = "https://jkc-testnet-api.s3na.xyz";

#[tokio::test]
#[ignore]
async fn my_live_test() {
    let client = ElectrsClient::new(ELECTRS_URL.to_string()).await;
    // ... test logic ...
}
```

### Wallet for live tests

Private key and address are in `/home/sena/Documents/DedooProjects/PSOBProjects/jkc-testnet-wallet.md`.
Never commit private keys to the repository.
