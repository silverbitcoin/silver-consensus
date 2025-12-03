//! Multi-tier validator system
//!
//! This module implements a four-tier validator system with different
//! stake requirements, voting power multipliers, and reward multipliers.
//!
//! Tiers:
//! - Bronze: 10,000 SBTC minimum, 0.5x voting power, 1.0x rewards
//! - Silver: 50,000 SBTC minimum, 1.0x voting power, 1.2x rewards
//! - Gold: 100,000 SBTC minimum, 1.5x voting power, 1.5x rewards
//! - Platinum: 500,000 SBTC minimum, 2.0x voting power, 2.0x rewards

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::fmt;
use tracing::{info, warn};

/// Validator tier levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub enum ValidatorTier {
    /// Bronze tier: 10,000 SBTC minimum
    Bronze,
    /// Silver tier: 50,000 SBTC minimum
    Silver,
    /// Gold tier: 100,000 SBTC minimum
    Gold,
    /// Platinum tier: 500,000 SBTC minimum
    Platinum,
}

impl ValidatorTier {
    /// Get minimum stake requirement for this tier in SBTC
    pub fn min_stake(&self) -> u64 {
        match self {
            ValidatorTier::Bronze => 10_000,
            ValidatorTier::Silver => 50_000,
            ValidatorTier::Gold => 100_000,
            ValidatorTier::Platinum => 500_000,
        }
    }

    /// Get voting power multiplier for this tier
    pub fn voting_power_multiplier(&self) -> f64 {
        match self {
            ValidatorTier::Bronze => 0.5,
            ValidatorTier::Silver => 1.0,
            ValidatorTier::Gold => 1.5,
            ValidatorTier::Platinum => 2.0,
        }
    }

    /// Get reward multiplier for this tier
    pub fn reward_multiplier(&self) -> f64 {
        match self {
            ValidatorTier::Bronze => 1.0,
            ValidatorTier::Silver => 1.2,
            ValidatorTier::Gold => 1.5,
            ValidatorTier::Platinum => 2.0,
        }
    }

    /// Determine tier from stake amount
    pub fn from_stake(stake: u64) -> Self {
        if stake >= ValidatorTier::Platinum.min_stake() {
            ValidatorTier::Platinum
        } else if stake >= ValidatorTier::Gold.min_stake() {
            ValidatorTier::Gold
        } else if stake >= ValidatorTier::Silver.min_stake() {
            ValidatorTier::Silver
        } else {
            ValidatorTier::Bronze
        }
    }

    /// Get all tiers in ascending order
    pub fn all_tiers() -> Vec<ValidatorTier> {
        vec![
            ValidatorTier::Bronze,
            ValidatorTier::Silver,
            ValidatorTier::Gold,
            ValidatorTier::Platinum,
        ]
    }

    /// Get tier name as string
    pub fn name(&self) -> &'static str {
        match self {
            ValidatorTier::Bronze => "Bronze",
            ValidatorTier::Silver => "Silver",
            ValidatorTier::Gold => "Gold",
            ValidatorTier::Platinum => "Platinum",
        }
    }

    /// Check if can upgrade to target tier with given stake
    pub fn can_upgrade_to(&self, target: ValidatorTier, stake: u64) -> bool {
        target > *self && stake >= target.min_stake()
    }

    /// Check if will downgrade to target tier with given stake
    pub fn will_downgrade_to(&self, stake: u64) -> Option<ValidatorTier> {
        let new_tier = ValidatorTier::from_stake(stake);
        if new_tier < *self {
            Some(new_tier)
        } else {
            None
        }
    }
}

impl fmt::Display for ValidatorTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Tier change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierChangeEvent {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Previous tier
    pub from_tier: ValidatorTier,

    /// New tier
    pub to_tier: ValidatorTier,

    /// Stake amount at time of change
    pub stake_amount: u64,

    /// Timestamp of change
    pub timestamp: u64,

    /// Cycle when change occurred
    pub cycle: u64,
}

