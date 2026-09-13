# UTXO-VM Court (L1 Bonding/Slashing)

## What L1 checks

- Bond UTXO is P2TR with a 2-leaf script tree (Model B).
- Leaf 1 (unbond): `<delay> CSV DROP <operator_xonly_pubkey> CHECKSIG`.
- Leaf 2 (challenge): the **equivocation covenant** — an OP_CAT-based
  tapleaf that verifies two conflicting operator attestations on-chain.
  No watcher committee co-signing is required for this path.
- Challenge UTXO (second stage): claim leaf (CSV delay + challenger
  x-only sig), rebut leaf (operator x-only sig, no timelock).
- CSV + Taproot + disabled-opcode reactivation (OP_CAT) verified at
  startup via `assert_chain_supports_bonding()` (one-time hard fail).
- BIP-342 compliance: all tapscript leaves use 32-byte x-only pubkeys
  and `TapSighashType::All` for script-path spends.

## Equivocation covenant (committee-free)

The challenge leaf (`covenants::build_equivocation_covenant`) uses
OP_CAT to reconstruct each attestation preimage byte-for-byte from
witness-provided length-prefixed fields, OP_SHA256 to hash them, and
OP_CHECKSIGVERIFY to verify the operator's signature. It then checks
`root_1 != root_2` via `OP_EQUAL OP_NOT OP_VERIFY`. Anyone who finds
two conflicting attestations from the same operator can slash the bond
without a watcher committee.

The attestation preimage format (from `serialize_attestation_preimage`):
```
"UTXO_VM_ATTESTATION_V1" || u32(len_chain) || chain
  || u64(height) || u32(len_block_hash) || block_hash
  || u32(len_state_root) || state_root
```

Witness stack (bottom to top):
```
root_1, root_2,
len_chain_field, height_field, len_bh_field_1, len_sr_field_1, sig_1,
len_chain_field, height_field, len_bh_field_2, len_sr_field_2, sig_2,
<script leaf>
```

**Limitation:** BIP-342 `OP_CHECKSIG` verifies against the transaction
sighash, not a stack-provided message. The covenant reconstructs the
attestation preimages on-stack and includes the signatures as evidence,
but full on-chain attestation-signature verification (over the
reconstructed message hash) requires `OP_CHECKSIGFROMSTACK`, which is
not in the JKC re-enabled opcode set. The attestation signatures are
verified off-chain by `verify_equivocation_proof()` before the
challenge transaction is constructed. The on-chain covenant enforces
structural equivocation (same chain/height, different roots) and
commits the evidence to the chain via the witness and an OP_RETURN
evidence output.

## Computation-fraud disputes (still committee-gated)

A single wrong-but-consistent state root cannot be detected by Script
alone — verifying it requires re-executing the WASM transition, which
Script cannot do. This fraud class still needs **Item B**: a watcher
committee that independently re-executes the transition and co-signs a
challenge. The committee path is preserved in `l1_scripts.rs` for this
purpose but is not the equivocation path.

## What operators check

- Re-execute WASM deterministically; sign `(chain, height, block_hash, state_root)`.
- Signed payload hash: `SHA256("UTXO_VM_ATTESTATION_V1" || len-prefixed fields)`.
- Quorum result = state_root with most attestations (threshold met).
- Scanner monitors L1 block hash continuity and rolls back state on reorgs.

## How a liar loses JKC

- **Equivocation:** operator signs two different roots at the same
  chain/height. Any whistleblower constructs the covenant witness from
  the two attestations and slashes the bond. No committee needed.
- **Computation fraud:** operator signs one wrong root. Watcher
  committee (M-of-N) re-executes, co-signs a challenge tx, and slashes
  via the committee path. Operator may rebut; if not, challenger claims
  after CSV delay.

## JKC activation requirements

| Feature | Mainnet | Testnet | Regtest |
|:---|---:|---:|---:|
| CSV + SegWit | 1,145,000 | — | 0 |
| Taproot + disabled-opcode reactivation | 1,155,000 | 160,000 | 0 |
| MWEB | 1,165,000 | — | — |

`assert_chain_supports_bonding()` checks all three (CSV, Taproot,
disabled-opcode reactivation) and refuses to start bonding if any is
missing. Before disabled-opcode reactivation, OP_CAT-containing
tapleaves are OP_SUCCESS (anyone-can-spend) — the startup check
prevents funding a bond before it is safe.

## What is still a stub / unresolved

- On-chain attestation-signature verification over reconstructed message
  hash (requires OP_CHECKSIGFROMSTACK, not in JKC opcode set).
- Watcher committee selection and key management for computation-fraud
  disputes.
- Live testnet broadcast of a real equivocation-covenant spend (planned;
  see `docs/TESTING.md`).
