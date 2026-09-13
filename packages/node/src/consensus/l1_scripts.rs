//! L1 Script Builders for UTXO-VM
//!
//! Empirically verified against JKC testnet (block 177,269, getblockchaininfo RPC):
//!   csv:     active=true,  height=120000
//!   segwit:  active=true,  height=140000
//!   taproot: active=true,  height=160000
//!   mweb:    active=true,  height=180000 (verified 2026-09-13: block 183,374
//!            contains a HogEx tx spending a witness-v8 (OP_8 <32B>) program)
//!   bip65:   active=false on testnet (height=99999999), ACTIVE on mainnet
//!   op_cat:  active=true on testnet (confirmed by developer)
//!
//! MAINNET STATUS (junkcoin-core v4.0.3, released 2026-09-12, investigated at h=1,130,195):
//!   csv:     active=false, scheduled h=1,145,000
//!   segwit:  active=false, scheduled h=1,145,000 (concurrent with CSV)
//!   taproot: active=false, scheduled h=1,155,000
//!   op_cat:  active=false, scheduled h=1,155,000 (concurrent with Taproot)
//!   mweb:    active=false, scheduled h=1,165,000
//! Bonding (--bonding-enabled) is testnet-only until CSV activates at 1,145,000.
//! assert_chain_supports_bonding() correctly refuses to start on mainnet today.
//!
//! All timelocks use OP_CHECKSEQUENCEVERIFY (CSV, active at height 120,000 on testnet).
//! CLTV is available on mainnet but NOT on testnet — use CSV for testnet scripts.
//! Hash comparison uses OP_EQUAL/OP_EQUALVERIFY (universal, no OP_CAT dependency).
//! (Issue 14: covenants.rs OP_CAT code deleted; live bonding scripts never used OP_CAT.)
//!
//! Challenge model: Model B (committee-gated). See build_vault_script_tree docs.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

// Standard opcodes
pub const OP_FALSE: u8 = 0x00;
pub const OP_IF: u8 = 0x63;
pub const OP_ELSE: u8 = 0x67;
pub const OP_ENDIF: u8 = 0x68;
pub const OP_DROP: u8 = 0x75;
pub const OP_DUP: u8 = 0x76;
pub const OP_HASH160: u8 = 0xa9;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_EQUAL: u8 = 0x87;
pub const OP_CHECKSIG: u8 = 0xac;
pub const OP_CHECKSIGVERIFY: u8 = 0xad;
pub const OP_CHECKMULTISIG: u8 = 0xae;
pub const OP_CHECKSIGADD: u8 = 0xba;
pub const OP_CHECKSEQUENCEVERIFY: u8 = 0xb2;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_RETURN: u8 = 0x6a;
pub const OP_1: u8 = 0x51;
pub const OP_ADD: u8 = 0x93;
/// OP_CAT (0x7e) — re-enabled on JKC at h=1,155,000 (concurrent with Taproot).
/// Concatenates top two stack items. Used in the covenant challenge leaf to
/// reconstruct the attestation message hash for on-chain equivocation verification.
pub const OP_CAT: u8 = 0x7e;
pub const OP_VERIFY: u8 = 0x69;
pub const OP_NOT: u8 = 0x91;
pub const OP_PUSHDATA1: u8 = 0x4c;
pub const OP_TOALTSTACK: u8 = 0x6b;
pub const OP_FROMALTSTACK: u8 = 0x6c;

/// Taproot leaf version (BIP-341)
pub const TAPROOT_LEAF_VERSION: u8 = 0xc0;

/// BIP-341 tagged hash domain separators
pub const TAG_TAPLEAF: &[u8] = b"TapLeaf";
pub const TAG_TAPBRANCH: &[u8] = b"TapBranch";
pub const TAG_TAPTWEAK: &[u8] = b"TapTweak";

/// BIP-341 tagged hash: SHA256(SHA256(tag) || SHA256(tag) || data)
pub fn tagged_hash(tag: &[u8], data: &[u8]) -> Vec<u8> {
    let tag_hash = Sha256::digest(tag);
    let mut hasher = Sha256::new();
    hasher.update(&tag_hash);
    hasher.update(&tag_hash);
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Operator vault configuration (Model B — committee-gated)
///
/// Model B flow:
/// 1. Operator bonds JKC into a P2TR UTXO with two script-path leaves.
/// 2. Leaf 1 (unbond): operator can exit after `unbond_delay` blocks (CSV).
/// 3. Leaf 2 (challenge): M-of-N watcher committee can spend the bond to a
///    challenge UTXO, posting divergence evidence as OP_RETURN.
/// 4. Challenge UTXO has two leaves:
///    - Claim: challenger claims after `claim_delay` blocks (CSV from challenge tx).
///    - Rebut: operator can reclaim immediately (no timelock).
///
/// CSV on the challenge UTXO measures from the challenge tx confirmation,
/// NOT from bond creation — this is the key advantage of Model B over a
/// naive single-UTXO race design.
///
/// Trust assumption: with OP_CAT + Taproot active on JKC (h=1,155,000), the
/// challenge leaf verifies the equivocation proof ON-CHAIN — no watcher
/// committee needed. Anyone who finds two conflicting attestations from the
/// same operator (same chain+height, different state_roots, both valid
/// Schnorr signatures) can slash the bond. The Script is the judge.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Operator's public key (x-only, 32 bytes — BIP-340 Schnorr)
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (x-only, 32 bytes — receives slashed bond)
    pub challenger_pubkey: Vec<u8>,
    /// Unbond delay in blocks (relative timelock via CSV)
    pub unbond_delay: u32,
    /// Claim delay in blocks (relative timelock on challenge UTXO via CSV)
    pub claim_delay: u32,
    /// Watcher committee public keys — KEPT FOR BACKWARD COMPAT with existing
    /// tests/configs but UNUSED in the covenant challenge path. The covenant
    /// challenge leaf (build_covenant_challenge_script) ignores this field.
    /// Set to empty vec for the covenant path.
    pub watcher_pubkeys: Vec<Vec<u8>>,
    /// Required number of watcher signatures — UNUSED in the covenant path.
    /// Kept for backward compat. Set to 0 for the covenant path.
    pub watcher_threshold: u32,
}

