# UTXO-VM Court (L1 Bonding/Slashing)

## What L1 checks
- Bond UTXO is P2TR with a 2-leaf script tree (Model B, committee-gated).
- Leaf 1 (unbond): `<delay> CSV DROP <operator_xonly_pubkey> CHECKSIG`.
- Leaf 2 (challenge): `<xonly_pk1> CHECKSIG <xonly_pk2> CHECKSIGADD ... <xonly_pkN> CHECKSIGADD <M> OP_EQUAL` — M-of-N watcher committee (BIP-342: individual CHECKSIG + CHECKSIGADD, not CHECKMULTISIG).
- Challenge UTXO (second stage): claim leaf (CSV delay + challenger x-only sig), rebut leaf (operator x-only sig, no timelock).
- No CLTV, no OP_CAT. CSV + Taproot only, verified at startup (one-time hard fail).
- BIP-342 compliance: all tapscript leaves use 32-byte x-only pubkeys and `TapSighashType::All` for script-path spends.

## What operators check
- Re-execute WASM deterministically; sign `(chain, height, block_hash, state_root)`.
- Signed payload hash: `SHA256("UTXO_VM_ATTESTATION_V1" || len-prefixed fields)`. Versioned domain separator; changing it is consensus-breaking and requires coordinated rollout.
- Quorum result = state_root with most attestations (threshold met).
- Scanner monitors L1 block hash continuity and automatically rolls back state via `rollback_to_block` on chain reorgs.

## How a liar loses JKC
- Operator signs a divergent state_root.
- Quorum result is built from supporting attestations + Merkle root.
- `QuorumDivergenceProof` captures operator attestation vs quorum root.
- Watcher committee (M-of-N) co-signs a challenge tx spending the bond to a challenge UTXO.
- If operator cannot rebut (no valid counter-proof), challenger claims after CSV delay.

## Live testnet validation (2026-09-10)
- P2TR vault deployed on JKC testnet: `4cba01ccfd6c4aa2...` (100k sat bond).
- Challenge slash tx broadcast + accepted: `76ee0e2f78ea0e7b...` (bond slashed, 99k sats to challenger).
- This is the first real on-chain P2TR script-path spend slashing a bonded operator vault.
- See `docs/TESTING.md` for full test flow and results.

## What is still a stub
- On-chain divergence verification (Bitcoin Script cannot verify off-chain WASM execution).
- Watcher committee selection and key management.

## JKC mainnet softfork status (investigated 2026-09-13)

**Resolved: versionbit 21 warning.** The `unknown new rules activated (versionbit 21)` warning
from `getblockchaininfo` means miners are signaling BIP9 bit 21 (`1 << 21 = 0x200000`) in
block `nVersion`, but `junkcoind` v4.0.3 has no deployment registered for that bit. This is
NOT any of the known softforks — CSV, SegWit, Taproot, and MWEB are all "buried"
(height-gated, not BIP9 version bits), and `testdummy` (the only BIP9 deployment) uses a
different bit and has status "failed". Junkcoin is forked from Litecoin Core v0.21.4, so bit 21
is most likely an inherited deployment slot that is not relevant to Junkcoin's consensus. No
rules are enforced for bit 21, so this warning is benign for UTXO-VM — the bonding path checks
`query_softfork_status()` which reads the `active` field for CSV/Taproot, not version bits.

**Mainnet softfork activation schedule (junkcoin-core v4.0.3, released 2026-09-12):**

| Softfork | Activation height | Status (as of h=1,130,195) |
|:---|:---|:---|
| CSV (BIP 68/112/113) | 1,145,000 | NOT YET ACTIVE (~14,805 blocks / ~10 days away) |
| SegWit (BIP 141/143/147) | 1,145,000 | NOT YET ACTIVE (concurrent with CSV) |
| Taproot (BIP 340/341/342) | 1,155,000 | NOT YET ACTIVE (~24,805 blocks / ~17 days away) |
| Re-enabled Opcodes (OP_CAT etc.) | 1,155,000 | NOT YET ACTIVE (concurrent with Taproot) |
| MWEB | 1,165,000 | NOT YET ACTIVE (~34,805 blocks / ~24 days away) |

**Implication for UTXO-VM:** Bonding (`--bonding-enabled`) is NOT yet possible on JKC mainnet.
`assert_chain_supports_bonding()` will correctly refuse to start because `csv_active=false`
and `taproot_active=false` at the current height. Bonding is testnet-only until CSV activates
at block 1,145,000. Source: [junkcoin-core v4.0.3 release](https://github.com/Junkcoin-Foundation/junkcoin/releases/tag/v4.0.3).
