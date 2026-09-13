# Watcher Committee Selection & Key Management — Design Options

> **Status: research document, NOT a decision.** This file exists to surface concrete
> tradeoffs for a human reviewer. Nothing here is implemented. No code should be written
> against any option below until a human signs off on a specific choice. See `docs/COURT.md`
> "What is still a stub" and the Item B brief.

## Why this is a decision, not an implementation task

The M-of-N challenge tapscript in `l1_scripts.rs::build_committee_challenge_script`
already accepts any N pubkeys and enforces M-of-N via BIP-342
`OP_CHECKSIG ... OP_CHECKSIGADD ... <M> OP_EQUAL`. The script itself is correct.
What's missing is everything *around* it:

- Who are the N pubkeys?
- How were those keys generated, and who holds them?
- How does a watcher actually produce a signature over a *specific* challenge
  transaction at the moment a violation is detected (the static-CLI-signature path
  in `main.rs::run_fraud_watch_pass` cannot work for a real, previously-unknown
  violation — a valid signature must cover the transaction built at detection time)?
- What happens if fewer than M watchers are reachable when a real challenge needs
  co-signing?

Getting any of these wrong does not cause a test failure. It silently becomes
load-bearing security infrastructure for the entire "a liar loses their JKC" trust
model in `docs/TRUST-MODEL.tex`. This is the same class of decision the project
already treated as human-gated for `totalSupply` semantics (Issue 3) and the SMT
rewrite (Issue 12).

---

## 1. Membership: who is eligible to be a watcher, and how is N determined?

### Option 1.A — Watchers = the bonded operator set (`operator_set.rs`)

The same operators who run `utxo-vmd` and post attestations also serve as watchers.
`OperatorSetConfig` already has `min_operators=3, max_operators=10,
honest_threshold=2`.

- **Collusion resistance:** An operator majority that wants to sign a wrong root
  also controls the watchers, so they can refuse to challenge themselves. This is
  the core failure mode. It is *partially* mitigated by the quorum-divergence path
  (`detect_divergence` only fires when *some* operator disagrees with the quorum —
  i.e. at least one honest operator exists), but if the colluding majority is also
  the watcher majority, the honest minority's divergence proof cannot get co-signed.
- **Bootstrapping cost:** Zero. The operator set already exists and is bonded.
- **Honest exit guarantee:** The silence escape path in `l1_scripts.rs
  ::build_silence_escape_script` (CSV-delayed user exit) still works regardless of
  watcher collusion, so users are not trapped — they just can't slash the liar.

### Option 1.B — Separate watcher registration/bonding process

A distinct on-chain registration flow where watchers bond JKC independently of
operators, with their own slashable stake.

- **Collusion resistance:** Strong — watchers have their own skin in the game and
  no shared interest with operators. A watcher that refuses to co-sign a valid
  challenge can itself be slashable (if the design adds a watcher-slashing path).
- **Bootstrapping cost:** High. Requires a new registration script, a new bond
  vault template, a new slashing path for watcher non-liveness, and a working
  permissionless registration flow on day one. This is roughly the same scope as
  the entire operator bonding flow built so far.
- **Honest exit guarantee:** Same as 1.A — silence escape still works.

### Option 1.C — Fixed/permissioned watcher set for v1 mainnet, expandable later

A small set of N watchers chosen by the project/foundation at launch, hardcoded
in a config file or a deploy-time registry, with a documented plan to move to
permissionless registration in a later phase.

- **Collusion resistance:** Depends entirely on who the N are. With N=3
  independent watchers (e.g. foundation, a community-elected operator, an external
  auditor) and M=2, a single operator colluding with one watcher is not enough.
  This is *not* decentralized, and the document must say so plainly.
- **Bootstrapping cost:** Low — no new on-chain registration flow needed, just a
  config file and a documented key-generation ceremony.
