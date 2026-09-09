//! Fee Management Module
//!
//! This module handles:
//! - Fee envelope structure (L1 fee + indexer output)
//! - Fee distribution mechanism (pay operators proportionally)
//! - Fee calculation (based on batch count and challenge status)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Fee envelope structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeEnvelope {
    /// Total fee amount (satoshis)
    pub total_fee: u64,
    /// Indexer fee (satoshis) - fixed portion for indexer
    pub indexer_fee: u64,
    /// Operator pool (satoshis) - variable portion for operators
    pub operator_pool: u64,
    /// Batch count at time of fee calculation
    pub batch_count: u64,
    /// Block height when fee was collected
    pub block_height: u64,
    /// Transaction ID where fee was collected
    pub txid: String,
}

/// Operator fee share
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorFeeShare {
    /// Operator's public key
    pub pubkey: String,
    /// Number of batches posted by this operator
    pub batches_posted: u64,
    /// Number of batches challenged (rejected)
    pub batches_challenged: u64,
    /// Fee share (satoshis)
    pub fee_share: u64,
    /// Payment address
    pub address: String,
}

/// Fee distribution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeDistribution {
    /// Fee envelopes collected
    pub envelopes: Vec<FeeEnvelope>,
    /// Operator fee shares
    pub operator_shares: Vec<OperatorFeeShare>,
    /// Total fees collected (satoshis)
    pub total_fees_collected: u64,
    /// Total fees distributed (satoshis)
    pub total_fees_distributed: u64,
    /// Pending fees (satoshis)
    pub pending_fees: u64,
}

impl FeeDistribution {
    /// Create a new fee distribution
    pub fn new() -> Self {
        Self {
            envelopes: Vec::new(),
            operator_shares: Vec::new(),
            total_fees_collected: 0,
            total_fees_distributed: 0,
            pending_fees: 0,
        }
    }

    /// Create a fee envelope
    pub fn create_fee_envelope(
        &mut self,
        total_fee: u64,
        batch_count: u64,
        block_height: u64,
        txid: &str,
    ) -> Result<FeeEnvelope> {
        // Indexer fee is 10% of total fee (minimum 1000 satoshis)
        let indexer_fee = std::cmp::max(total_fee / 10, 1000);
        let operator_pool = total_fee - indexer_fee;

        let envelope = FeeEnvelope {
            total_fee,
            indexer_fee,
            operator_pool,
            batch_count,
            block_height,
            txid: txid.to_string(),
        };

        self.envelopes.push(envelope.clone());
        self.total_fees_collected += total_fee;
        self.pending_fees += operator_pool;

        Ok(envelope)
    }

    /// Calculate operator fee shares
    pub fn calculate_operator_shares(
        &mut self,
        operators: Vec<OperatorFeeShare>,
    ) -> Result<Vec<OperatorFeeShare>> {
        if operators.is_empty() {
            return Err(anyhow!("No operators to distribute fees to"));
        }

        let total_effective_batches: u64 = operators.iter()
            .map(|op| op.batches_posted.saturating_sub(op.batches_challenged))
            .sum();
        if total_effective_batches == 0 {
            return Err(anyhow!("No effective batches posted"));
        }

        let mut shares = Vec::new();
        let mut distributed = 0u64;

        for operator in &operators {
            let effective_batches = operator.batches_posted.saturating_sub(operator.batches_challenged);
            let share = (self.pending_fees * effective_batches) / total_effective_batches;

            let fee_share = OperatorFeeShare {
                pubkey: operator.pubkey.clone(),
                batches_posted: operator.batches_posted,
                batches_challenged: operator.batches_challenged,
                fee_share: share,
                address: operator.address.clone(),
            };

            shares.push(fee_share);
            distributed += share;
        }

        if distributed < self.pending_fees && !shares.is_empty() {
            shares[0].fee_share += self.pending_fees - distributed;
        }

        self.operator_shares = shares.clone();

        Ok(shares)
    }

    /// Distribute fees to operators (call after calculate_operator_shares)
    pub fn distribute_fees(&mut self) {
        let distributed: u64 = self.operator_shares.iter().map(|s| s.fee_share).sum();
        self.total_fees_distributed += distributed;
        self.pending_fees = self.pending_fees.saturating_sub(distributed);
    }

    /// Mark operator as challenged (reduces their fee share)
    pub fn mark_operator_challenged(&mut self, pubkey: &str) -> Result<()> {
        for share in &mut self.operator_shares {
            if share.pubkey == pubkey {
                share.batches_challenged += 1;
                return Ok(());
            }
        }
        Err(anyhow!("Operator not found in shares"))
    }

    /// Get pending fees for distribution
    pub fn pending_fees(&self) -> u64 {
        self.pending_fees
    }

    /// Get total fees collected
    pub fn total_fees_collected(&self) -> u64 {
        self.total_fees_collected
    }

    /// Get total fees distributed
    pub fn total_fees_distributed(&self) -> u64 {
        self.total_fees_distributed
    }

    /// Get fee summary
    pub fn summary(&self) -> FeeSummary {
        FeeSummary {
            total_envelopes: self.envelopes.len() as u32,
            total_fees_collected: self.total_fees_collected,
            total_fees_distributed: self.total_fees_distributed,
            pending_fees: self.pending_fees,
            total_operators: self.operator_shares.len() as u32,
        }
    }
}

/// Fee summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeSummary {
    pub total_envelopes: u32,
    pub total_fees_collected: u64,
    pub total_fees_distributed: u64,
    pub pending_fees: u64,
    pub total_operators: u32,
}

