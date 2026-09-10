# UTXO-VM Court (L1 Bonding/Slashing)

## What L1 checks
- Bond UTXO is P2TR with a 2-leaf script tree (Model B, committee-gated).
- Leaf 1 (unbond): `<delay> CSV DROP <operator_pubkey> CHECKSIG`.
- Leaf 2 (challenge): `<M> <pubkey1..N> <N> CHECKMULTISIG` — M-of-N watcher committee.
- Challenge UTXO (second stage): claim leaf (CSV delay + challenger sig), rebut leaf (operator sig, no timelock).
- No CLTV, no OP_CAT. CSV + Taproot only, verified at startup (one-time hard fail).

## What operators check
- Re-execute WASM deterministically; sign `(chain, height, block_hash, state_root)`.
- Quorum result = state_root with most attestations (threshold met).

## How a liar loses JKC
- Operator signs a divergent state_root.
- Quorum result is built from supporting attestations + Merkle root.
- `QuorumDivergenceProof` captures operator attestation vs quorum root.
- Watcher committee (M-of-N) co-signs a challenge tx spending the bond to a challenge UTXO.
- If operator cannot rebut (no valid counter-proof), challenger claims after CSV delay.

## What is still a stub
- On-chain divergence verification (Bitcoin Script cannot verify off-chain WASM execution).
- Watcher committee selection and key management.
- Live broadcast/spend tests on JKC testnet (opt-in, requires private node).
- The `unknown new rules activated (versionbit 21)` warning on JKC mainnet is unresolved — investigate with `junkcoind --version` and `getnetworkinfo`.