- **Honest exit guarantee:** Same — silence escape still works.
- **Honest labeling:** This is a federated/permissioned launch, not a
  decentralized one. That is a legitimate choice for a first mainnet phase as long
  as it's stated in `docs/COURT.md` and `docs/TRUST-MODEL.tex` rather than dressed
  up as decentralized.

---

## 2. Key generation and rotation

### Option 2.A — Individually-held keys, aggregated into the existing tapscript

Each watcher generates and holds their own secp256k1 key. The vault's challenge
leaf lists all N pubkeys: `<pk1> CHECKSIG <pk2> CHECKSIGADD ... <M> OP_EQUAL`.
This is exactly what `build_committee_challenge_script` already builds.

- **Codebase cost:** Zero script-structure change. The existing tapscript, the
  existing `VaultConfig.watcher_pubkeys` field, and the existing
  `build_challenge_transaction` witness construction all work as-is.
- **Threshold:** M-of-N count check, computed by Bitcoin Script itself. No
  off-chain coordination needed to reach the threshold — each watcher signs
  independently and the tx is valid once M signatures are present.
- **Rotation cost:** **High.** A watcher key change requires re-deriving the
  vault's tapscript, which means the bond UTXO must be spent to a new P2TR
  output with the updated key list. For already-funded bonds this is a
  consensus-visible migration (operator must cooperate to move the bond, or a
  separate rotation path must exist). There is no cheap rotation path for
  already-deployed vaults under this option.

### Option 2.B — Threshold-aggregated single key (FROST / MuSig2 DKG)

Watchers run a distributed key generation (DKG) ceremony to produce a single
aggregate public key. The challenge leaf becomes `<aggregate_pk> CHECKSIG` — a
1-of-1-looking script that is actually M-of-N under the hood.

- **Codebase cost:** **Script-structure change.** `build_committee_challenge_script`
  would need a new code path producing a single-key leaf, and
  `build_challenge_transaction` would need to coordinate a FROST/MuSig2 signing
  session rather than collecting M independent signatures. This is a non-trivial
  change to `l1_scripts.rs` and `challenge.rs`, not just a config change.
- **Threshold:** The M-of-N enforcement moves from Bitcoin Script into the
  off-chain FROST/MuSig2 protocol. Bitcoin Script sees a single signature and
  cannot independently verify that M-of-N watchers participated.
- **Rotation cost:** A DKG re-run produces a new aggregate key, which still
  requires spending the bond UTXO to a new P2TR output. Same migration cost as
  2.A for already-funded bonds, plus the DKG ceremony overhead.
- **Privacy:** A single-key leaf is indistinguishable from a regular P2TR spend
  on-chain, hiding the fact that a committee even exists. This is a minor
  privacy win but not a v1 goal.

### Option 2.C — Individually-held keys now, threshold aggregation as a later upgrade

