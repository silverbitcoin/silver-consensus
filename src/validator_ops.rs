//! Validator operations integration
//!
//! This module integrates validator key management, staking, and reward distribution
//! with proper penalty application for validators with >10% downtime.

use crate::{
    ValidatorSet, ValidatorInfo, StakingManager, RewardDistributor,
    FuelFeeCollector, ValidatorReward,
};
use silver_core::{ValidatorID, ValidatorMetadata, Result, Error};
use std::collections::HashMap;
use tracing::{info, warn};

/// Downtime threshold for penalties (10%)
pub const DOWNTIME_THRESHOLD: f64 = 0.10;

/// Validator operations coordinator
///
/// Coordinates all validator-related operations including:
/// - Validator registration with stake deposits
/// - Participation tracking
/// - Reward distribution with downtime penalties
pub struct ValidatorOperations {
    /// Validator set
    validator_set: ValidatorSet,
    
    /// Staking manager
    staking_manager: StakingManager,
    
    /// Reward distributor
    reward_distributor: RewardDistributor,
    
    /// Fee collector
    fee_collector: FuelFeeCollector,
}

impl ValidatorOperations {
    /// Create new validator operations coordinator
    pub fn new() -> Self {
        // Set minimum participation to 90% (10% downtime threshold)
        let reward_distributor = RewardDistributor::new(0.9, 1.0);
        
        Self {
            validator_set: ValidatorSet::new(),
            staking_manager: StakingManager::new(),
            reward_distributor,
            fee_collector: FuelFeeCollector::new(),
        }
    }

    /// Register a new validator with stake deposit
    pub fn register_validator(
        &mut self,
        metadata: ValidatorMetadata,
        deposit_tx: Vec<u8>,
    ) -> Result<()> {
        let validator_id = metadata.id();
        let stake_amount = metadata.stake_amount;

        // Validate minimum stake
        if stake_amount < crate::MIN_STAKE_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Validator stake {} is below minimum {}",
                stake_amount, crate::MIN_STAKE_AMOUNT
            )));
        }

        // Add to validator set
        self.validator_set.add_validator(metadata)?;

        // Record stake deposit
        self.staking_manager.deposit_stake(
            validator_id.clone(),
            stake_amount,
            deposit_tx,
        )?;

        info!(
            "Registered validator {} with {} SBTC stake",
            validator_id, stake_amount
        );

        Ok(())
    }

    /// Record validator participation in a snapshot
    pub fn record_snapshot_participation(
        &mut self,
        validator_id: &ValidatorID,
        participated: bool,
    ) {
        self.validator_set.record_participation(validator_id, participated);
    }

    /// Collect transaction fee
    pub fn collect_transaction_fee(
        &mut self,
        tx_digest: [u8; 64],
        fuel_consumed: u64,
        fuel_price: u64,
    ) {
        self.fee_collector.collect_fee(tx_digest, fuel_consumed, fuel_price);
    }

    /// End cycle and distribute rewards
    ///
    /// This applies the following logic:
    /// 1. Calculate base rewards proportional to stake
    /// 2. Apply 100% penalty for validators with >10% downtime
    /// 3. Distribute rewards to validators
    /// 4. Process unbonding requests
    pub fn end_cycle(&mut self) -> HashMap<ValidatorID, ValidatorReward> {
        info!("Ending cycle and distributing rewards");

        // Get all active validators
        let validators = self.validator_set.get_active_validators();

        // Build validator data for reward calculation
        let mut validator_data = HashMap::new();
        for validator in &validators {
            let stake = self.staking_manager.get_active_stake(&validator.id());
            let participation_rate = validator.participation_rate();
            
            validator_data.insert(
                validator.id(),
                (stake, participation_rate),
            );
        }

        // Calculate and distribute rewards
        let rewards = self.reward_distributor.distribute_cycle_rewards(
            &self.fee_collector,
            &validator_data,
        );

        // Log penalties for validators with high downtime
        for (validator_id, reward) in &rewards {
            let downtime = 1.0 - reward.participation_rate;
            
            if downtime > DOWNTIME_THRESHOLD {
                warn!(
                    "Validator {} penalized for {:.1}% downtime (participation: {:.1}%)",
                    validator_id,
                    downtime * 100.0,
                    reward.participation_rate * 100.0
                );
            }
        }

        // Reset fee collector for next cycle
        self.fee_collector.reset();

        // Advance validator set cycle
        self.validator_set.advance_cycle();

        // Process unbonding requests
        let completed_unbonding = self.staking_manager.process_unbonding();
        if !completed_unbonding.is_empty() {
            info!(
                "Processed unbonding for {} validators",
                completed_unbonding.len()
            );
        }

        rewards
    }

    /// Request validator unstaking
    pub fn request_unstake(
        &mut self,
        validator_id: &ValidatorID,
        amount: u64,
    ) -> Result<(crate::UnstakingRequest, Option<crate::TierChangeEvent>)> {
        let (request, tier_event) = self.staking_manager.request_unstake(validator_id, amount)?;

        // Check if validator still meets minimum stake
        if !self.staking_manager.meets_minimum_stake(validator_id) {
            warn!(
                "Validator {} no longer meets minimum stake requirement after unstaking",
                validator_id
            );
        }

        if let Some(ref event) = tier_event {
            info!(
                "Validator {} tier changed from {} to {} after unstaking",
                validator_id, event.from_tier, event.to_tier
            );
        }

        Ok((request, tier_event))
    }

    /// Get validator info
    pub fn get_validator(&self, validator_id: &ValidatorID) -> Option<ValidatorInfo> {
        self.validator_set.get_validator(validator_id)
    }

    /// Get validator active stake
    pub fn get_validator_stake(&self, validator_id: &ValidatorID) -> u64 {
        self.staking_manager.get_active_stake(validator_id)
    }

    /// Get all active validators
    pub fn get_active_validators(&self) -> Vec<ValidatorInfo> {
        self.validator_set.get_active_validators()
    }

    /// Get total staked amount
    pub fn total_staked(&self) -> u64 {
        self.staking_manager.total_staked()
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.validator_set.current_cycle()
    }

    /// Get total fees collected this cycle
    pub fn total_fees_this_cycle(&self) -> u64 {
        self.fee_collector.total_fees()
    }

    /// Check if validator set has quorum
    pub fn has_quorum(&self, validator_ids: &[ValidatorID]) -> bool {
        self.validator_set.has_quorum(validator_ids)
    }

    /// Get validators below minimum stake
    pub fn get_validators_below_minimum(&self) -> Vec<ValidatorID> {
        self.staking_manager.get_below_minimum_stake()
    }

    /// Apply participation penalties
    ///
    /// Returns list of validators with participation below threshold
    pub fn get_low_participation_validators(&mut self) -> Vec<ValidatorID> {
        // 90% minimum participation (10% downtime threshold)
        self.validator_set.apply_participation_penalties(0.9)
    }
}

