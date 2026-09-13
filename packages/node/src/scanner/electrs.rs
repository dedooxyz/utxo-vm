use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Maximum response body size accepted from electrs (64 MB).
///
/// Electrs is a trusted, operator-configured backend, so this is not an
/// attacker-facing vulnerability today. But a misbehaving, buggy, or
/// compromised electrs instance (or a misconfigured proxy) could return an
/// unbounded response and exhaust this node's memory. 64 MB is generous
/// (a full block's worth of transactions is well under this) but bounded.
const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Validate that `s` is a well-formed hex string of exactly `expected_len` bytes
/// (i.e. `expected_len * 2` hex chars). Returns `Err` if the value contains
/// non-hex characters, is the wrong length, or could alter the URL path (e.g.
/// contains `/`, `..`, or other path-altering characters — which hex never does,
/// so the all-hex check is sufficient).
///
/// This guards every method that interpolates a hash-shaped parameter into a
/// URL path, preventing path-traversal against the trusted electrs backend.
fn validate_hex_hash(s: &str, expected_len: usize) -> Result<()> {
    let expected_hex_len = expected_len * 2;
    if s.len() != expected_hex_len {
        return Err(anyhow::anyhow!(
            "invalid hash: expected {} hex chars ({} bytes), got {} chars",
            expected_hex_len,
            expected_len,
            s.len()
        ));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!(
            "invalid hash: contains non-hex characters (path traversal guard)"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectrsTxInput {
    pub txid: String,
    pub vout: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectrsTxOutput {
    pub value: u64,
    pub scriptpubkey: String,
    pub scriptpubkey_asm: Option<String>,
    pub scriptpubkey_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectrsTx {
    pub txid: String,
    pub vin: Vec<ElectrsTxInput>,
    pub vout: Vec<ElectrsTxOutput>,
}

#[derive(Clone)]
pub struct ElectrsClient {
    base_url: String,
    client: Client,
}

impl ElectrsClient {
    /// Read the response body into a `Vec<u8>` with a hard cap of
    /// `MAX_RESPONSE_BODY_BYTES`. Returns `Err` if the body exceeds the cap
    /// or if `Content-Length` advertises more than the cap. This prevents a
    /// misbehaving electrs/proxy from exhausting this node's memory with an
    /// unbounded response.
    async fn read_capped_body(&self, resp: reqwest::Response) -> Result<Vec<u8>> {
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_RESPONSE_BODY_BYTES {
                return Err(anyhow::anyhow!(
                    "electrs response too large: Content-Length={} bytes (cap={})",
                    len,
                    MAX_RESPONSE_BODY_BYTES
                ));
            }
        }
        let body = resp
            .bytes()
            .await
            .context("reading electrs response body")?;
        if body.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(anyhow::anyhow!(
                "electrs response too large: {} bytes (cap={})",
                body.len(),
                MAX_RESPONSE_BODY_BYTES
            ));
        }
        Ok(body.to_vec())
    }

    /// Read + cap the body, then deserialize as JSON.
    async fn read_capped_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T> {
        let body = self.read_capped_body(resp).await?;
        let val = serde_json::from_slice(&body)
            .context("parsing electrs JSON response")?;
        Ok(val)
    }

    pub fn new(base_url: String) -> Self {
        Self::with_timeout(base_url, 15)
    }

    pub fn with_timeout(base_url: String, timeout_secs: u64) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!("[Electrs] Failed to build HTTP client with timeout: {} — using default", e);
                Client::new()
            });
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    pub async fn get_tip_height(&self) -> Result<u64> {
        let url = format!("{}/blocks/tip/height", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        let height: u64 = text.trim().parse().context("Failed to parse tip height")?;
        Ok(height)
    }

    pub async fn get_block_hash(&self, height: u64) -> Result<String> {
        let url = format!("{}/block-height/{}", self.base_url, height);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let hash = resp.text().await?.trim().to_string();
        Ok(hash)
    }

    pub async fn get_block_txs(&self, block_hash: &str) -> Result<Vec<ElectrsTx>> {
        validate_hex_hash(block_hash, 32).context("block_hash")?;
        let url = format!("{}/block/{}/txs", self.base_url, block_hash);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let txs = self.read_capped_json::<Vec<ElectrsTx>>(resp).await?;
        Ok(txs)
    }

    pub async fn get_mempool_tx_ids(&self) -> Result<Vec<String>> {
        let url = format!("{}/mempool/txids", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let txids = self.read_capped_json::<Vec<String>>(resp).await?;
        Ok(txids)
    }

    pub async fn get_tx(&self, txid: &str) -> Result<ElectrsTx> {
        validate_hex_hash(txid, 32).context("txid")?;
        let url = format!("{}/tx/{}", self.base_url, txid);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let tx = self.read_capped_json::<ElectrsTx>(resp).await?;
        Ok(tx)
    }

    /// Broadcast a raw transaction to the network.
    ///
    /// Submits the raw tx hex to the electrs `/tx` POST endpoint.
    /// Returns the broadcast txid on success.
    pub async fn broadcast_tx(&self, raw_hex: &str) -> Result<String> {
        let url = format!("{}/tx", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(raw_hex.to_string())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Broadcast failed: HTTP {} — {}",
                status,
                body
            ));
        }

        let txid = resp.text().await?.trim().to_string();
        if txid.is_empty() {
            return Err(anyhow::anyhow!("Broadcast returned empty txid"));
        }
        Ok(txid)
    }
}