Ship 2.A for v1 (no script change, no DKG dependency), document the upgrade path
to 2.B as a future hard-cutover (same coordinated-rollout treatment as Issue 5's
attestation hash and Issue 12's SMT root format).

- **Codebase cost:** Zero for v1.
- **Rotation cost:** Same as 2.A for v1.
- **Upgrade risk:** A future move to 2.B is a consensus-breaking tapscript
  change requiring all bonded operators to migrate to new vaults. This is
  documented, not hidden.

---

## 3. Signing at detection time — the concrete blocker

`main.rs::run_fraud_watch_pass` currently takes watcher signatures from a static
CLI flag (`--fraud-watch-watcher-sigs`). This **cannot work for a real
violation**: `build_challenge_transaction` constructs a transaction whose
sighash depends on the specific bond UTXO, evidence OP_RETURN, and challenge
output — all of which are only known at detection time. A signature produced
before the tx exists cannot be valid for that tx. Any committee design that
doesn't specify how a watcher signs the *actual* tx at the *actual* moment of
detection is still unimplementable.

### Option 3.A — Per-watcher signing API endpoint

Each watcher runs a small HTTP/gRPC service (or extends `utxo-vmd` with a
watcher RPC endpoint) that accepts `{ unsigned_tx_hex, equivocation_proof }`,
independently re-verifies the proof (`verify_equivocation_proof`), re-derives
the challenge tx from the proof + its own view of the bond UTXO, checks the
sighash matches, and returns a signature over it.

- **Trust model:** Each watcher independently verifies the equivocation before
  signing. A watcher that signs without verifying is slashable (if a
  watcher-slashing path exists — see Option 1.B).
- **Network assumption:** The fraud-watch task must be able to reach M of N
  watcher endpoints within the challenge window. This is a liveness assumption
  on the watcher set, separate from the on-chain silence escape path.
- **Codebase cost:** New RPC endpoint + auth + retry logic. Moderate scope.

### Option 3.B — Off-chain co-signing coordination via the existing P2P layer

The fraud-watch task broadcasts a `ChallengeRequest` over the existing libp2p
gossipsub channel; watchers that receive it verify the proof and respond with a
signature over the constructed tx. The fraud-watch task collects M signatures
and broadcasts the tx.

- **Trust model:** Same as 3.A — each watcher independently verifies.
- **Network assumption:** Uses the existing P2P mesh, no new HTTP service. But
  the existing libp2p layer is best-effort; a watcher that's offline when the
  request is gossiped misses the request. No replay guarantee without a
  separate request-tracking mechanism.
- **Codebase cost:** New gossipsub topic + request/response correlation. The
  existing `P2pCommand::PutContract`/`GetContract` request-response pattern is a
  reasonable template.

### Option 3.C — Manual challenge for v1, automatic detection only

The fraud-watch task detects and logs divergence/equivocation (already
implemented in Item A) but does NOT attempt to co-sign automatically. A human
watcher (or the operator themselves, for a self-slash demo) takes the logged
proof, runs a CLI command to build + sign + broadcast the challenge tx, with
the watcher key(s) available locally to that signer.

- **Trust model:** The human watcher is the verification step. This is exactly
  what the live testnet demonstration in `docs/COURT.md` did (manual TXIDs).
- **Network assumption:** None — no automatic co-signing.
- **Codebase cost:** Zero beyond what Item A already shipped. A CLI subcommand
  to build+sign+broadcast from a proof file would be a small addition.
- **Honest labeling:** This is "automatic detection, manual slashing" — not
  the full "a stranger can punish a lie" automation MISSION.md describes, but it
  is a real, shippable v1 that doesn't pretend to be more than it is.

---

## 4. Liveness / degraded operation

### What the silence escape path already covers

`build_silence_escape_script` (`l1_scripts.rs:659`) lets a user exit their
position after `silence_delay` blocks with no operator or watcher cooperation.
This is the user-funds-safety floor: even if every watcher disappears, users
are not trapped. **This is independent of the watcher committee and does not
need to be re-designed here.**

### What the silence escape path does NOT cover

The silence escape lets users *exit*, but it does not *slash* a lying operator.
If fewer than M watchers are reachable when a real challenge needs co-signing,
the operator keeps their bond. The committee design must specify what happens
in this window:

- **Option 4.A — Accept the gap.** Document that slashing requires M-of-N
  watcher liveness at detection time, and that the silence escape is the only
  user protection when watchers are offline. The operator's bond is not at risk
  in that window. This is honest and simple.
- **Option 4.B — Watcher liveness bonding.** Watchers themselves bond JKC
  (Option 1.B) and are slashable for missing a valid challenge. This requires a
  watcher-non-liveness detection path, which is a new on-chain mechanism.
- **Option 4.C — Lower M for degraded mode.** Allow the challenge to proceed
  with fewer than M signatures after a timeout. This weakens the security model
  (a single watcher could slash unilaterally after the timeout) and is probably
  not worth the complexity for v1.

---

## 5. Comparable systems (brief, cited for what's actually analogous)

- **Lightning watchtowers** (BOLT-eligible penalty watchers): A watchtower
  stores penalty transactions and broadcasts them on detection of a channel
  breach. The watchtower does not co-sign — it holds pre-signed penalty txs.
  This is *not* analogous to UTXO-VM's design, where the challenge tx can only
  be constructed after the violation is observed (the equivocation proof is
  data-of-record, not pre-signed). Cited for contrast, not as a template.
- **BitVM-style bridges** (e.g. the BitVM bridge design by Robin Linus): Use a
  challenge-response game on Bitcoin Script with a fixed set of operators who
  can challenge each other. The operator set is the watcher set (Option 1.A).
  The challenge mechanism is a multi-round interactive game, not a single
  M-of-N co-sign — UTXO-VM's v1 slash is single-round (one challenge tx), so
  the watcher coordination is simpler but the collusion risk is the same.
- **Liquid / federation multisig** (Blockstream Liquid): A fixed federation
  of functionaries with an M-of-N block-signing key. This is the closest
  analogue to Option 1.C + 2.A: a permissioned federation, individually-held
  keys, M-of-N threshold via `CHECKSIGADD`. Liquid documents its federation
  membership publicly and treats rotation as a coordinated event — the same
  honest labeling Option 1.C requires.

---

## Agent's suggested default (NOT a decision)

For a **minimal, honest v1 mainnet launch**:

- **Membership:** Option 1.C — a fixed, publicly-documented set of N=3 watchers
  chosen at launch (e.g. foundation + one community-elected operator + one
  external auditor), M=2. This is a permissioned federation, not decentralized.
  Say so in `docs/COURT.md` and `docs/TRUST-MODEL.tex`.
- **Keys:** Option 2.A — individually-held keys, no DKG, no script-structure
  change. The existing `build_committee_challenge_script` and
  `build_challenge_transaction` work as-is.
- **Signing:** Option 3.C — automatic detection (already shipped in Item A),
  manual challenge for v1. The fraud-watch task logs the proof; a CLI
  subcommand builds + signs + broadcasts from a proof file using a local
  watcher key. This is exactly what the live testnet demo did, just with a
  cleaner CLI. Document that automatic co-signing (3.A or 3.B) is a later
  phase, not a v1 claim.
- **Liveness:** Option 4.A — accept that slashing requires watcher liveness,
  document it, and rely on the silence escape path as the user-protection
  floor when watchers are offline.

This ships the smallest honest v1: detection is automatic, slashing is manual
but real, the watcher set is permissioned and labeled as such, and no new
consensus-breaking script structure or DKG dependency is introduced. Every
piece of this matches what the codebase already does, with no new
load-bearing infrastructure invented by an agent.

---

## What a human needs to decide before any of this becomes code

1. **Membership model** — pick one of 1.A / 1.B / 1.C. If 1.C, name the initial
   watcher set and the rotation policy.
2. **Key generation** — pick one of 2.A / 2.B / 2.C. If 2.B, accept the
   script-structure change to `l1_scripts.rs` and the DKG dependency.
3. **Signing flow** — pick one of 3.A / 3.B / 3.C. If 3.A or 3.B, accept the
   new RPC/P2P endpoint and its liveness assumption. If 3.C, accept that v1
   slashing is manual and update `MISSION.md` accordingly.
4. **Liveness policy** — pick one of 4.A / 4.B / 4.C. If 4.B, accept the new
   watcher-bonding flow. If 4.A, document the gap in `docs/TRUST-MODEL.tex`.
5. **Initial N and M** — concretely, even if 1.C is chosen: how many watchers,
   what threshold, and who are they for the first mainnet phase.
6. **Rotation policy** — even under 2.A (no DKG), decide whether watchers can
   rotate keys at all in v1, and if so, whether that requires a bond migration
   or is simply not supported until a later phase.

Once a human signs off on a specific combination, file the decision as a short
design doc (similar rigor to `docs/TRUST-MODEL.tex`) and *then* an agent can
implement against that spec. Implementation before this decision risks
inventing arbitrary policy that becomes de facto security infrastructure
without anyone having chosen it.