impl TierChangeEvent {
    /// Create new tier change event
    pub fn new(
        validator_id: ValidatorID,
        from_tier: ValidatorTier,
        to_tier: ValidatorTier,
        stake_amount: u64,
        cycle: u64,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            validator_id,
            from_tier,
            to_tier,
            stake_amount,
            timestamp,
            cycle,
        }
    }

    /// Check if this is an upgrade
    pub fn is_upgrade(&self) -> bool {
        self.to_tier > self.from_tier
    }

    /// Check if this is a downgrade
    pub fn is_downgrade(&self) -> bool {
        self.to_tier < self.from_tier
    }
}

/// Validator tier information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorTierInfo {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Current tier
    pub current_tier: ValidatorTier,

    /// Current stake amount
    pub stake_amount: u64,

    /// Tier history (most recent first)
    pub tier_history: Vec<TierChangeEvent>,

    /// Cycle when tier was last updated
    pub last_updated_cycle: u64,
}

impl ValidatorTierInfo {
    /// Create new validator tier info
    pub fn new(validator_id: ValidatorID, stake_amount: u64, cycle: u64) -> Self {
        let tier = ValidatorTier::from_stake(stake_amount);

        Self {
            validator_id,
            current_tier: tier,
            stake_amount,
            tier_history: Vec::new(),
            last_updated_cycle: cycle,
        }
    }

    /// Update stake and check for tier change
    pub fn update_stake(&mut self, new_stake: u64, cycle: u64) -> Option<TierChangeEvent> {
        let old_tier = self.current_tier;
        let new_tier = ValidatorTier::from_stake(new_stake);

        self.stake_amount = new_stake;
        self.last_updated_cycle = cycle;

        if new_tier != old_tier {
            let event = TierChangeEvent::new(
                self.validator_id.clone(),
                old_tier,
                new_tier,
                new_stake,
                cycle,
            );

            self.current_tier = new_tier;
            self.tier_history.insert(0, event.clone());

            // Keep only last 100 tier changes
            if self.tier_history.len() > 100 {
                self.tier_history.truncate(100);
            }

            if event.is_upgrade() {
                info!(
                    "Validator {} upgraded from {} to {} tier (stake: {} SBTC)",
                    self.validator_id, old_tier, new_tier, new_stake
                );
            } else {
                warn!(
                    "Validator {} downgraded from {} to {} tier (stake: {} SBTC)",
                    self.validator_id, old_tier, new_tier, new_stake
                );
            }

            Some(event)
        } else {
            None
        }
    }

    /// Get effective voting power (stake * multiplier)
    pub fn effective_voting_power(&self) -> u64 {
        let multiplier = self.current_tier.voting_power_multiplier();
        (self.stake_amount as f64 * multiplier) as u64
    }

    /// Get effective reward multiplier
    pub fn reward_multiplier(&self) -> f64 {
        self.current_tier.reward_multiplier()
    }

    /// Get tier upgrade path
    pub fn upgrade_path(&self) -> Vec<(ValidatorTier, u64)> {
        ValidatorTier::all_tiers()
            .into_iter()
            .filter(|tier| *tier > self.current_tier)
            .map(|tier| {
                let required_stake = tier.min_stake();
                let additional_needed = if self.stake_amount < required_stake {
                    required_stake - self.stake_amount
                } else {
                    0
                };
                (tier, additional_needed)
            })
            .collect()
    }

    /// Check if validator can upgrade to target tier
    pub fn can_upgrade_to(&self, target: ValidatorTier) -> bool {
        self.current_tier.can_upgrade_to(target, self.stake_amount)
    }

    /// Get tier change count
    pub fn tier_change_count(&self) -> usize {
        self.tier_history.len()
    }

    /// Get most recent tier change
    pub fn last_tier_change(&self) -> Option<&TierChangeEvent> {
        self.tier_history.first()
    }
}

/// Validator tier manager
///
/// Manages tier assignments and transitions for all validators
pub struct ValidatorTierManager {
    /// Tier information for each validator
    tiers: std::collections::HashMap<ValidatorID, ValidatorTierInfo>,

    /// Current cycle
    current_cycle: u64,

    /// All tier change events
    all_tier_changes: Vec<TierChangeEvent>,
}

impl ValidatorTierManager {
    /// Create new validator tier manager
    pub fn new() -> Self {
        Self {
            tiers: std::collections::HashMap::new(),
            current_cycle: 0,
            all_tier_changes: Vec::new(),
        }
    }

