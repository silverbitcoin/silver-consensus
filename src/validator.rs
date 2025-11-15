//! Validator set management
//!
//! This module manages the validator set for consensus, including:
//! - Validator registration and stake tracking
//! - Stake-weighted voting
//! - Validator set reconfiguration at cycle boundaries

use silver_core::{Error, Result, SilverAddress, ValidatorID, ValidatorMetadata};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{info, warn};

/// Validator information
#[derive(Debug, Clone)]
pub struct ValidatorInfo {
    /// Validator metadata
    pub metadata: ValidatorMetadata,

    /// Current stake amount
    pub stake: u64,

    /// Whether validator is active
    pub active: bool,

    /// Number of snapshots participated in this cycle
    pub snapshots_participated: u64,

    /// Total snapshots in this cycle
    pub total_snapshots: u64,
}

impl ValidatorInfo {
    /// Create new validator info
    pub fn new(metadata: ValidatorMetadata) -> Self {
        let stake = metadata.stake_amount;
        Self {
            metadata,
            stake,
            active: true,
            snapshots_participated: 0,
            total_snapshots: 0,
        }
    }

    /// Get validator ID
    pub fn id(&self) -> ValidatorID {
        self.metadata.id()
    }

    /// Get validator address
    pub fn address(&self) -> &SilverAddress {
        &self.metadata.silver_address
    }

    /// Get stake amount
    pub fn stake_amount(&self) -> u64 {
        self.stake
    }

    /// Check if validator is active
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Calculate participation rate
    pub fn participation_rate(&self) -> f64 {
        if self.total_snapshots == 0 {
            return 0.0;
        }
        self.snapshots_participated as f64 / self.total_snapshots as f64
    }

    /// Record snapshot participation
    pub fn record_participation(&mut self, participated: bool) {
        self.total_snapshots += 1;
        if participated {
            self.snapshots_participated += 1;
        }
    }

    /// Reset cycle statistics
    pub fn reset_cycle_stats(&mut self) {
        self.snapshots_participated = 0;
        self.total_snapshots = 0;
    }
}

/// Validator set managing all validators
pub struct ValidatorSet {
    /// Validators indexed by ID
    validators: Arc<DashMap<ValidatorID, ValidatorInfo>>,

    /// Total stake in the network
    total_stake: Arc<RwLock<u64>>,

    /// Current cycle ID
    current_cycle: Arc<RwLock<u64>>,
}

impl ValidatorSet {
    /// Create a new validator set
    pub fn new() -> Self {
        Self {
            validators: Arc::new(DashMap::new()),
            total_stake: Arc::new(RwLock::new(0)),
            current_cycle: Arc::new(RwLock::new(0)),
        }
    }