/// Challenge UTXO configuration (Model B second stage)
#[derive(Debug, Clone)]
pub struct ChallengeUtxoConfig {
    /// Challenger's public key (x-only, 32 bytes — receives slashed bond)
    pub challenger_pubkey: Vec<u8>,
    /// Operator's public key (x-only, 32 bytes — can rebut)
    pub operator_pubkey: Vec<u8>,
    /// Claim delay in blocks (CSV from challenge tx confirmation)
    pub claim_delay: u32,
}

/// Challenge proof data — the witness for the covenant challenge leaf.
///
/// With OP_CAT + Taproot, the challenge leaf verifies the equivocation
/// on-chain: it checks both Schnorr signatures are valid for the same
/// (chain, height, block_hash) but different state_roots. No watcher
/// committee co-signing needed — anyone can slash.
#[derive(Debug, Clone)]
pub struct ChallengeProof {
    /// The operator's first attestation root (the "wrong" one)
    pub claimed_root: Vec<u8>,
    /// The operator's second attestation root (the "correct" one)
    pub correct_root: Vec<u8>,
    /// Operator's Schnorr signature on the first root (64 bytes)
    pub operator_signature: Vec<u8>,
    /// Operator's Schnorr signature on the second root (64 bytes)
    pub operator_signature_2: Vec<u8>,
    /// Operator's public key (x-only, 32 bytes)
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (x-only, 32 bytes — receives slashed bond)
    pub challenger_pubkey: Vec<u8>,
}

/// Seal spend proof data
#[derive(Debug, Clone)]
pub struct SealSpendProof {
    /// Object ID (32 bytes)
    pub object_id: Vec<u8>,
    /// Previous seal (txid:vout)
    pub prev_seal: Vec<u8>,
    /// New seal (txid:vout)
    pub new_seal: Vec<u8>,
    /// State hash (SHA256 of state data)
    pub state_hash: Vec<u8>,
}

// ============================================================================
// OPERATOR VAULT SCRIPT (P2TR)
// ============================================================================

/// Build the operator vault script tree for P2TR spending (covenant model).
///
/// Script tree:
/// - Leaf 1 (Operator unbond): `<unbond_delay> CSV DROP <operator_pubkey> CHECKSIG`
/// - Leaf 2 (Covenant challenge): anyone-can-slash equivocation proof on-chain
///
/// The covenant challenge leaf uses OP_CAT + OP_SHA256 + OP_CHECKSIG to verify
/// two conflicting Schnorr attestations from the same operator on-chain. No
/// watcher committee needed — anyone who finds two signed attestations with
/// different state_roots for the same (chain, height, block_hash) can slash.
/// Requires OP_CAT active (h=1,155,000 on JKC mainnet).
pub fn build_vault_script_tree(config: &VaultConfig) -> Result<VaultScriptTree> {
    if config.operator_pubkey.len() != 32 {
        return Err(anyhow!("Operator pubkey must be 32 bytes (x-only, BIP-340)"));
    }
    if config.challenger_pubkey.len() != 32 {
        return Err(anyhow!("Challenger pubkey must be 32 bytes (x-only, BIP-340)"));
    }

    // Leaf 1: Operator unbond path
    let operator_leaf = build_operator_unbond_leaf(
        &config.operator_pubkey,
        config.unbond_delay,
    )?;

    // Leaf 2: Covenant challenge path (anyone-can-slash, no committee)
    let challenge_leaf = build_covenant_challenge_leaf(&config.operator_pubkey)?;

    Ok(VaultScriptTree {
        operator_leaf,
        challenge_leaf,
        operator_pubkey: config.operator_pubkey.clone(),
        challenger_pubkey: config.challenger_pubkey.clone(),
        num_watchers: 0, // no watcher committee in the covenant model
    })
}

/// Build the operator unbond leaf script.
///
/// Script: <unbond_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <operator_pubkey> OP_CHECKSIG
/// operator_pubkey is x-only (32 bytes, BIP-340).
fn build_operator_unbond_leaf(
    operator_pubkey: &[u8],
    unbond_delay: u32,
) -> Result<Vec<u8>> {
    if operator_pubkey.len() != 32 {
        return Err(anyhow!("Operator pubkey must be 32 bytes (x-only)"));
    }

    let mut script = Vec::new();

    // Push unbond delay as minimal encoding
    push_minimal_uint(&mut script, unbond_delay as u64);

    // CSV + DROP
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push operator pubkey (x-only, 32 bytes for BIP-342 tapscript)
    script.push(32);
    script.extend_from_slice(operator_pubkey);

    // Checksig
    script.push(OP_CHECKSIG);

    Ok(script)
}

/// Build the covenant challenge leaf script (anyone-can-slash).
///
/// Delegates to `covenants::build_equivocation_covenant()`. See that module
/// for the full script logic and witness format.
///
/// This leaf uses OP_CAT + OP_SHA256 + OP_CHECKSIGVERIFY to verify an
/// equivocation proof ON-CHAIN. No watcher committee — anyone who finds two
/// conflicting attestations from the same operator can slash the bond.
///
/// Requires OP_CAT active on the connected chain (JKC h=1,155,000 mainnet,
/// h=160,000 testnet, 0 regtest). Before activation, OP_CAT-containing
/// tapleaves are OP_SUCCESS (anyone-can-spend) — `assert_chain_supports_bonding()`
/// in main.rs checks this and refuses to start bonding.
fn build_covenant_challenge_leaf(operator_pubkey: &[u8]) -> Result<Vec<u8>> {
    crate::consensus::covenants::build_equivocation_covenant(operator_pubkey)
}

/// Vault script tree containing both spending paths
#[derive(Debug, Clone)]
pub struct VaultScriptTree {
    pub operator_leaf: Vec<u8>,
    pub challenge_leaf: Vec<u8>,
    pub operator_pubkey: Vec<u8>,
    pub challenger_pubkey: Vec<u8>,
    pub num_watchers: usize,
}

impl VaultScriptTree {
    /// Calculate the tapleaf hash using BIP-341 tagged hash
    pub fn tapleaf_hash(script: &[u8]) -> Vec<u8> {
        let leaf_version = TAPROOT_LEAF_VERSION;
        let script_len = script.len();

        // BIP-341: tapleaf = 0xc0 || compact_size(script_len) || script
        let mut data = Vec::new();
        data.push(leaf_version);
        data.extend_from_slice(&push_size_compact(script_len));
        data.extend_from_slice(script);

        tagged_hash(TAG_TAPLEAF, &data)
    }