    /// Register a validator with initial stake
    pub fn register_validator(
        &mut self,
        validator_id: ValidatorID,
        stake: u64,
    ) -> Result<ValidatorTier> {
        if stake < ValidatorTier::Bronze.min_stake() {
            return Err(Error::InvalidData(format!(
                "Stake {} is below minimum tier requirement of {} SBTC",
                stake,
                ValidatorTier::Bronze.min_stake()
            )));
        }

        if self.tiers.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} already registered",
                validator_id
            )));
        }

        let tier_info = ValidatorTierInfo::new(validator_id.clone(), stake, self.current_cycle);
        let tier = tier_info.current_tier;

        self.tiers.insert(validator_id.clone(), tier_info);

        info!(
            "Registered validator {} at {} tier with {} SBTC",
            validator_id, tier, stake
        );

        Ok(tier)
    }

    /// Update validator stake and handle tier changes
    pub fn update_validator_stake(
        &mut self,
        validator_id: &ValidatorID,
        new_stake: u64,
    ) -> Result<Option<TierChangeEvent>> {
        let tier_info = self
            .tiers
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        if new_stake < ValidatorTier::Bronze.min_stake() {
            return Err(Error::InvalidData(format!(
                "Stake {} is below minimum tier requirement of {} SBTC",
                new_stake,
                ValidatorTier::Bronze.min_stake()
            )));
        }

        let event = tier_info.update_stake(new_stake, self.current_cycle);

        if let Some(ref e) = event {
            self.all_tier_changes.push(e.clone());
        }

        Ok(event)
    }

    /// Get validator tier
    pub fn get_tier(&self, validator_id: &ValidatorID) -> Option<ValidatorTier> {
        self.tiers.get(validator_id).map(|info| info.current_tier)
    }

    /// Get validator tier info
    pub fn get_tier_info(&self, validator_id: &ValidatorID) -> Option<&ValidatorTierInfo> {
        self.tiers.get(validator_id)
    }

    /// Get effective voting power for validator
    pub fn get_voting_power(&self, validator_id: &ValidatorID) -> u64 {
        self.tiers
            .get(validator_id)
            .map(|info| info.effective_voting_power())
            .unwrap_or(0)
    }

    /// Get reward multiplier for validator
    pub fn get_reward_multiplier(&self, validator_id: &ValidatorID) -> f64 {
        self.tiers
            .get(validator_id)
            .map(|info| info.reward_multiplier())
            .unwrap_or(1.0)
    }

    /// Get all validators by tier
    pub fn get_validators_by_tier(&self, tier: ValidatorTier) -> Vec<ValidatorID> {
        self.tiers
            .iter()
            .filter(|(_, info)| info.current_tier == tier)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get tier distribution
    pub fn get_tier_distribution(&self) -> std::collections::HashMap<ValidatorTier, usize> {
        let mut distribution = std::collections::HashMap::new();

        for tier in ValidatorTier::all_tiers() {
            distribution.insert(tier, 0);
        }

        for info in self.tiers.values() {
            *distribution.entry(info.current_tier).or_insert(0) += 1;
        }

        distribution
    }

    /// Get total voting power across all validators
    pub fn total_voting_power(&self) -> u64 {
        self.tiers
            .values()
            .map(|info| info.effective_voting_power())
            .sum()
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) {
        self.current_cycle += 1;
        info!("Advanced tier manager to cycle {}", self.current_cycle);
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get all tier change events
    pub fn get_tier_changes(&self) -> &[TierChangeEvent] {
        &self.all_tier_changes
    }

    /// Get tier changes for specific validator
    pub fn get_validator_tier_changes(&self, validator_id: &ValidatorID) -> Vec<TierChangeEvent> {
        self.all_tier_changes
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .cloned()
            .collect()
    }

    /// Remove validator
    pub fn remove_validator(&mut self, validator_id: &ValidatorID) -> Result<()> {
        self.tiers
            .remove(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        info!("Removed validator {} from tier system", validator_id);
        Ok(())
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.tiers.len()
    }

    /// Clear all validators
    pub fn clear(&mut self) {
        self.tiers.clear();
        self.all_tier_changes.clear();
        info!("Cleared all validator tiers");
    }
}

impl Default for ValidatorTierManager {
    fn default() -> Self {
        Self::new()
    }
}
