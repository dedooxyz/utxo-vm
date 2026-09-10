# UTXO-VM Court (L1 Bonding/Slashing)

## What L1 checks
- Bond UTXO is P2TR with a 2-leaf script tree (Model B, committee-gated).
- Leaf 1 (unbond): `<delay> CSV DROP <operator_xonly_pubkey> CHECKSIG`.
- Leaf 2 (challenge): `<xonly_pk1> CHECKSIG <xonly_pk2> CHECKSIG ... OP_ADD OP_ADD ... <M> OP_EQUAL` — M-of-N watcher committee (BIP-342: individual CHECKSIG, not CHECKMULTISIG).
- Challenge UTXO (second stage): claim leaf (CSV delay + challenger x-only sig), rebut leaf (operator x-only sig, no timelock).
- No CLTV, no OP_CAT. CSV + Taproot only, verified at startup (one-time hard fail).
- BIP-342 compliance: all tapscript leaves use 32-byte x-only pubkeys and `TapSighashType::All` for script-path spends.

## What operators check
- Re-execute WASM deterministically; sign `(chain, height, block_hash, state_root)`.
- Quorum result = state_root with most attestations (threshold met).

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
- The `unknown new rules activated (versionbit 21)` warning on JKC mainnet is unresolved — investigate with `junkcoind --version` and `getnetworkinfo`.