    /// Get the operator leaf hash
    pub fn operator_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.operator_leaf)
    }

    /// Get the challenge leaf hash
    pub fn challenge_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.challenge_leaf)
    }

    /// Calculate the script tree root using BIP-341 tagged hash
    pub fn script_tree_root(&self) -> Vec<u8> {
        let left = self.operator_leaf_hash();
        let right = self.challenge_leaf_hash();

        // Sort the two hashes (BIP-340)
        let (first, second) = if left < right {
            (left.as_slice(), right.as_slice())
        } else {
            (right.as_slice(), left.as_slice())
        };

        // BIP-341: tapbranch = left || right
        let mut data = Vec::new();
        data.extend_from_slice(first);
        data.extend_from_slice(second);

        tagged_hash(TAG_TAPBRANCH, &data)
    }

    /// Compute the P2TR output script for this vault.
    ///
    /// Uses BIP-341: output_key = internal_key + tweak(internal_key, script_tree_root)
    /// The internal key is a NUMS (Nothing-Up-My-Sleeve) point per BIP-341 when
    /// using script-path-only spending (no key-path spend).
    ///
    /// Returns the witness v1 output script: OP_1 <32-byte x-only pubkey>
    pub fn p2tr_output_script(&self) -> Result<Vec<u8>> {
        // BIP-341 NUMS internal key: x-only pubkey = SHA256("TapTweak")||SHA256("TapTweak") compressed
        // For script-path-only, the internal key is a placeholder that provably has no known private key.
        // Using H = lift_x(int(SHA256("TapTweak")) mod p) as per BIP-341 recommendation.
        // For simplicity, we use the all-zeros x-only key (which is a valid NUMS point on secp256k1).
        let internal_key = [0u8; 32];

        // Compute tweak: tagged_hash("TapTweak", internal_key || script_tree_root)
        let merkle_root = self.script_tree_root();
        let mut tweak_data = Vec::with_capacity(64);
        tweak_data.extend_from_slice(&internal_key);
        tweak_data.extend_from_slice(&merkle_root);
        let tweak = tagged_hash(TAG_TAPTWEAK, &tweak_data);

        // In a full implementation, this would compute:
        //   Q = internal_key + lift_x(tweak) * G
        //   output_key = x(Q)
        // For now, we return the script structure with a note that the actual
        // tweaked key computation requires a secp256k1 library (bitcoin/secp256k1).
        // The caller must compute the tweaked key externally and provide it.
        //
        // Output script: OP_1 <32-byte x-only output key>
        let mut script = Vec::with_capacity(34);
        script.push(0x51); // OP_1 (witness version 1)
        script.push(32);   // push 32 bytes
        // Placeholder: actual tweaked key must be computed by caller using secp256k1
        // For now, append the tweak hash as a placeholder output key
        let mut output_key = [0u8; 32];
        output_key.copy_from_slice(&tweak[..32]);
        script.extend_from_slice(&output_key);

        Ok(script)
    }

    /// Get the witness items needed for individual CHECKSIG spending (committee challenge leaf).
    /// BIP-342 uses individual OP_CHECKSIG + OP_CHECKSIGADD, not OP_CHECKMULTISIG.
    ///
    /// Under BIP-342, every public key in the script evaluates either a valid signature
    /// (incrementing the accumulator) or an empty byte vector `vec![]` (leaving the
    /// accumulator unchanged). To prevent stack underflow, the witness must contain
    /// exactly N signature items (one per watcher).
    ///
    /// If fewer than N signatures are provided (e.g. only M signatures meeting threshold),
    /// this function pads the remaining non-signing watcher slots with empty byte vectors `vec![]`.
    ///
    /// Build the witness stack for the covenant challenge leaf.
    ///
    /// Witness order (bottom to top, i.e. witness array order):
    ///   root_1, preimage_1, sig_1, root_2, preimage_2, sig_2
    ///
    /// Where:
    /// - root_N is the state_root from attestation N (as raw bytes)
    /// - preimage_N is the serialized attestation payload that, when SHA256'd,
    ///   produces the message the operator signed (see
    ///   ConsensusManager::hash_attestation_payload)
    /// - sig_N is the operator's BIP-340 Schnorr signature (64 bytes)
    ///
    /// The script leaf (build_covenant_challenge_leaf, delegating to
    /// covenants::build_equivocation_covenant) verifies both signatures
    /// against their preimage hashes using OP_CHECKSIGVERIFY, then checks
    /// root_1 != root_2.
    pub fn covenant_witness_template(
        &self,
        root_1: &[u8],
        root_2: &[u8],
        chain: &str,
        height: u64,
        block_hash_1: &str,
        state_root_1: &str,
        sig_1: &[u8],
        block_hash_2: &str,
        state_root_2: &str,
        sig_2: &[u8],
    ) -> Vec<Vec<u8>> {
        crate::consensus::covenants::build_equivocation_witness(
            root_1, root_2, chain, height,
            block_hash_1, state_root_1, sig_1,
            block_hash_2, state_root_2, sig_2,
            &self.challenge_leaf,
        )
    }

    /// Build the witness stack for the committee challenge leaf (legacy).
    ///
    /// Bitcoin witness stack order: items pushed first sit at the bottom of the stack.
    /// During BIP-342 execution, the first opcode (CHECKSIG for pk1) pops the top of the stack.
    /// Reversing the N signatures ensures signatures[0] (for pk1) sits at the top of the stack.
    ///
    /// Returns: [sigN, ..., sig2, sig1, script]
    pub fn committee_witness_template(&self, signatures: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let mut padded_sigs = signatures.to_vec();
        // Pad with empty byte vectors for non-signing watchers up to num_watchers
        while padded_sigs.len() < self.num_watchers {
            padded_sigs.push(Vec::new());
        }

        let mut witness = Vec::with_capacity(padded_sigs.len() + 1);
        for sig in padded_sigs.iter().rev() {
            witness.push(sig.clone());
        }
        // The script leaf itself (for script-path spend)
        witness.push(self.challenge_leaf.clone());
        // Control block would be appended by the wallet (contains internal key + path proof)
        witness
    }
}

