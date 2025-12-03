//! Validator rewards and fuel distribution
//!
//! This module handles:
//! - Collection of fuel fees from transactions
//! - Distribution of fees to validators at cycle end
//! - Reward calculation based on stake weight
//! - Penalty application for low participation

use silver_core::ValidatorID;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Fuel fee collector
///
/// Collects fuel fees from executed transactions during a cycle.
#[derive(Debug, Clone)]
pub struct FuelFeeCollector {
    /// Total fuel fees collected this cycle (in MIST)
    total_fees: u64,

    /// Fees collected per transaction
    transaction_fees: Vec<TransactionFee>,
}

/// Fee from a single transaction
#[derive(Debug, Clone)]
pub struct TransactionFee {
    /// Transaction digest
    pub digest: [u8; 64],

    /// Fuel consumed
    pub fuel_consumed: u64,

    /// Fuel price (MIST per fuel unit)
    pub fuel_price: u64,

    /// Total fee (consumed * price)
    pub total_fee: u64,
}

impl FuelFeeCollector {
    /// Create a new fuel fee collector
    pub fn new() -> Self {
        Self {
            total_fees: 0,
            transaction_fees: Vec::new(),
        }
    }

    /// Collect fee from a transaction
    ///
    /// # Arguments
    /// * `digest` - Transaction digest
    /// * `fuel_consumed` - Fuel consumed by transaction
    /// * `fuel_price` - Fuel price in MIST per fuel unit
    pub fn collect_fee(&mut self, digest: [u8; 64], fuel_consumed: u64, fuel_price: u64) {
        let total_fee = fuel_consumed.saturating_mul(fuel_price);

        self.transaction_fees.push(TransactionFee {
            digest,
            fuel_consumed,
            fuel_price,
            total_fee,
        });

        self.total_fees = self.total_fees.saturating_add(total_fee);

        debug!(
            "Collected fee: {} MIST (fuel: {}, price: {})",
            total_fee, fuel_consumed, fuel_price
        );
    }

    /// Get total fees collected
    pub fn total_fees(&self) -> u64 {
        self.total_fees
    }

    /// Get number of transactions
    pub fn transaction_count(&self) -> usize {
        self.transaction_fees.len()
    }

    /// Get average fee per transaction
    pub fn average_fee(&self) -> u64 {
        if self.transaction_fees.is_empty() {
            return 0;
        }
        self.total_fees / self.transaction_fees.len() as u64
    }

    /// Reset for new cycle
    pub fn reset(&mut self) {
        self.total_fees = 0;
        self.transaction_fees.clear();
    }
}

impl Default for FuelFeeCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Validator reward information
#[derive(Debug, Clone)]
pub struct ValidatorReward {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Stake amount
    pub stake: u64,

    /// Stake weight (proportion of total stake)
    pub stake_weight: f64,

    /// Participation rate (0.0 to 1.0)
    pub participation_rate: f64,

    /// Base reward (proportional to stake)
    pub base_reward: u64,

    /// Penalty amount (for low participation)
    pub penalty: u64,

    /// Final reward (base - penalty)
    pub final_reward: u64,
}

/// Reward distributor
///
/// Distributes fuel fees to validators at cycle end based on:
/// - Stake weight (proportional distribution)
/// - Participation rate (penalties for low participation)
pub struct RewardDistributor {
    /// Minimum participation rate to avoid penalty (default 0.9 = 90%)
    min_participation_rate: f64,

    /// Penalty rate for low participation (default 0.5 = 50% reduction)
    penalty_rate: f64,
}

impl RewardDistributor {
    /// Create a new reward distributor
    ///
    /// # Arguments
    /// * `min_participation_rate` - Minimum participation to avoid penalty (0.0 to 1.0)
    /// * `penalty_rate` - Penalty multiplier for low participation (0.0 to 1.0)
    pub fn new(min_participation_rate: f64, penalty_rate: f64) -> Self {
        Self {
            min_participation_rate,
            penalty_rate,
        }
    }

    /// Create with default parameters
    ///
    /// - Minimum participation: 90%
    /// - Penalty rate: 50% reduction
    pub fn default() -> Self {
        Self::new(0.9, 0.5)
    }

