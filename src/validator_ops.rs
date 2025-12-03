//! Validator operations integration
//!
//! This module integrates validator key management, staking, and reward distribution
//! with proper penalty application for validators with >10% downtime.

use crate::{
    FuelFeeCollector, RewardDistributor, StakingManager, ValidatorInfo, ValidatorReward,
    ValidatorSet,
};
use silver_core::{Error, Result, ValidatorID, ValidatorMetadata};
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
                stake_amount,
                crate::MIN_STAKE_AMOUNT
            )));
        }

        // Add to validator set
        self.validator_set.add_validator(metadata)?;

        // Record stake deposit
        self.staking_manager
            .deposit_stake(validator_id.clone(), stake_amount, deposit_tx)?;

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
        self.validator_set
            .record_participation(validator_id, participated);
    }

    /// Collect transaction fee
    pub fn collect_transaction_fee(
        &mut self,
        tx_digest: [u8; 64],
        fuel_consumed: u64,
        fuel_price: u64,
    ) {
        self.fee_collector
            .collect_fee(tx_digest, fuel_consumed, fuel_price);
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

            validator_data.insert(validator.id(), (stake, participation_rate));
        }

        // Calculate and distribute rewards
        let rewards = self
            .reward_distributor
            .distribute_cycle_rewards(&self.fee_collector, &validator_data);

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