    /// Add a validator to the set
    pub fn add_validator(&mut self, metadata: ValidatorMetadata) -> Result<()> {
        metadata.validate()?;

        let validator_id = metadata.id();
        let stake = metadata.stake_amount;

        if self.validators.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} already exists",
                validator_id
            )));
        }

        let info = ValidatorInfo::new(metadata);
        self.validators.insert(validator_id.clone(), info);

        // Update total stake
        *self.total_stake.write() += stake;

        info!(
            "Added validator {} with stake {} SBTC",
            validator_id, stake
        );

        Ok(())
    }

    /// Remove a validator from the set
    pub fn remove_validator(&mut self, validator_id: &ValidatorID) -> Result<()> {
        if let Some((_, info)) = self.validators.remove(validator_id) {
            // Update total stake
            *self.total_stake.write() -= info.stake;

            info!("Removed validator {}", validator_id);
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Validator {} not found",
                validator_id
            )))
        }
    }

    /// Get validator info
    pub fn get_validator(&self, validator_id: &ValidatorID) -> Option<ValidatorInfo> {
        self.validators.get(validator_id).map(|v| v.clone())
    }

    /// Check if validator exists
    pub fn contains_validator(&self, validator_id: &ValidatorID) -> bool {
        self.validators.contains_key(validator_id)
    }

    /// Get all validators
    pub fn get_all_validators(&self) -> Vec<ValidatorInfo> {
        self.validators
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get active validators
    pub fn get_active_validators(&self) -> Vec<ValidatorInfo> {
        self.validators
            .iter()
            .filter(|entry| entry.value().is_active())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get total stake
    pub fn total_stake(&self) -> u64 {
        *self.total_stake.read()
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.validators.len()
    }

    /// Get active validator count
    pub fn active_validator_count(&self) -> usize {
        self.validators
            .iter()
            .filter(|entry| entry.value().is_active())
            .count()
    }

    /// Calculate stake weight for a set of validators
    pub fn calculate_stake_weight(&self, validator_ids: &[ValidatorID]) -> u64 {
        validator_ids
            .iter()
            .filter_map(|id| self.validators.get(id).map(|v| v.stake))
            .sum()
    }

    /// Check if a set of validators has quorum (2/3+ stake)
    pub fn has_quorum(&self, validator_ids: &[ValidatorID]) -> bool {
        let stake_weight = self.calculate_stake_weight(validator_ids);
        let total = self.total_stake();
        stake_weight * 3 > total * 2
    }

    /// Record validator participation in a snapshot
    pub fn record_participation(&mut self, validator_id: &ValidatorID, participated: bool) {
        if let Some(mut validator) = self.validators.get_mut(validator_id) {
            validator.record_participation(participated);
        }
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        *self.current_cycle.read()
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) -> u64 {
        let mut cycle = self.current_cycle.write();
        *cycle += 1;

        // Reset cycle statistics for all validators
        for mut validator in self.validators.iter_mut() {
            validator.reset_cycle_stats();
        }

        info!("Advanced to cycle {}", *cycle);
        *cycle
    }

    /// Apply penalties for low participation
    pub fn apply_participation_penalties(&mut self, threshold: f64) -> Vec<ValidatorID> {
        let mut penalized = Vec::new();

        for mut entry in self.validators.iter_mut() {
            let validator = entry.value_mut();
            let rate = validator.participation_rate();

            if rate < threshold {
                warn!(
                    "Validator {} has low participation rate: {:.2}%",
                    validator.id(),
                    rate * 100.0
                );
                penalized.push(validator.id());
            }
        }

        penalized
    }

    /// Clear all validators
    pub fn clear(&mut self) {
        self.validators.clear();
        *self.total_stake.write() = 0;
        info!("Cleared validator set");
    }
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{PublicKey, SignatureScheme};

    fn create_test_validator(id: u8, stake: u64) -> ValidatorMetadata {
        let address = SilverAddress::new([id; 64]);
        let pubkey = PublicKey {
            scheme: SignatureScheme::Dilithium3,
            bytes: vec![0u8; 100],
        };

        ValidatorMetadata::new(
            address,
            pubkey.clone(),
            pubkey.clone(),
            pubkey,
            stake,
            "127.0.0.1:9000".to_string(),
            "127.0.0.1:9001".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn test_validator_set_add() {
        let mut set = ValidatorSet::new();
        let metadata = create_test_validator(1, 1_000_000);
        let id = metadata.id();

        assert!(set.add_validator(metadata).is_ok());
        assert!(set.contains_validator(&id));
        assert_eq!(set.validator_count(), 1);
        assert_eq!(set.total_stake(), 1_000_000);
    }

    #[test]
    fn test_validator_set_quorum() {
        let mut set = ValidatorSet::new();

        // Add 3 validators with equal stake
        for i in 1..=3 {
            let metadata = create_test_validator(i, 1_000_000);
            set.add_validator(metadata).unwrap();
        }

        assert_eq!(set.total_stake(), 3_000_000);

        // 2 validators = 2/3 stake = quorum
        let val1 = create_test_validator(1, 1_000_000).id();
        let val2 = create_test_validator(2, 1_000_000).id();
        assert!(set.has_quorum(&[val1, val2]));

        // 1 validator = 1/3 stake = no quorum
        assert!(!set.has_quorum(&[val1]));
    }

    #[test]
    fn test_validator_participation() {
        let mut info = ValidatorInfo::new(create_test_validator(1, 1_000_000));

        info.record_participation(true);
        info.record_participation(true);
        info.record_participation(false);

        assert_eq!(info.snapshots_participated, 2);
        assert_eq!(info.total_snapshots, 3);
        assert!((info.participation_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_validator_set_cycle() {
        let mut set = ValidatorSet::new();
        assert_eq!(set.current_cycle(), 0);

        let cycle = set.advance_cycle();
        assert_eq!(cycle, 1);
        assert_eq!(set.current_cycle(), 1);
    }
}