    /// Calculate rewards for validators
    ///
    /// # Arguments
    /// * `total_fees` - Total fuel fees collected this cycle
    /// * `validators` - Map of validator ID to (stake, participation_rate)
    ///
    /// # Returns
    /// Map of validator ID to reward amount
    pub fn calculate_rewards(
        &self,
        total_fees: u64,
        validators: &HashMap<ValidatorID, (u64, f64)>,
    ) -> HashMap<ValidatorID, ValidatorReward> {
        if validators.is_empty() || total_fees == 0 {
            return HashMap::new();
        }

        // Calculate total stake
        let total_stake: u64 = validators.values().map(|(stake, _)| stake).sum();

        if total_stake == 0 {
            warn!("Total stake is zero, cannot distribute rewards");
            return HashMap::new();
        }

        let mut rewards = HashMap::new();

        for (validator_id, (stake, participation_rate)) in validators {
            // Calculate stake weight
            let stake_weight = *stake as f64 / total_stake as f64;

            // Calculate base reward proportional to stake
            let base_reward = (total_fees as f64 * stake_weight) as u64;

            // Calculate penalty for low participation
            let penalty = if *participation_rate < self.min_participation_rate {
                (base_reward as f64 * self.penalty_rate) as u64
            } else {
                0
            };

            // Final reward after penalty
            let final_reward = base_reward.saturating_sub(penalty);

            let reward = ValidatorReward {
                validator_id: validator_id.clone(),
                stake: *stake,
                stake_weight,
                participation_rate: *participation_rate,
                base_reward,
                penalty,
                final_reward,
            };

            debug!(
                "Validator {} reward: {} MIST (base: {}, penalty: {}, participation: {:.2}%)",
                validator_id,
                final_reward,
                base_reward,
                penalty,
                participation_rate * 100.0
            );

            rewards.insert(validator_id.clone(), reward);
        }

        info!(
            "Distributed {} MIST in rewards to {} validators",
            total_fees,
            validators.len()
        );

        rewards
    }

    /// Distribute rewards at cycle end
    ///
    /// This is the main entry point for reward distribution.
    ///
    /// # Arguments
    /// * `collector` - Fuel fee collector with accumulated fees
    /// * `validators` - Map of validator ID to (stake, participation_rate)
    ///
    /// # Returns
    /// Map of validator ID to reward amount
    pub fn distribute_cycle_rewards(
        &self,
        collector: &FuelFeeCollector,
        validators: &HashMap<ValidatorID, (u64, f64)>,
    ) -> HashMap<ValidatorID, ValidatorReward> {
        let total_fees = collector.total_fees();

        info!(
            "Distributing {} MIST from {} transactions to {} validators",
            total_fees,
            collector.transaction_count(),
            validators.len()
        );

        self.calculate_rewards(total_fees, validators)
    }

    /// Get minimum participation rate
    pub fn min_participation_rate(&self) -> f64 {
        self.min_participation_rate
    }

    /// Get penalty rate
    pub fn penalty_rate(&self) -> f64 {
        self.penalty_rate
    }
}

impl Default for RewardDistributor {
    fn default() -> Self {
        Self::new(0.9, 0.5)
    }
}

/// Cycle rewards manager
///
/// Manages the complete reward cycle:
/// 1. Collect fees during cycle
/// 2. Calculate rewards at cycle end
/// 3. Distribute to validators
pub struct CycleRewardsManager {
    /// Fee collector
    collector: FuelFeeCollector,

    /// Reward distributor
    distributor: RewardDistributor,

    /// Current cycle ID
    current_cycle: u64,
}

impl CycleRewardsManager {
    /// Create a new cycle rewards manager
    pub fn new(distributor: RewardDistributor) -> Self {
        Self {
            collector: FuelFeeCollector::new(),
            distributor,
            current_cycle: 0,
        }
    }

    /// Create with default distributor
    pub fn default() -> Self {
        Self::new(RewardDistributor::default())
    }

    /// Collect fee from a transaction
    pub fn collect_transaction_fee(
        &mut self,
        digest: [u8; 64],
        fuel_consumed: u64,
        fuel_price: u64,
    ) {
        self.collector
            .collect_fee(digest, fuel_consumed, fuel_price);
    }

    /// End current cycle and distribute rewards
    ///
    /// # Arguments
    /// * `validators` - Map of validator ID to (stake, participation_rate)
    ///
    /// # Returns
    /// Map of validator ID to reward amount
    pub fn end_cycle(
        &mut self,
        validators: &HashMap<ValidatorID, (u64, f64)>,
    ) -> HashMap<ValidatorID, ValidatorReward> {
        info!(
            "Ending cycle {} with {} MIST in fees",
            self.current_cycle,
            self.collector.total_fees()
        );

        // Distribute rewards
        let rewards = self
            .distributor
            .distribute_cycle_rewards(&self.collector, validators);

        // Reset collector for next cycle
        self.collector.reset();
        self.current_cycle += 1;

        info!("Started cycle {}", self.current_cycle);

        rewards
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get total fees collected this cycle
    pub fn total_fees_this_cycle(&self) -> u64 {
        self.collector.total_fees()
    }

    /// Get transaction count this cycle
    pub fn transaction_count_this_cycle(&self) -> usize {
        self.collector.transaction_count()
    }

    /// Get the fee collector
    pub fn collector(&self) -> &FuelFeeCollector {
        &self.collector
    }

    /// Get the reward distributor
    pub fn distributor(&self) -> &RewardDistributor {
        &self.distributor
    }
}

impl Default for CycleRewardsManager {
    fn default() -> Self {
        Self::new(RewardDistributor::default())
    }
}