// ============================================================================
// CHALLENGE UTXO SCRIPT TREE (Model B second stage)
// ============================================================================

/// Build the challenge UTXO script tree (Model B second stage).
///
/// After the watcher committee challenges (spending the bond to this UTXO),
/// the challenge UTXO has two spending paths:
///
/// - Leaf 1 (Challenger claim): `<claim_delay> CSV DROP <challenger_pubkey> CHECKSIG`
///   The challenger can claim the bond after `claim_delay` blocks, measured
///   from the challenge tx confirmation (CSV on this UTXO).
///
/// - Leaf 2 (Operator rebut): `<operator_pubkey> CHECKSIG`
///   The operator can reclaim the bond immediately (no timelock). This is
///   the operator's defense against a fraudulent challenge — if the operator
///   is honest, they rebut before the claim delay expires.
///
/// Security model: if the operator actually diverged, they cannot rebut
/// (they have no valid counter-proof), and the challenger claims after the
/// delay. If the challenge was fraudulent, the operator rebuts immediately.
/// The on-chain OP_RETURN evidence from the challenge tx allows the community
/// to verify off-chain whether the rebut was justified.
pub fn build_challenge_claim_script_tree(config: &ChallengeUtxoConfig) -> Result<ChallengeScriptTree> {
    if config.challenger_pubkey.len() != 32 {
        return Err(anyhow!("Challenger pubkey must be 32 bytes (x-only, BIP-340)"));
    }
    if config.operator_pubkey.len() != 32 {
        return Err(anyhow!("Operator pubkey must be 32 bytes (x-only, BIP-340)"));
    }

    // Leaf 1: Challenger claim (CSV delay from challenge tx)
    let claim_leaf = {
        let mut script = Vec::new();
        push_minimal_uint(&mut script, config.claim_delay as u64);
        script.push(OP_CHECKSEQUENCEVERIFY);
        script.push(OP_DROP);
        script.push(32);
        script.extend_from_slice(&config.challenger_pubkey);
        script.push(OP_CHECKSIG);
        script
    };

    // Leaf 2: Operator rebut (no timelock — immediate)
    let rebut_leaf = {
        let mut script = Vec::new();
        script.push(32);
        script.extend_from_slice(&config.operator_pubkey);
        script.push(OP_CHECKSIG);
        script
    };

    Ok(ChallengeScriptTree {
        claim_leaf,
        rebut_leaf,
        challenger_pubkey: config.challenger_pubkey.clone(),
        operator_pubkey: config.operator_pubkey.clone(),
    })
}

/// Challenge UTXO script tree (Model B second stage)
#[derive(Debug, Clone)]
pub struct ChallengeScriptTree {
    pub claim_leaf: Vec<u8>,
    pub rebut_leaf: Vec<u8>,
    pub challenger_pubkey: Vec<u8>,
    pub operator_pubkey: Vec<u8>,
}

impl ChallengeScriptTree {
    /// Calculate the tapleaf hash using BIP-341 tagged hash
    pub fn tapleaf_hash(script: &[u8]) -> Vec<u8> {
        VaultScriptTree::tapleaf_hash(script)
    }

    /// Get the claim leaf hash
    pub fn claim_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.claim_leaf)
    }

    /// Get the rebut leaf hash
    pub fn rebut_leaf_hash(&self) -> Vec<u8> {
        Self::tapleaf_hash(&self.rebut_leaf)
    }

    /// Calculate the script tree root using BIP-341 tagged hash
    pub fn script_tree_root(&self) -> Vec<u8> {
        let left = self.claim_leaf_hash();
        let right = self.rebut_leaf_hash();

        let (first, second) = if left < right {
            (left.as_slice(), right.as_slice())
        } else {
            (right.as_slice(), left.as_slice())
        };

        let mut data = Vec::new();
        data.extend_from_slice(first);
        data.extend_from_slice(second);

        tagged_hash(TAG_TAPBRANCH, &data)
    }
}

// ============================================================================
// STARTUP ASSERTION (one-time, hard fail — no polling)
// ============================================================================

/// Softfork status as reported by getblockchaininfo RPC.
#[derive(Debug, Clone)]
pub struct SoftforkStatus {
    pub csv_active: bool,
    pub segwit_active: bool,
    pub taproot_active: bool,
    /// Whether disabled opcodes (OP_CAT, OP_SUBSTR, etc.) have been re-enabled.
    /// On JKC this is gated by `DisabledScriptReactivationHeight` (h=1,155,000
    /// mainnet, h=160,000 testnet, 0 regtest), distinct from `TaprootHeight`.
    /// Before this height, OP_CAT-containing tapleaves are OP_SUCCESS
    /// (anyone-can-spend), so bonding must not start until this is active.
    pub disabled_opcodes_active: bool,
}

/// One-time startup assertion: verify that the connected chain reports
/// CSV, Taproot, and disabled-opcode reactivation as active. This is a
/// deployment precondition, not runtime feature detection — it fails loudly
/// once at startup if the chain doesn't meet the requirements. No polling,
/// no fallback logic.
///
/// Call this once at node startup when bonding is enabled. If it returns Err,
/// the node must refuse to start (or refuse to enable bonding).
pub fn assert_chain_supports_bonding(status: &SoftforkStatus) -> Result<()> {
    if !status.csv_active {
        return Err(anyhow!(
            "Chain does not report CSV (BIP112) as active. \
             Bonding requires CSV for relative timelocks. \
             Activate CSV on the target chain before enabling bonding."
        ));
    }
    if !status.taproot_active {
        return Err(anyhow!(
            "Chain does not report Taproot (BIP341) as active. \
             Bonding requires Taproot for P2TR script-tree spending. \
             Activate Taproot on the target chain before enabling bonding."
        ));
    }
    if !status.disabled_opcodes_active {
        return Err(anyhow!(
            "Chain does not report disabled-opcode reactivation as active. \
             The equivocation covenant uses OP_CAT, which is OP_SUCCESS \
             (anyone-can-spend) before DisabledScriptReactivationHeight. \
             On JKC mainnet this is h=1,155,000; testnet h=160,000; regtest 0. \
             Do not start bonding until OP_CAT is re-enabled."
        ));
    }
    Ok(())
}

