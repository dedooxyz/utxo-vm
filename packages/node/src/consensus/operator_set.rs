//! Operator Management Module
//!
//! This module handles:
//! - Operator registration (lock JKC into vault)
//! - Operator set management (committee, threshold)
//! - Operator metrics (uptime, last root, missed batches)
//! - Bond management (floor vs TVL)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Operator status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperatorStatus {
    /// Operator is active and posting batches
    Active,
    /// Operator is unbonding (waiting for unbond delay)
    Unbonding,
    /// Operator has been slashed
    Slashed,
    /// Operator has exited
    Exited,
}

/// Operator record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operator {
    /// Operator's public key (compressed, 33 bytes)
    pub pubkey: String,
    /// Operator's address (P2PKH or P2TR)
    pub address: String,
    /// Bond amount (satoshis)
    pub bond_amount: u64,
    /// Bond UTXO (txid:vout)
    pub bond_utxo: Option<String>,
    /// Status
    pub status: OperatorStatus,
    /// Registration block height
    pub registered_at: u64,
    /// Block height when unbond was initiated (None if not unbonding)
    pub unbond_started_at: Option<u64>,
    /// Last batch posted block height
    pub last_batch_block: Option<u64>,
    /// Last batch state root
    pub last_batch_root: Option<String>,
    /// Total batches posted
    pub total_batches: u64,
    /// Missed batches (when operator was expected to post but didn't)
    pub missed_batches: u64,
    /// Uptime percentage (last 1000 blocks)
    pub uptime_pct: f64,
}

/// Operator set configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSetConfig {
    /// Minimum number of operators
    pub min_operators: u32,
    /// Maximum number of operators
    pub max_operators: u32,
    /// Honest threshold (t in n-of-t)
    pub honest_threshold: u32,
    /// Minimum bond amount (satoshis)
    pub min_bond: u64,
    /// Unbond delay (blocks)
    pub unbond_delay: u32,
    /// Challenge window (blocks)
    pub challenge_window: u32,
    /// Batch interval (blocks) - how often operator should post
    pub batch_interval: u32,
}

impl Default for OperatorSetConfig {
    fn default() -> Self {
        Self {
            min_operators: 3,
            max_operators: 10,
            honest_threshold: 2,
            min_bond: 100_000_000, // 1 JKC
            unbond_delay: 60,
            challenge_window: 10,
            batch_interval: 5,
        }
    }
}

/// Operator set state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSet {
    /// Configuration
    pub config: OperatorSetConfig,
    /// Active operators
    pub operators: Vec<Operator>,
    /// Total bond locked (satoshis)
    pub total_bond: u64,
    /// Total TVL tracked (satoshis)
    pub total_tvl: u64,
}

impl OperatorSet {
    /// Create a new operator set
    pub fn new(config: OperatorSetConfig) -> Self {
        Self {
            config,
            operators: Vec::new(),
            total_bond: 0,
            total_tvl: 0,
        }
    }

    /// Register a new operator
    pub fn register_operator(
        &mut self,
        pubkey: &str,
        address: &str,
        bond_amount: u64,
        bond_utxo: &str,
        block_height: u64,
    ) -> Result<()> {
        // Validate bond amount
        if bond_amount < self.config.min_bond {
            return Err(anyhow!(
                "Bond amount {} is below minimum {}",
                bond_amount,
                self.config.min_bond
            ));
        }

        // Check if operator already exists
        if self.operators.iter().any(|op| op.pubkey == pubkey) {
            return Err(anyhow!("Operator already registered"));
        }

        // Check if we've reached maximum operators
        if self.operators.len() >= self.config.max_operators as usize {
            return Err(anyhow!("Maximum operators reached"));
        }

        // Create operator record
        let operator = Operator {
            pubkey: pubkey.to_string(),
            address: address.to_string(),
            bond_amount,
            bond_utxo: Some(bond_utxo.to_string()),
            status: OperatorStatus::Active,
            registered_at: block_height,
            unbond_started_at: None,
            last_batch_block: None,
            last_batch_root: None,
            total_batches: 0,
            missed_batches: 0,
            uptime_pct: 100.0,
        };

        self.operators.push(operator);
        self.total_bond += bond_amount;

        Ok(())
    }

