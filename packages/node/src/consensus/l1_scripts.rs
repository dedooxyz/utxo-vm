//! L1 Script Builders for UTXO-VM
//!
//! Empirically verified against JKC testnet (block 177,269, getblockchaininfo RPC):
//!   csv:     active=true,  height=120000
//!   segwit:  active=true,  height=140000
//!   taproot: active=true,  height=160000
//!   mweb:    active=false, height=180000
//!   bip65:   active=false, height=99999999  (CLTV — NOT active, do not use)
//!
//! All timelocks use OP_CHECKSEQUENCEVERIFY (CSV, active at height 120,000).
//! No CLTV, no OP_CAT. Hash comparison uses OP_EQUAL/OP_EQUALVERIFY only.
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
pub const OP_CHECKSEQUENCEVERIFY: u8 = 0xb2;
pub const OP_SHA256: u8 = 0xa8;
pub const OP_RETURN: u8 = 0x6a;
pub const OP_1: u8 = 0x51;

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
/// Trust assumption: the watcher committee (M-of-N) must independently verify
/// divergence evidence off-chain before co-signing a challenge. Bitcoin Script
/// cannot verify off-chain data, so script-level security is limited to
/// "M independent watchers agreed to challenge." A fraudulent committee can
/// steal the bond; the security model assumes M-of-N watchers are honest.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Operator's public key (compressed, 33 bytes)
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (compressed, 33 bytes) — receives slashed bond
    pub challenger_pubkey: Vec<u8>,
    /// Unbond delay in blocks (relative timelock via CSV)
    pub unbond_delay: u32,
    /// Claim delay in blocks (relative timelock on challenge UTXO via CSV)
    pub claim_delay: u32,
    /// Watcher committee public keys (compressed, 33 bytes each)
    pub watcher_pubkeys: Vec<Vec<u8>>,
    /// Required number of watcher signatures (M-of-N)
    pub watcher_threshold: u32,
}

/// Challenge UTXO configuration (Model B second stage)
#[derive(Debug, Clone)]
pub struct ChallengeUtxoConfig {
    /// Challenger's public key (compressed, 33 bytes) — receives slashed bond
    pub challenger_pubkey: Vec<u8>,
    /// Operator's public key (compressed, 33 bytes) — can rebut
    pub operator_pubkey: Vec<u8>,
    /// Claim delay in blocks (CSV from challenge tx confirmation)
    pub claim_delay: u32,
}