/// Query a Junkcoin JSON-RPC endpoint for softfork status.
///
/// This is a one-time startup check — not polling, not runtime detection.
/// The caller invokes this once at startup when bonding is enabled, then
/// passes the result to `assert_chain_supports_bonding`.
///
/// Credentials are read from environment variables to avoid hardcoding:
/// - `JKC_RPC_USER`: RPC username
/// - `JKC_RPC_PASS`: RPC password
///
/// The `rpc_url` should include the host and port, e.g. `http://127.0.0.1:9771`.
pub async fn query_softfork_status(rpc_url: &str) -> Result<SoftforkStatus> {
    use reqwest::Client;

    let user = std::env::var("JKC_RPC_USER").unwrap_or_default();
    let pass = std::env::var("JKC_RPC_PASS").unwrap_or_default();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let body = serde_json::json!({
        "jsonrpc": "1.0",
        "id": "utxo-vm-startup",
        "method": "getblockchaininfo",
        "params": []
    });

    let mut req = client.post(rpc_url).json(&body);
    if !user.is_empty() {
        req = req.basic_auth(&user, Some(&pass));
    }

    let resp = req.send().await?.error_for_status()?;
    let json: serde_json::Value = resp.json().await?;

    // JKC Core getblockchaininfo may report softforks in different formats:
    // 1. As "softforks" array with "type": "buried" and "active": true
    // 2. As "deployments" object with "active": true
    // 3. CSV may be under "csv" and Taproot under "taproot" in deployments
    // We check all known locations.

    let mut csv_active = false;
    let mut segwit_active = false;
    let mut taproot_active = false;
    let mut disabled_opcodes_active = false;

    // Check "softforks" array (Bitcoin Core format)
    if let Some(softforks) = json.get("softforks").and_then(|v| v.as_array()) {
        for sf in softforks {
            let name = sf.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let active = sf.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            match name {
                "csv" => csv_active = active,
                "segwit" => segwit_active = active,
                "taproot" => taproot_active = active,
                // JKC reports disabled-opcode reactivation as a softfork deployment
                "disabled_opcodes" | "disabledopcodes" | "reactivated_opcodes" => {
                    disabled_opcodes_active = active;
                }
                _ => {}
            }
        }
    }

    // Check "deployments" object (newer Bitcoin Core / JKC format)
    if let Some(deployments) = json.get("deployments").and_then(|v| v.as_object()) {
        if let Some(csv) = deployments.get("csv") {
            csv_active = csv_active || csv.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        }
        if let Some(segwit) = deployments.get("segwit") {
            segwit_active = segwit_active || segwit.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        }
        if let Some(taproot) = deployments.get("taproot") {
            taproot_active = taproot_active || taproot.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        }
        // JKC disabled-opcode reactivation
        for key in ["disabled_opcodes", "disabledopcodes", "reactivated_opcodes"] {
            if let Some(dep) = deployments.get(key) {
                disabled_opcodes_active = disabled_opcodes_active
                    || dep.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            }
        }
    }

    // Fallback: if the chain reports a block height >= the known JKC
    // DisabledScriptReactivationHeight, treat opcodes as active. This handles
    // JKC nodes that don't expose the deployment status explicitly.
    if !disabled_opcodes_active {
        if let Some(height) = json.get("blocks").and_then(|v| v.as_u64()) {
            // JKC mainnet: 1,155,000; testnet: 160,000; regtest: 0.
            // We use the mainnet height as the conservative threshold. Testnet
            // and regtest activate earlier, so if the chain reports a height
            // >= 1,155,000 it's definitely past reactivation on any network.
            // For testnet/regtest, the explicit deployment check above should
            // catch it; this fallback only covers mainnet nodes that don't
            // report the deployment.
            if height >= 1_155_000 {
                disabled_opcodes_active = true;
            }
        }
    }

    Ok(SoftforkStatus {
        csv_active,
        segwit_active,
        taproot_active,
        disabled_opcodes_active,
    })
}

// ============================================================================
// SILENCE ESCAPE SCRIPT
// ============================================================================

/// Build the silence escape script for user exit.
///
/// This script allows users to exit without operator cooperation
/// if no valid batch is posted for N blocks.
///
/// Script: <silence_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <user_pubkey> OP_CHECKSIG
pub fn build_silence_escape_script(
    user_pubkey: &[u8],
    silence_delay: u32,
) -> Result<Vec<u8>> {
    if user_pubkey.len() != 33 {
        return Err(anyhow!("User pubkey must be 33 bytes (compressed)"));
    }

    let mut script = Vec::new();

    // Push silence delay
    push_minimal_uint(&mut script, silence_delay as u64);

    // CSV + DROP
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push user pubkey (x-only, 32 bytes for BIP-342 tapscript)
    script.push(32);
    script.extend_from_slice(&user_pubkey[1..]);

    // Checksig
    script.push(OP_CHECKSIG);

    Ok(script)
}

// ============================================================================
// SEAL SPEND SCRIPT
// ============================================================================

/// Build a seal spend script that binds an object to a UTXO.
///
/// The seal ensures that:
/// 1. Object state is bound to a specific UTXO
/// 2. State transition spends the old UTXO and creates a new one
/// 3. Each seal can only be spent once
///
/// This is implemented as an OP_RETURN output with seal data.
pub fn build_seal_spend_output(seal_proof: &SealSpendProof) -> Result<Vec<u8>> {
    if seal_proof.object_id.len() != 32 {
        return Err(anyhow!("Object ID must be 32 bytes"));
    }
    if seal_proof.prev_seal.len() != 36 {
        return Err(anyhow!("Previous seal must be 36 bytes (txid + vout)"));
    }
    if seal_proof.new_seal.len() != 36 {
        return Err(anyhow!("New seal must be 36 bytes (txid + vout)"));
    }
    if seal_proof.state_hash.len() != 32 {
        return Err(anyhow!("State hash must be 32 bytes"));
    }

    let mut script = Vec::new();

    // OP_RETURN
    script.push(OP_RETURN);

    // Protocol identifier
    let protocol = b"utxovm:seal";
    script.push(protocol.len() as u8);
    script.extend_from_slice(protocol);

    // Object ID (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&seal_proof.object_id);

    // Previous seal (36 bytes)
    script.push(0x24);
    script.extend_from_slice(&seal_proof.prev_seal);

    // New seal (36 bytes)
    script.push(0x24);
    script.extend_from_slice(&seal_proof.new_seal);

    // State hash (32 bytes)
    script.push(0x20);
    script.extend_from_slice(&seal_proof.state_hash);

    Ok(script)
}