    /// Unbond an operator
    pub fn unbond_operator(&mut self, pubkey: &str, block_height: u64) -> Result<()> {
        let operator = self.operators
            .iter_mut()
            .find(|op| op.pubkey == pubkey)
            .ok_or_else(|| anyhow!("Operator not found"))?;

        if operator.status != OperatorStatus::Active {
            return Err(anyhow!("Operator is not active"));
        }

        operator.status = OperatorStatus::Unbonding;
        operator.unbond_started_at = Some(block_height);

        Ok(())
    }

    /// Exit an operator (after unbond delay)
    pub fn exit_operator(&mut self, pubkey: &str, block_height: u64) -> Result<()> {
        let operator = self.operators
            .iter_mut()
            .find(|op| op.pubkey == pubkey)
            .ok_or_else(|| anyhow!("Operator not found"))?;

        if operator.status != OperatorStatus::Unbonding {
            return Err(anyhow!("Operator is not unbonding"));
        }

        let unbond_started_at = operator.unbond_started_at
            .ok_or_else(|| anyhow!("Unbond start time not recorded"))?;

        // Check unbond delay from when unbond was initiated
        if block_height < unbond_started_at + self.config.unbond_delay as u64 {
            return Err(anyhow!(
                "Unbond delay not met. Need {} more blocks",
                unbond_started_at + self.config.unbond_delay as u64 - block_height
            ));
        }

        operator.status = OperatorStatus::Exited;
        self.total_bond -= operator.bond_amount;

        Ok(())
    }

    /// Slash an operator (for fraud)
    pub fn slash_operator(&mut self, pubkey: &str) -> Result<u64> {
        let operator = self.operators
            .iter_mut()
            .find(|op| op.pubkey == pubkey)
            .ok_or_else(|| anyhow!("Operator not found"))?;

        if operator.status != OperatorStatus::Active {
            return Err(anyhow!("Operator is not active"));
        }

        operator.status = OperatorStatus::Slashed;
        let slashed_amount = operator.bond_amount;
        self.total_bond -= slashed_amount;

        Ok(slashed_amount)
    }

    /// Record a batch posted by an operator
    pub fn record_batch(
        &mut self,
        pubkey: &str,
        block_height: u64,
        state_root: &str,
    ) -> Result<()> {
        let operator = self.operators
            .iter_mut()
            .find(|op| op.pubkey == pubkey)
            .ok_or_else(|| anyhow!("Operator not found"))?;

        if operator.status != OperatorStatus::Active {
            return Err(anyhow!("Operator is not active"));
        }

        operator.last_batch_block = Some(block_height);
        operator.last_batch_root = Some(state_root.to_string());
        operator.total_batches += 1;

        Ok(())
    }

    /// Record a missed batch
    pub fn record_missed_batch(&mut self, pubkey: &str) -> Result<()> {
        let operator = self.operators
            .iter_mut()
            .find(|op| op.pubkey == pubkey)
            .ok_or_else(|| anyhow!("Operator not found"))?;

        operator.missed_batches += 1;

        // Update uptime
        let total_expected = operator.total_batches + operator.missed_batches;
        if total_expected > 0 {
            operator.uptime_pct = (operator.total_batches as f64 / total_expected as f64) * 100.0;
        }

        Ok(())
    }

    /// Get active operators
    pub fn active_operators(&self) -> Vec<&Operator> {
        self.operators
            .iter()
            .filter(|op| op.status == OperatorStatus::Active)
            .collect()
    }

    /// Check if we have enough operators for consensus
    pub fn has_quorum(&self) -> bool {
        self.active_operators().len() >= self.config.honest_threshold as usize
    }

    /// Get operator by pubkey
    pub fn get_operator(&self, pubkey: &str) -> Option<&Operator> {
        self.operators.iter().find(|op| op.pubkey == pubkey)
    }

    /// Update TVL
    pub fn update_tvl(&mut self, tvl: u64) {
        self.total_tvl = tvl;
    }