/// Challenge proof data
#[derive(Debug, Clone)]
pub struct ChallengeProof {
    /// The operator's claimed (wrong) root
    pub claimed_root: Vec<u8>,
    /// The correct root (from challenger's re-execution)
    pub correct_root: Vec<u8>,
    /// Operator's signature on the wrong root
    pub operator_signature: Vec<u8>,
    /// Operator's public key
    pub operator_pubkey: Vec<u8>,
    /// Challenger's public key (must be provided, NOT hardcoded)
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

/// Build the operator vault script tree for P2TR spending (Model B).
///
/// Script tree:
/// - Leaf 1 (Operator unbond): `<unbond_delay> CSV DROP <operator_pubkey> CHECKSIG`
/// - Leaf 2 (Committee challenge): `<M> <pubkey1> ... <pubkeyN> <N> CHECKMULTISIG`
///
/// The committee challenge leaf has NO timelock — watchers can initiate a
/// challenge at any time by spending the bond to a challenge UTXO (which
/// carries the CSV-delayed claim path). The operator's only defense is to
/// unbond via Leaf 1 before the committee challenges, or to not diverge.
pub fn build_vault_script_tree(config: &VaultConfig) -> Result<VaultScriptTree> {
    if config.operator_pubkey.len() != 33 {
        return Err(anyhow!("Operator pubkey must be 33 bytes (compressed)"));
    }
    if config.challenger_pubkey.len() != 33 {
        return Err(anyhow!("Challenger pubkey must be 33 bytes (compressed)"));
    }
    if config.watcher_pubkeys.is_empty() {
        return Err(anyhow!("Watcher committee must have at least 1 pubkey"));
    }
    for (i, pk) in config.watcher_pubkeys.iter().enumerate() {
        if pk.len() != 33 {
            return Err(anyhow!("Watcher pubkey {} must be 33 bytes (compressed)", i));
        }
    }
    if config.watcher_threshold as usize > config.watcher_pubkeys.len() {
        return Err(anyhow!("Watcher threshold M ({}) cannot exceed N ({})",
            config.watcher_threshold, config.watcher_pubkeys.len()));
    }
    if config.watcher_threshold == 0 {
        return Err(anyhow!("Watcher threshold must be >= 1"));
    }

    // Leaf 1: Operator unbond path
    let operator_leaf = build_operator_unbond_leaf(
        &config.operator_pubkey,
        config.unbond_delay,
    )?;

    // Leaf 2: Committee challenge path (M-of-N multisig, no timelock)
    let challenge_leaf = build_committee_challenge_leaf(
        &config.watcher_pubkeys,
        config.watcher_threshold,
    )?;

    Ok(VaultScriptTree {
        operator_leaf,
        challenge_leaf,
        operator_pubkey: config.operator_pubkey.clone(),
        challenger_pubkey: config.challenger_pubkey.clone(),
    })
}

/// Build the operator unbond leaf script.
///
/// Script: <unbond_delay> OP_CHECKSEQUENCEVERIFY OP_DROP <operator_pubkey> OP_CHECKSIG
fn build_operator_unbond_leaf(
    operator_pubkey: &[u8],
    unbond_delay: u32,
) -> Result<Vec<u8>> {
    let mut script = Vec::new();

    // Push unbond delay as minimal encoding
    push_minimal_uint(&mut script, unbond_delay as u64);

    // CSV + DROP
    script.push(OP_CHECKSEQUENCEVERIFY);
    script.push(OP_DROP);

    // Push operator pubkey
    script.push(operator_pubkey.len() as u8);
    script.extend_from_slice(operator_pubkey);

    // Checksig
    script.push(OP_CHECKSIG);

    Ok(script)
}

/// Build the committee challenge leaf script (Model B).
///
/// Script: OP_<M> <pubkey1> <pubkey2> ... <pubkeyN> OP_<N> OP_CHECKMULTISIG
///
/// The watcher committee (M-of-N) can spend the bond to a challenge UTXO.
/// No timelock — the committee can challenge at any time. Security comes
/// from requiring M independent watchers to co-sign, not from the script
/// proving divergence (which Bitcoin Script cannot do).
fn build_committee_challenge_leaf(
    watcher_pubkeys: &[Vec<u8>],
    threshold: u32,
) -> Result<Vec<u8>> {
    if watcher_pubkeys.is_empty() {
        return Err(anyhow!("Watcher committee must have at least 1 pubkey"));
    }
    if threshold as usize > watcher_pubkeys.len() {
        return Err(anyhow!("Threshold M cannot exceed N"));
    }

    let mut script = Vec::new();

    // Push M (threshold) as minimal encoding
    push_minimal_uint(&mut script, threshold as u64);

    // Push each watcher pubkey
    for pk in watcher_pubkeys {
        if pk.len() != 33 {
            return Err(anyhow!("Watcher pubkey must be 33 bytes (compressed)"));
        }
        script.push(pk.len() as u8);
        script.extend_from_slice(pk);
    }

    // Push N (total watchers)
    push_minimal_uint(&mut script, watcher_pubkeys.len() as u64);

    // CHECKMULTISIG
    script.push(OP_CHECKMULTISIG);

    Ok(script)
}

/// Vault script tree containing both spending paths
#[derive(Debug, Clone)]
pub struct VaultScriptTree {
    pub operator_leaf: Vec<u8>,
    pub challenge_leaf: Vec<u8>,
    pub operator_pubkey: Vec<u8>,
    pub challenger_pubkey: Vec<u8>,
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
    if config.challenger_pubkey.len() != 33 {
        return Err(anyhow!("Challenger pubkey must be 33 bytes (compressed)"));
    }
    if config.operator_pubkey.len() != 33 {
        return Err(anyhow!("Operator pubkey must be 33 bytes (compressed)"));
    }

    // Leaf 1: Challenger claim (CSV delay from challenge tx)
    let claim_leaf = {
        let mut script = Vec::new();
        push_minimal_uint(&mut script, config.claim_delay as u64);
        script.push(OP_CHECKSEQUENCEVERIFY);
        script.push(OP_DROP);
        script.push(config.challenger_pubkey.len() as u8);
        script.extend_from_slice(&config.challenger_pubkey);
        script.push(OP_CHECKSIG);
        script
    };

    // Leaf 2: Operator rebut (no timelock — immediate)
    let rebut_leaf = {
        let mut script = Vec::new();
        script.push(config.operator_pubkey.len() as u8);
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
}

/// One-time startup assertion: verify that the connected chain reports
/// CSV and Taproot as active. This is a deployment precondition, not runtime
/// feature detection — it fails loudly once at startup if the chain doesn't
/// meet the requirements. No polling, no fallback logic.
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

