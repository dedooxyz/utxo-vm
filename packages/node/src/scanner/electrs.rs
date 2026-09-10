use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

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
        let url = format!("{}/block/{}/txs", self.base_url, block_hash);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let txs = resp.json::<Vec<ElectrsTx>>().await?;
        Ok(txs)
    }

    pub async fn get_mempool_tx_ids(&self) -> Result<Vec<String>> {
        let url = format!("{}/mempool/txids", self.base_url);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let txids = resp.json::<Vec<String>>().await?;
        Ok(txids)
    }

    pub async fn get_tx(&self, txid: &str) -> Result<ElectrsTx> {
        let url = format!("{}/tx/{}", self.base_url, txid);
        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let tx = resp.json::<ElectrsTx>().await?;
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