impl Default for ValidatorOperations {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{PublicKey, SignatureScheme, SilverAddress};

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
    fn test_validator_registration() {
        let mut ops = ValidatorOperations::new();
        let metadata = create_test_validator(1, 50_000); // Silver tier
        let validator_id = metadata.id();

        let result = ops.register_validator(metadata, vec![1u8; 64]);
        assert!(result.is_ok());

        assert_eq!(ops.get_validator_stake(&validator_id), 1_000_000);
        assert!(ops.get_validator(&validator_id).is_some());
    }

    #[test]
    fn test_participation_tracking() {
        let mut ops = ValidatorOperations::new();
        let metadata = create_test_validator(1, 1_000_000);
        let validator_id = metadata.id();

        ops.register_validator(metadata, vec![1u8; 64]).unwrap();

        // Record participation
        ops.record_snapshot_participation(&validator_id, true);
        ops.record_snapshot_participation(&validator_id, true);
        ops.record_snapshot_participation(&validator_id, false);

        let validator = ops.get_validator(&validator_id).unwrap();
        assert_eq!(validator.snapshots_participated, 2);
        assert_eq!(validator.total_snapshots, 3);
    }

    #[test]
    fn test_reward_distribution() {
        let mut ops = ValidatorOperations::new();
        
        // Register two validators
        let metadata1 = create_test_validator(1, 1_000_000);
        let metadata2 = create_test_validator(2, 2_000_000);
        let id1 = metadata1.id();
        let id2 = metadata2.id();

        ops.register_validator(metadata1, vec![1u8; 64]).unwrap();
        ops.register_validator(metadata2, vec![2u8; 64]).unwrap();

        // Record participation (validator 1: 100%, validator 2: 80%)
        for _ in 0..10 {
            ops.record_snapshot_participation(&id1, true);
            ops.record_snapshot_participation(&id2, true);
        }
        for _ in 0..2 {
            ops.record_snapshot_participation(&id2, false);
        }

        // Collect fees
        ops.collect_transaction_fee([1; 64], 1000, 1000);
        ops.collect_transaction_fee([2; 64], 2000, 1000);

        // End cycle and distribute rewards
        let rewards = ops.end_cycle();

        assert_eq!(rewards.len(), 2);
        
        // Validator 1 should get full reward (100% participation)
        let reward1 = rewards.get(&id1).unwrap();
        assert_eq!(reward1.penalty, 0);

        // Validator 2 should get penalty (80% participation < 90% threshold)
        let reward2 = rewards.get(&id2).unwrap();
        assert!(reward2.penalty > 0);
    }

    #[test]
    fn test_unstaking() {
        let mut ops = ValidatorOperations::new();
        let metadata = create_test_validator(1, 500_000); // Platinum tier
        let validator_id = metadata.id();

        ops.register_validator(metadata, vec![1u8; 64]).unwrap();

        // Request unstaking (should downgrade to Gold)
        let (request, tier_event) = ops.request_unstake(&validator_id, 400_000).unwrap();
        assert!(tier_event.is_some()); // Tier changed
        assert!(!request.completed);
        assert_eq!(ops.get_validator_stake(&validator_id), 100_000);
    }

    #[test]
    fn test_downtime_threshold() {
        assert_eq!(DOWNTIME_THRESHOLD, 0.10);
    }
}