    /// Check if bond is adequate for TVL
    pub fn is_bond_adequate(&self) -> bool {
        if self.total_tvl == 0 {
            return true;
        }
        // Bond should be at least 10x TVL
        self.total_bond >= self.total_tvl * 10
    }

    /// Get metrics summary
    pub fn metrics(&self) -> OperatorSetMetrics {
        let active = self.active_operators();
        let total_batches: u64 = self.operators.iter().map(|op| op.total_batches).sum();
        let total_missed: u64 = self.operators.iter().map(|op| op.missed_batches).sum();
        let avg_uptime = if !self.operators.is_empty() {
            self.operators.iter().map(|op| op.uptime_pct).sum::<f64>() / self.operators.len() as f64
        } else {
            0.0
        };

        OperatorSetMetrics {
            total_operators: self.operators.len() as u32,
            active_operators: active.len() as u32,
            total_bond: self.total_bond,
            total_tvl: self.total_tvl,
            total_batches,
            total_missed,
            average_uptime: avg_uptime,
            has_quorum: self.has_quorum(),
            bond_adequate: self.is_bond_adequate(),
        }
    }
}

/// Operator set metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSetMetrics {
    pub total_operators: u32,
    pub active_operators: u32,
    pub total_bond: u64,
    pub total_tvl: u64,
    pub total_batches: u64,
    pub total_missed: u64,
    pub average_uptime: f64,
    pub has_quorum: bool,
    pub bond_adequate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_registration() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        let result = opset.register_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            200_000_000, // 2 JKC
            "abc123:0",
            100,
        );
        
        assert!(result.is_ok());
        assert_eq!(opset.operators.len(), 1);
        assert_eq!(opset.total_bond, 200_000_000);
    }

    #[test]
    fn test_operator_registration_insufficient_bond() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        let result = opset.register_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            50_000_000, // 0.5 JKC (below minimum)
            "abc123:0",
            100,
        );
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("below minimum"));
    }

    #[test]
    fn test_operator_unbond() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        opset.register_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            200_000_000,
            "abc123:0",
            100,
        ).unwrap();
        
        let result = opset.unbond_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            200,
        );
        
        assert!(result.is_ok());
        assert_eq!(opset.operators[0].status, OperatorStatus::Unbonding);
    }

    #[test]
    fn test_operator_slash() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        opset.register_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            200_000_000,
            "abc123:0",
            100,
        ).unwrap();
        
        let slashed = opset.slash_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        ).unwrap();
        
        assert_eq!(slashed, 200_000_000);
        assert_eq!(opset.operators[0].status, OperatorStatus::Slashed);
        assert_eq!(opset.total_bond, 0);
    }

    #[test]
    fn test_quorum() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        // Register 3 operators
        for i in 0..3 {
            opset.register_operator(
                &format!("02{:064x}", i),
                "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
                200_000_000,
                &format!("abc{}:0", i),
                100,
            ).unwrap();
        }
        
        assert!(opset.has_quorum());
        
        // Slash one operator
        opset.slash_operator("020000000000000000000000000000000000000000000000000000000000000000").unwrap();
        
        // Still have quorum (2 active operators, threshold is 2)
        assert!(opset.has_quorum());
    }

    #[test]
    fn test_metrics() {
        let mut opset = OperatorSet::new(OperatorSetConfig::default());
        
        // Register 2 operators to meet quorum (threshold is 2)
        opset.register_operator(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            200_000_000,
            "abc123:0",
            100,
        ).unwrap();
        
        opset.register_operator(
            "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
            "muZpTpBYhxmRFuCjLc7C6BBDF32C8XVJUi",
            200_000_000,
            "abc456:0",
            100,
        ).unwrap();
        
        opset.record_batch(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            150,
            "abc123",
        ).unwrap();
        
        let metrics = opset.metrics();
        assert_eq!(metrics.total_operators, 2);
        assert_eq!(metrics.active_operators, 2);
        assert_eq!(metrics.total_batches, 1);
        assert!(metrics.has_quorum);
    }
}