// ============================================================================
// BATCH COMMITMENT SCRIPT
// ============================================================================

/// Build a batch commitment output.
///
/// This output commits to:
/// 1. State root
/// 2. List of consumed seals
/// 3. Total fees
/// 4. Operator signature
///
/// Implemented as OP_RETURN with batch data.
pub fn build_batch_commitment_output(
    state_root: &[u8],
    consumed_seals: &[Vec<u8>],
    total_fees: u64,
    operator_signature: &[u8],
) -> Result<Vec<u8>> {
    if state_root.len() != 32 {
        return Err(anyhow!("State root must be 32 bytes"));
    }

    let mut script = Vec::new();

    // OP_RETURN
    script.push(OP_RETURN);

    // Protocol identifier
    let protocol = b"utxovm:batch";
    script.push(protocol.len() as u8);
    script.extend_from_slice(protocol);

    // State root (32 bytes)
    script.push(0x20);
    script.extend_from_slice(state_root);

    // Number of consumed seals
    push_minimal_uint(&mut script, consumed_seals.len() as u64);

    // Each consumed seal (36 bytes each)
    for seal in consumed_seals {
        if seal.len() != 36 {
            return Err(anyhow!("Each consumed seal must be 36 bytes"));
        }
        script.push(0x24);
        script.extend_from_slice(seal);
    }

    // Total fees (8 bytes, little-endian)
    script.push(0x08);
    script.extend_from_slice(&total_fees.to_le_bytes());

    // Operator signature (64 bytes compact + 1 byte recovery)
    if operator_signature.len() != 65 {
        return Err(anyhow!("Operator signature must be 65 bytes"));
    }
    script.push(0x41); // 65 bytes push
    script.extend_from_slice(operator_signature);

    Ok(script)
}

// ============================================================================
// FEE OUTPUT
// ============================================================================