/// Fee calculation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeParams {
    /// Base fee per transaction (satoshis)
    pub base_fee: u64,
    /// Fee per byte of data (satoshis)
    pub fee_per_byte: u64,
    /// Indexer fee percentage (1-100)
    pub indexer_fee_pct: u8,
    /// Minimum indexer fee (satoshis)
    pub min_indexer_fee: u64,
    /// Minimum operator fee per batch (satoshis)
    pub min_operator_fee: u64,
}

impl Default for FeeParams {
    fn default() -> Self {
        Self {
            base_fee: 1000,
            fee_per_byte: 1,
            indexer_fee_pct: 10,
            min_indexer_fee: 1000,
            min_operator_fee: 100,
        }
    }
}

/// Calculate total fee for a transaction
pub fn calculate_total_fee(
    params: &FeeParams,
    data_size: u32,
    batch_count: u64,
) -> u64 {
    let data_fee = params.base_fee + (data_size as u64 * params.fee_per_byte);
    let batch_fee = batch_count * params.min_operator_fee;
    data_fee + batch_fee
}

/// Calculate indexer fee from total
pub fn calculate_indexer_fee(params: &FeeParams, total_fee: u64) -> u64 {
    let indexer_fee = (total_fee * params.indexer_fee_pct as u64) / 100;
    std::cmp::max(indexer_fee, params.min_indexer_fee)
}

/// Calculate operator pool from total
pub fn calculate_operator_pool(params: &FeeParams, total_fee: u64) -> u64 {
    let indexer_fee = calculate_indexer_fee(params, total_fee);
    total_fee - indexer_fee
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fee_envelope_creation() {
        let mut dist = FeeDistribution::new();
        
        let envelope = dist.create_fee_envelope(
            10000,
            10,
            12345,
            "abc123",
        ).unwrap();
        
        assert_eq!(envelope.total_fee, 10000);
        assert_eq!(envelope.indexer_fee, 1000); // 10% of 10000
        assert_eq!(envelope.operator_pool, 9000);
        assert_eq!(dist.total_fees_collected, 10000);
        assert_eq!(dist.pending_fees, 9000);
    }

    #[test]
    fn test_operator_shares_calculation() {
        let mut dist = FeeDistribution::new();
        
        // Create fee envelope
        dist.create_fee_envelope(10000, 10, 12345, "abc123").unwrap();
        
        // Create operator shares
        let operators = vec![
            OperatorFeeShare {
                pubkey: "op1".to_string(),
                batches_posted: 5,
                batches_challenged: 0,
                fee_share: 0,
                address: "addr1".to_string(),
            },
            OperatorFeeShare {
                pubkey: "op2".to_string(),
                batches_posted: 5,
                batches_challenged: 0,
                fee_share: 0,
                address: "addr2".to_string(),
            },
        ];
        
        let shares = dist.calculate_operator_shares(operators).unwrap();
        
        assert_eq!(shares.len(), 2);
        // Each operator posted 5 batches, total 10, pending fees 9000
        // Each should get 4500
        assert_eq!(shares[0].fee_share, 4500);
        assert_eq!(shares[1].fee_share, 4500);
    }

    #[test]
    fn test_operator_challenged() {
        let mut dist = FeeDistribution::new();
        
        dist.create_fee_envelope(10000, 10, 12345, "abc123").unwrap();
        
        // Calculate shares with op1 having 1 challenged batch
        let operators = vec![
            OperatorFeeShare {
                pubkey: "op1".to_string(),
                batches_posted: 5,
                batches_challenged: 1,
                fee_share: 0,
                address: "addr1".to_string(),
            },
            OperatorFeeShare {
                pubkey: "op2".to_string(),
                batches_posted: 5,
                batches_challenged: 0,
                fee_share: 0,
                address: "addr2".to_string(),
            },
        ];
        
        let shares = dist.calculate_operator_shares(operators).unwrap();
        
        // op1: effective_batches = 5 - 1 = 4
        // op2: effective_batches = 5 - 0 = 5
        // total effective = 9
        // op1 share: (9000 * 4) / 9 = 4000
        // op2 share: (9000 * 5) / 9 = 5000
        assert_eq!(shares[0].fee_share, 4000);
        assert_eq!(shares[1].fee_share, 5000);
    }

    #[test]
    fn test_summary() {
        let mut dist = FeeDistribution::new();
        
        dist.create_fee_envelope(10000, 10, 12345, "abc123").unwrap();
        dist.create_fee_envelope(5000, 5, 12346, "def456").unwrap();
        
        let summary = dist.summary();
        
        assert_eq!(summary.total_envelopes, 2);
        assert_eq!(summary.total_fees_collected, 15000);
        // Pending fees: 9000 + 4000 = 13000
        // (5000/10 = 500, but minimum indexer fee is 1000, so operator_pool = 4000)
        assert_eq!(summary.pending_fees, 13000);
    }

    #[test]
    fn test_fee_calculation() {
        let params = FeeParams::default();
        
        let total_fee = calculate_total_fee(&params, 100, 5);
        // data_fee = 1000 + (100 * 1) = 1100
        // batch_fee = 5 * 100 = 500
        // total = 1600
        assert_eq!(total_fee, 1600);
    }

    #[test]
    fn test_indexer_fee() {
        let params = FeeParams::default();
        
        let indexer_fee = calculate_indexer_fee(&params, 10000);
        // 10% of 10000 = 1000
        assert_eq!(indexer_fee, 1000);
        
        // Test minimum indexer fee
        let indexer_fee = calculate_indexer_fee(&params, 5000);
        // 10% of 5000 = 500, but minimum is 1000
        assert_eq!(indexer_fee, 1000);
    }
}