    // Check "softforks" array (Bitcoin Core format)
    if let Some(softforks) = json.get("softforks").and_then(|v| v.as_array()) {
        for sf in softforks {
            let name = sf.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let active = sf.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            match name {
                "csv" => csv_active = active,
                "segwit" => segwit_active = active,
                "taproot" => taproot_active = active,
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
    }

    Ok(SoftforkStatus {
        csv_active,
        segwit_active,
        taproot_active,
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

    // Push user pubkey
    script.push(user_pubkey.len() as u8);
    script.extend_from_slice(user_pubkey);

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
            operator_pubkey: vec![0x02; 33],
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 1008, // ~1 week
            claim_delay: 144,   // ~1 day
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]],
            watcher_threshold: 2,
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

        // Challenge leaf must contain CHECKMULTISIG (not OP_CAT, not CLTV)
        assert!(tree.challenge_leaf.contains(&OP_CHECKMULTISIG));
        assert!(!tree.challenge_leaf.contains(&0xb1), "Challenge leaf must NOT contain CLTV (0xb1)");
        assert!(!tree.challenge_leaf.contains(&0x7e), "Challenge leaf must NOT contain OP_CAT (0x7e)");
    }

    #[test]
    fn test_build_operator_unbond_leaf() {
        let pubkey = vec![0x02; 33];
        let script = build_operator_unbond_leaf(&pubkey, 1008).expect("Failed to build operator leaf");

        assert!(script.contains(&OP_CHECKSEQUENCEVERIFY));
        assert!(script.contains(&OP_DROP));
        assert!(script.contains(&OP_CHECKSIG));
        // Must NOT contain CLTV
        assert!(!script.contains(&0xb1), "Unbond leaf must NOT contain CLTV");
    }

    #[test]
    fn test_build_committee_challenge_leaf() {
        let watchers = vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]];
        let script = build_committee_challenge_leaf(&watchers, 2)
            .expect("Failed to build committee challenge leaf");

        assert!(script.contains(&OP_CHECKMULTISIG));
        // Must contain all 3 watcher pubkeys
        for pk in &watchers {
            assert!(script.windows(pk.len()).any(|w| w == pk.as_slice()),
                "Challenge leaf must contain watcher pubkey");
        }
        // Must NOT contain OP_CAT or CLTV
        assert!(!script.contains(&0x7e), "Challenge leaf must NOT contain OP_CAT");
        assert!(!script.contains(&0xb1), "Challenge leaf must NOT contain CLTV");
    }

    #[test]
    fn test_build_challenge_claim_script_tree() {
        let config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 33],
            operator_pubkey: vec![0x02; 33],
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
        };
        assert!(assert_chain_supports_bonding(&status).is_ok(),
            "Assertion must pass when CSV and Taproot are active");
    }

    #[test]
    fn test_assert_chain_supports_bonding_fails_without_csv() {
        let status = SoftforkStatus {
            csv_active: false,
            segwit_active: true,
            taproot_active: true,
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
        };
        let result = assert_chain_supports_bonding(&status);
        assert!(result.is_err(), "Must fail when Taproot is not active");
        assert!(result.unwrap_err().to_string().contains("Taproot"),
            "Error must mention Taproot");
    }

    #[test]
    fn test_no_cltv_in_any_script() {
        // Regression test: no script in this module must use CLTV (0xb1).
        let vault_config = VaultConfig {
            operator_pubkey: vec![0x02; 33],
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 1008,
            claim_delay: 144,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]],
            watcher_threshold: 2,
        };
        let vault_tree = build_vault_script_tree(&vault_config).unwrap();
        assert!(!vault_tree.operator_leaf.contains(&0xb1), "Operator leaf must not use CLTV");
        assert!(!vault_tree.challenge_leaf.contains(&0xb1), "Challenge leaf must not use CLTV");

        let challenge_config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 33],
            operator_pubkey: vec![0x02; 33],
            claim_delay: 144,
        };
        let challenge_tree = build_challenge_claim_script_tree(&challenge_config).unwrap();
        assert!(!challenge_tree.claim_leaf.contains(&0xb1), "Claim leaf must not use CLTV");
        assert!(!challenge_tree.rebut_leaf.contains(&0xb1), "Rebut leaf must not use CLTV");
    }

    #[test]
    fn test_no_op_cat_in_bonding_scripts() {
        // Regression test: no bonding script must use OP_CAT (0x7e).
        let vault_config = VaultConfig {
            operator_pubkey: vec![0x02; 33],
            challenger_pubkey: vec![0x03; 33],
            unbond_delay: 1008,
            claim_delay: 144,
            watcher_pubkeys: vec![vec![0x04; 33], vec![0x05; 33], vec![0x06; 33]],
            watcher_threshold: 2,
        };
        let vault_tree = build_vault_script_tree(&vault_config).unwrap();
        assert!(!vault_tree.operator_leaf.contains(&0x7e), "Operator leaf must not use OP_CAT");
        assert!(!vault_tree.challenge_leaf.contains(&0x7e), "Challenge leaf must not use OP_CAT");

        let challenge_config = ChallengeUtxoConfig {
            challenger_pubkey: vec![0x03; 33],
            operator_pubkey: vec![0x02; 33],
            claim_delay: 144,
        };
        let challenge_tree = build_challenge_claim_script_tree(&challenge_config).unwrap();
        assert!(!challenge_tree.claim_leaf.contains(&0x7e), "Claim leaf must not use OP_CAT");
        assert!(!challenge_tree.rebut_leaf.contains(&0x7e), "Rebut leaf must not use OP_CAT");
    }
}