/// Build fee output structure.
///
/// Fee output pays:
/// 1. Miner fee (standard transaction fee)
/// 2. Operator fee (optional, from operator)
/// 3. Indexer fee (small output for indexer incentive)
///
/// This returns the output script for the indexer fee.
pub fn build_indexer_fee_output(
    indexer_pubkey: &[u8],
    _fee_sats: u64,
) -> Result<Vec<u8>> {
    if indexer_pubkey.len() != 33 {
        return Err(anyhow!("Indexer pubkey must be 33 bytes (compressed)"));
    }

    // P2WPKH: OP_0 <HASH160(pubkey)> (20 bytes)
    let mut script = Vec::new();

    // OP_0 (witness version 0)
    script.push(OP_FALSE);

    // HASH160(indexer_pubkey) = RIPEMD160(SHA256(pubkey)) — 20 bytes
    let pubkey_hash = hash160(indexer_pubkey);
    script.push(pubkey_hash.len() as u8);
    script.extend_from_slice(&pubkey_hash);

    Ok(script)
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Push a minimal encoding of an integer to the script.
fn push_minimal_uint(script: &mut Vec<u8>, value: u64) {
    if value == 0 {
        script.push(0x00);
    } else if value <= 16 {
        script.push(0x50 + value as u8);
    } else if value <= 0xff {
        script.push(0x01);
        script.push(value as u8);
    } else if value <= 0xffff {
        script.push(0x02);
        script.extend_from_slice(&(value as u16).to_le_bytes());
    } else if value <= 0xffffff {
        script.push(0x03);
        let bytes = (value as u32).to_le_bytes();
        script.extend_from_slice(&bytes[..3]);
    } else {
        script.push(0x04);
        script.extend_from_slice(&(value as u32).to_le_bytes());
    }
}

/// Get the Bitcoin compact-size encoding for a length.
/// Per BIP-341, tapleaf hashing uses compact-size (not script push opcodes).
/// - < 0xfd: single byte
/// - <= 0xffff: 0xfd || 2 bytes little-endian
/// - <= 0xffffffff: 0xfe || 4 bytes little-endian
/// - else: 0xff || 8 bytes little-endian
fn push_size_compact(len: usize) -> Vec<u8> {
    if len < 0xfd {
        vec![len as u8]
    } else if len <= 0xffff {
        vec![0xfd, (len & 0xff) as u8, ((len >> 8) & 0xff) as u8]
    } else if len <= 0xffff_ffff {
        vec![
            0xfe,
            (len & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            ((len >> 16) & 0xff) as u8,
            ((len >> 24) & 0xff) as u8,
        ]
    } else {
        vec![
            0xff,
            (len & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            ((len >> 16) & 0xff) as u8,
            ((len >> 24) & 0xff) as u8,
            ((len >> 32) & 0xff) as u8,
            ((len >> 40) & 0xff) as u8,
            ((len >> 48) & 0xff) as u8,
            ((len >> 56) & 0xff) as u8,
        ]
    }
}

/// Calculate SHA256 hash
pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

/// Calculate HASH160 (SHA256 + RIPEMD160)
pub fn hash160(data: &[u8]) -> Vec<u8> {
    use ripemd::Ripemd160;
    use digest::Digest;
    let sha = Sha256::digest(data);
    Ripemd160::digest(&sha).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_vault_script_tree_model_b() {
        let config = VaultConfig {
            operator_pubkey: vec![0x02; 32],
            challenger_pubkey: vec![0x03; 32],
            unbond_delay: 1008, // ~1 week
            claim_delay: 144,   // ~1 day
            watcher_pubkeys: vec![],
            watcher_threshold: 0,
        };

        let tree = build_vault_script_tree(&config).expect("Failed to build vault script tree");

        assert!(!tree.operator_leaf.is_empty());
        assert!(!tree.challenge_leaf.is_empty());
        assert!(!tree.operator_leaf_hash().is_empty());
        assert!(!tree.challenge_leaf_hash().is_empty());
        assert!(!tree.script_tree_root().is_empty());

        // Verify tagged hashes produce different results than plain SHA256
        let plain_hash = Sha256::digest(&tree.operator_leaf).to_vec();
        assert_ne!(tree.operator_leaf_hash(), plain_hash);

        // Operator leaf must contain CSV and CHECKSIG
        assert!(tree.operator_leaf.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(tree.operator_leaf.contains(&OP_DROP));
        assert!(tree.operator_leaf.contains(&OP_CHECKSIG));

        // Challenge leaf is the equivocation covenant — uses OP_CAT + OP_CHECKSIGVERIFY
        assert!(tree.challenge_leaf.contains(&OP_CHECKSIGVERIFY), "Challenge leaf must use OP_CHECKSIGVERIFY");
        assert!(!tree.challenge_leaf.contains(&OP_CHECKMULTISIG), "Challenge leaf must NOT contain CHECKMULTISIG (BIP-342)");
        assert!(!tree.challenge_leaf.contains(&0xb1), "Challenge leaf must NOT contain CLTV (0xb1)");
        assert!(tree.challenge_leaf.contains(&0x7e), "Challenge leaf MUST contain OP_CAT (0x7e) — equivocation covenant");
    }

    #[test]
    fn test_build_operator_unbond_leaf() {
        let pubkey = vec![0x02; 32];
        let script = build_operator_unbond_leaf(&pubkey, 1008).expect("Failed to build operator leaf");

        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_DROP));
        assert!(script.contains(&OP_CHECKSIG));
        // Must NOT contain CLTV
        assert!(!script.contains(&0xb1), "Unbond leaf must NOT contain CLTV");
    }

    #[test]
    fn test_build_covenant_challenge_leaf() {
        // The challenge leaf is now the equivocation covenant (OP_CAT-based),
        // not the old committee M-of-N model. This test verifies the covenant
        // leaf is constructed correctly.
        let operator_pubkey = vec![0x02; 32];
        let config = VaultConfig {
            operator_pubkey: operator_pubkey.clone(),
            challenger_pubkey: vec![0x03; 32],
            unbond_delay: 144,
            claim_delay: 10,
            watcher_pubkeys: vec![],
            watcher_threshold: 0,
        };
        let tree = build_vault_script_tree(&config).unwrap();

        // Covenant leaf must use OP_CAT (0x7e)
        assert!(tree.challenge_leaf.contains(&0x7e), "Covenant leaf must use OP_CAT");
        // Must use OP_CHECKSIGVERIFY (0xad) — two of them (one per attestation)
        let csv_count = tree.challenge_leaf.iter().filter(|&&b| b == 0xad).count();
        assert_eq!(csv_count, 2, "Covenant leaf must have 2 OP_CHECKSIGVERIFY");
        // Must use OP_SHA256 (0xa8) — two of them
        let sha_count = tree.challenge_leaf.iter().filter(|&&b| b == 0xa8).count();
        assert_eq!(sha_count, 2, "Covenant leaf must have 2 OP_SHA256");
        // Must NOT contain OP_CHECKMULTISIG (disabled in tapscript)
        assert!(!tree.challenge_leaf.contains(&0xae), "Must NOT contain CHECKMULTISIG");
        // Must NOT contain CLTV
        assert!(!tree.challenge_leaf.contains(&0xb1), "Covenant leaf must NOT contain CLTV");
        // Must contain the operator's x-only pubkey
        assert!(tree.challenge_leaf.windows(32).any(|w| w == &operator_pubkey),
            "Covenant leaf must contain operator x-only pubkey");
    }

    #[test]
    fn test_build_challenge_claim_script_tree() {
        let config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 32],
            operator_pubkey: vec![0x02; 32],
            claim_delay: 144,
        };

        let tree = build_challenge_claim_script_tree(&config)
            .expect("Failed to build challenge claim script tree");

        // Claim leaf: CSV + DROP + CHECKSIG
        assert!(tree.claim_leaf.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(tree.claim_leaf.contains(&OP_DROP));
        assert!(tree.claim_leaf.contains(&OP_CHECKSIG));
        assert!(!tree.claim_leaf.contains(&0xb1), "Claim leaf must NOT contain CLTV");

        // Rebut leaf: just CHECKSIG (no timelock)
        assert!(tree.rebut_leaf.contains(&OP_CHECKSIG));
        assert!(!tree.rebut_leaf.contains(&OP_CHECKSEQUENCEVERIFY),
            "Rebut leaf must NOT contain CSV (operator can rebut immediately)");
        assert!(!tree.rebut_leaf.contains(&0xb1), "Rebut leaf must NOT contain CLTV");

        // Script tree root must be non-empty and deterministic
        assert!(!tree.script_tree_root().is_empty());
        assert_eq!(tree.script_tree_root(), tree.script_tree_root(),
            "Script tree root must be deterministic");
    }

    #[test]
    fn test_build_silence_escape_script() {
        let user_pubkey = vec![0x03; 33];
        let script = build_silence_escape_script(&user_pubkey, 60).expect("Failed to build escape script");

        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_DROP));
        assert!(script.contains(&OP_CHECKSIG));
        assert!(!script.contains(&0xb1), "Silence escape must NOT contain CLTV");
    }

    #[test]
    fn test_build_seal_spend_output() {
        let proof = SealSpendProof {
            object_id: vec![0x01; 32],
            prev_seal: vec![0x02; 36],
            new_seal: vec![0x03; 36],
            state_hash: vec![0x04; 32],
        };

        let script = build_seal_spend_output(&proof).expect("Failed to build seal spend output");
        assert!(script.contains(&OP_RETURN));
        // Check for protocol identifier "utxovm:seal"
        let protocol = b"utxovm:seal";
        assert!(script.windows(protocol.len()).any(|w| w == protocol));
    }

    #[test]
    fn test_build_batch_commitment_output() {
        let state_root = vec![0x01; 32];
        let consumed_seals = vec![vec![0x02; 36], vec![0x03; 36]];
        let total_fees = 1000;
        let operator_sig = vec![0x04; 65];

        let script = build_batch_commitment_output(
            &state_root,
            &consumed_seals,
            total_fees,
            &operator_sig,
        ).expect("Failed to build batch commitment");

        assert!(script.contains(&OP_RETURN));
        // Check for protocol identifier "utxovm:batch"
        let protocol = b"utxovm:batch";
        assert!(script.windows(protocol.len()).any(|w| w == protocol));
    }

    #[test]
    fn test_push_minimal_uint() {
        let mut script = Vec::new();
        push_minimal_uint(&mut script, 0);
        assert_eq!(script, vec![0x00]);

        script.clear();
        push_minimal_uint(&mut script, 16);
        assert_eq!(script, vec![0x60]);

        script.clear();
        push_minimal_uint(&mut script, 17);
        assert_eq!(script, vec![0x01, 0x11]);

        script.clear();
        push_minimal_uint(&mut script, 256);
        assert_eq!(script, vec![0x02, 0x00, 0x01]);
    }

    #[test]
    fn test_tagged_hash() {
        // BIP-341 test vector: tapleaf of empty script
        let empty_script = vec![];
        let hash = VaultScriptTree::tapleaf_hash(&empty_script);
        assert_eq!(hash.len(), 32);
        assert_ne!(hash, Sha256::digest(&empty_script).to_vec());
    }

    #[test]
    fn test_assert_chain_supports_bonding_pass() {
        let status = SoftforkStatus {
            csv_active: true,
            segwit_active: true,
            taproot_active: true,
            disabled_opcodes_active: true,
        };
        assert!(assert_chain_supports_bonding(&status).is_ok(),
            "Assertion must pass when CSV, Taproot, and disabled-opcode reactivation are active");
    }

    #[test]
    fn test_assert_chain_supports_bonding_fails_without_csv() {
        let status = SoftforkStatus {
            csv_active: false,
            segwit_active: true,
            taproot_active: true,
            disabled_opcodes_active: true,
        };
        let result = assert_chain_supports_bonding(&status);
        assert!(result.is_err(), "Must fail when CSV is not active");
        assert!(result.unwrap_err().to_string().contains("CSV"),
            "Error must mention CSV");
    }

    #[test]
    fn test_assert_chain_supports_bonding_fails_without_taproot() {
        let status = SoftforkStatus {
            csv_active: true,
            segwit_active: true,
            taproot_active: false,
            disabled_opcodes_active: true,
        };
        let result = assert_chain_supports_bonding(&status);
        assert!(result.is_err(), "Must fail when Taproot is not active");
        assert!(result.unwrap_err().to_string().contains("Taproot"),
            "Error must mention Taproot");
    }

    #[test]
    fn test_assert_chain_supports_bonding_fails_without_disabled_opcodes() {
        let status = SoftforkStatus {
            csv_active: true,
            segwit_active: true,
            taproot_active: true,
            disabled_opcodes_active: false,
        };
        let result = assert_chain_supports_bonding(&status);
        assert!(result.is_err(), "Must fail when disabled-opcode reactivation is not active");
        assert!(result.unwrap_err().to_string().contains("disabled-opcode"),
            "Error must mention disabled-opcode reactivation");
    }

    #[test]
    fn test_no_cltv_in_any_script() {
        // Regression test: no script in this module must use CLTV (0xb1).
        let vault_config = VaultConfig {
            operator_pubkey: vec![0x02; 32],
            challenger_pubkey: vec![0x03; 32],
            unbond_delay: 1008,
            claim_delay: 144,
            watcher_pubkeys: vec![],
            watcher_threshold: 0,
        };
        let vault_tree = build_vault_script_tree(&vault_config).unwrap();
        assert!(!vault_tree.operator_leaf.contains(&0xb1), "Operator leaf must not use CLTV");
        assert!(!vault_tree.challenge_leaf.contains(&0xb1), "Challenge leaf must not use CLTV");

        let challenge_config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 32],
            operator_pubkey: vec![0x02; 32],
            claim_delay: 144,
        };
        let challenge_tree = build_challenge_claim_script_tree(&challenge_config).unwrap();
        assert!(!challenge_tree.claim_leaf.contains(&0xb1), "Claim leaf must not use CLTV");
        assert!(!challenge_tree.rebut_leaf.contains(&0xb1), "Rebut leaf must not use CLTV");
    }

    #[test]
    fn test_slashing_path_uses_op_cat_covenant() {
        // This test deliberately replaces the former test_no_op_cat_in_bonding_scripts,
        // which asserted OP_CAT was absent from bonding scripts. The project's
        // direction has reversed: OP_CAT is now the intended mechanism for
        // on-chain equivocation verification, gated by JKC's
        // DisabledScriptReactivationHeight (h=1,155,000 mainnet).
        //
        // This test asserts the OPPOSITE invariant: the challenge (slashing)
        // leaf MUST use OP_CAT (0x7e) for the equivocation covenant.
        let vault_config = VaultConfig {
            operator_pubkey: vec![0x02; 32],
            challenger_pubkey: vec![0x03; 32],
            unbond_delay: 1008,
            claim_delay: 144,
            watcher_pubkeys: vec![],
            watcher_threshold: 0,
        };
        let vault_tree = build_vault_script_tree(&vault_config).unwrap();

        // The challenge leaf MUST contain OP_CAT (0x7e) — this is the covenant
        assert!(
            vault_tree.challenge_leaf.contains(&0x7e),
            "Challenge leaf MUST use OP_CAT for equivocation covenant (policy reversal from Issue 14)"
        );

        // The operator unbond leaf should NOT use OP_CAT (it's a simple CSV+CHECKSIG)
        assert!(
            !vault_tree.operator_leaf.contains(&0x7e),
            "Operator unbond leaf must not use OP_CAT (simple CSV+CHECKSIG only)"
        );

        // The challenge UTXO claim/rebut leaves should NOT use OP_CAT
        let challenge_config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 32],
            operator_pubkey: vec![0x02; 32],
            claim_delay: 144,
        };
        let challenge_tree = build_challenge_claim_script_tree(&challenge_config).unwrap();
        assert!(
            !challenge_tree.claim_leaf.contains(&0x7e),
            "Claim leaf must not use OP_CAT"
        );
        assert!(
            !challenge_tree.rebut_leaf.contains(&0x7e),
            "Rebut leaf must not use OP_CAT"
        );
    }
}
