//! Validator staking operations with multi-tier support
//!
//! This module handles:
//! - Multi-tier validator stake deposits (10K-500K SBTC)
//! - Tier-based stake validation
//! - Unstaking with 7-day unbonding period
//! - Tier change tracking and events
//! - Stake tracking and validation

use crate::validator_tiers::{ValidatorTier, TierChangeEvent};
use silver_core::{Error, Result, ValidatorID};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Minimum stake amount for Bronze tier (10,000 SBTC)
pub const MIN_STAKE_AMOUNT: u64 = 10_000;

/// Unbonding period in seconds (7 days)
pub const UNBONDING_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;

/// Stake deposit record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeDeposit {
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Stake amount in SBTC
    pub amount: u64,
    
    /// Tier at time of deposit
    pub tier: ValidatorTier,
    
    /// Deposit timestamp
    pub deposited_at: u64,
    
    /// Transaction digest that deposited the stake
    pub deposit_tx: Vec<u8>,
}

impl StakeDeposit {
    /// Create a new stake deposit with tier detection
    pub fn new(
        validator_id: ValidatorID,
        amount: u64,
        deposit_tx: Vec<u8>,
    ) -> Result<Self> {
        if amount < MIN_STAKE_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Stake amount {} is below minimum {} (Bronze tier)",
                amount, MIN_STAKE_AMOUNT
            )));
        }

        if deposit_tx.len() != 64 {
            return Err(Error::InvalidData(format!(
                "Transaction digest must be 64 bytes, got {}",
                deposit_tx.len()
            )));
        }

        let tier = ValidatorTier::from_stake(amount);
        let deposited_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        info!(
            "Creating stake deposit for validator {} at {} tier with {} SBTC",
            validator_id, tier, amount
        );

        Ok(Self {
            validator_id,
            amount,
            tier,
            deposited_at,
            deposit_tx,
        })
    }
}

/// Unstaking request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnstakingRequest {
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Amount to unstake
    pub amount: u64,
    
    /// Request timestamp
    pub requested_at: u64,
    
    /// Unbonding completion timestamp
    pub unbonds_at: u64,
    
    /// Whether the unstaking is complete
    pub completed: bool,
}

impl UnstakingRequest {
    /// Create a new unstaking request
    pub fn new(validator_id: ValidatorID, amount: u64) -> Self {
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let unbonds_at = requested_at + UNBONDING_PERIOD_SECS;

        Self {
            validator_id,
            amount,
            requested_at,
            unbonds_at,
            completed: false,
        }
    }

    /// Check if unbonding period is complete
    pub fn is_unbonded(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now >= self.unbonds_at
    }

    /// Get remaining unbonding time in seconds
    pub fn remaining_unbonding_time(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if now >= self.unbonds_at {
            0
        } else {
            self.unbonds_at - now
        }
    }
}

/// Validator stake information with tier tracking
#[derive(Debug, Clone)]
pub struct ValidatorStake {
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Total staked amount
    pub total_stake: u64,
    
    /// Active stake (not unbonding)
    pub active_stake: u64,
    
    /// Unbonding stake
    pub unbonding_stake: u64,
    
    /// Current tier
    pub current_tier: ValidatorTier,
    
    /// Stake deposits
    pub deposits: Vec<StakeDeposit>,
    
    /// Pending unstaking requests
    pub unstaking_requests: Vec<UnstakingRequest>,
    
    /// Tier change history
    pub tier_history: Vec<TierChangeEvent>,
}

impl ValidatorStake {
    /// Create new validator stake
    pub fn new(validator_id: ValidatorID) -> Self {
        Self {
            validator_id,
            total_stake: 0,
            active_stake: 0,
            unbonding_stake: 0,
            current_tier: ValidatorTier::Bronze, // Default to lowest tier
            deposits: Vec::new(),
            unstaking_requests: Vec::new(),
            tier_history: Vec::new(),
        }
    }

    /// Add a stake deposit and update tier
    pub fn add_deposit(&mut self, deposit: StakeDeposit, cycle: u64) -> Option<TierChangeEvent> {
        let old_tier = self.current_tier;
        
        self.total_stake += deposit.amount;
        self.active_stake += deposit.amount;
        self.deposits.push(deposit);
        
        // Update tier based on new active stake
        let new_tier = ValidatorTier::from_stake(self.active_stake);
        
        if new_tier != old_tier {
            let event = TierChangeEvent::new(
                self.validator_id.clone(),
                old_tier,
                new_tier,
                self.active_stake,
                cycle,
            );
            
            self.current_tier = new_tier;
            self.tier_history.push(event.clone());
            
            info!(
                "Validator {} tier changed from {} to {} (stake: {} SBTC)",
                self.validator_id, old_tier, new_tier, self.active_stake
            );
            
            Some(event)
        } else {
            None
        }
    }

    /// Request unstaking and update tier
    pub fn request_unstake(&mut self, amount: u64, cycle: u64) -> Result<(UnstakingRequest, Option<TierChangeEvent>)> {
        if amount > self.active_stake {
            return Err(Error::InvalidData(format!(
                "Cannot unstake {} SBTC, only {} active",
                amount, self.active_stake
            )));
        }

        let old_tier = self.current_tier;
        let request = UnstakingRequest::new(self.validator_id.clone(), amount);
        
        self.active_stake -= amount;
        self.unbonding_stake += amount;
        self.unstaking_requests.push(request.clone());

        // Check for tier downgrade
        let new_tier = ValidatorTier::from_stake(self.active_stake);
        let tier_event = if new_tier != old_tier {
            let event = TierChangeEvent::new(
                self.validator_id.clone(),
                old_tier,
                new_tier,
                self.active_stake,
                cycle,
            );
            
            self.current_tier = new_tier;
            self.tier_history.push(event.clone());
            
            warn!(
                "Validator {} tier downgraded from {} to {} after unstaking (remaining: {} SBTC)",
                self.validator_id, old_tier, new_tier, self.active_stake
            );
            
            Some(event)
        } else {
            None
        };

        Ok((request, tier_event))
    }

    /// Process completed unbonding requests
    pub fn process_unbonding(&mut self) -> Vec<UnstakingRequest> {
        let mut completed = Vec::new();

        for request in &mut self.unstaking_requests {
            if !request.completed && request.is_unbonded() {
                request.completed = true;
                self.unbonding_stake -= request.amount;
                self.total_stake -= request.amount;
                completed.push(request.clone());
            }
        }

        // Remove completed requests
        self.unstaking_requests.retain(|r| !r.completed);

        completed
    }

    /// Check if validator meets minimum stake requirement
    pub fn meets_minimum_stake(&self) -> bool {
        self.active_stake >= MIN_STAKE_AMOUNT
    }

    /// Get current tier
    pub fn tier(&self) -> ValidatorTier {
        self.current_tier
    }

    /// Get tier history
    pub fn tier_history(&self) -> &[TierChangeEvent] {
        &self.tier_history
    }

    /// Get effective voting power (stake * tier multiplier)
    pub fn effective_voting_power(&self) -> u64 {
        let multiplier = self.current_tier.voting_power_multiplier();
        (self.active_stake as f64 * multiplier) as u64
    }

    /// Get reward multiplier for current tier
    pub fn reward_multiplier(&self) -> f64 {
        self.current_tier.reward_multiplier()
    }
}

/// Staking manager with multi-tier support
///
/// Manages all validator staking operations including deposits,
/// unstaking requests, unbonding periods, and tier tracking.
pub struct StakingManager {
    /// Validator stakes indexed by validator ID
    stakes: HashMap<ValidatorID, ValidatorStake>,
    
    /// Total staked amount across all validators
    total_staked: u64,
    
    /// Current cycle for tier change tracking
    current_cycle: u64,
    
    /// All tier change events
    all_tier_changes: Vec<TierChangeEvent>,
}

impl StakingManager {
    /// Create a new staking manager
    pub fn new() -> Self {
        Self {
            stakes: HashMap::new(),
            total_staked: 0,
            current_cycle: 0,
            all_tier_changes: Vec::new(),
        }
    }

    /// Deposit stake for a validator with tier detection
    pub fn deposit_stake(
        &mut self,
        validator_id: ValidatorID,
        amount: u64,
        deposit_tx: Vec<u8>,
    ) -> Result<Option<TierChangeEvent>> {
        if amount < MIN_STAKE_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Stake amount {} is below minimum {} (Bronze tier)",
                amount, MIN_STAKE_AMOUNT
            )));
        }

        let deposit = StakeDeposit::new(validator_id.clone(), amount, deposit_tx)?;
        let tier = deposit.tier;

        let stake = self.stakes
            .entry(validator_id.clone())
            .or_insert_with(|| ValidatorStake::new(validator_id.clone()));

        let tier_event = stake.add_deposit(deposit, self.current_cycle);
        self.total_staked += amount;

        if let Some(ref event) = tier_event {
            self.all_tier_changes.push(event.clone());
        }

        info!(
            "Validator {} deposited {} SBTC stake at {} tier (total: {}, active: {})",
            validator_id, amount, tier, stake.total_stake, stake.active_stake
        );

        Ok(tier_event)
    }

    /// Request unstaking for a validator with tier update
    pub fn request_unstake(
        &mut self,
        validator_id: &ValidatorID,
        amount: u64,
    ) -> Result<(UnstakingRequest, Option<TierChangeEvent>)> {
        let stake = self.stakes
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!(
                "Validator {} has no stake",
                validator_id
            )))?;

        let (request, tier_event) = stake.request_unstake(amount, self.current_cycle)?;

        if let Some(ref event) = tier_event {
            self.all_tier_changes.push(event.clone());
        }

        info!(
            "Validator {} requested unstaking {} SBTC (unbonds at: {}, tier: {})",
            validator_id, amount, request.unbonds_at, stake.current_tier
        );

        Ok((request, tier_event))
    }

    /// Process all unbonding requests
    pub fn process_unbonding(&mut self) -> HashMap<ValidatorID, Vec<UnstakingRequest>> {
        let mut completed_by_validator = HashMap::new();

        for (validator_id, stake) in &mut self.stakes {
            let completed = stake.process_unbonding();
            
            if !completed.is_empty() {
                info!(
                    "Validator {} completed {} unbonding requests",
                    validator_id,
                    completed.len()
                );
                
                // Update total staked
                for request in &completed {
                    self.total_staked -= request.amount;
                }
                
                completed_by_validator.insert(validator_id.clone(), completed);
            }
        }

        completed_by_validator
    }

    /// Get validator stake
    pub fn get_stake(&self, validator_id: &ValidatorID) -> Option<&ValidatorStake> {
        self.stakes.get(validator_id)
    }

    /// Get validator active stake amount
    pub fn get_active_stake(&self, validator_id: &ValidatorID) -> u64 {
        self.stakes
            .get(validator_id)
            .map(|s| s.active_stake)
            .unwrap_or(0)
    }

    /// Get total staked amount
    pub fn total_staked(&self) -> u64 {
        self.total_staked
    }

    /// Get all validators with active stake
    pub fn get_staked_validators(&self) -> Vec<ValidatorID> {
        self.stakes
            .iter()
            .filter(|(_, stake)| stake.active_stake > 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Check if validator meets minimum stake requirement
    pub fn meets_minimum_stake(&self, validator_id: &ValidatorID) -> bool {
        self.stakes
            .get(validator_id)
            .map(|s| s.meets_minimum_stake())
            .unwrap_or(false)
    }

    /// Get validators below minimum stake
    pub fn get_below_minimum_stake(&self) -> Vec<ValidatorID> {
        self.stakes
            .iter()
            .filter(|(_, stake)| !stake.meets_minimum_stake())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Remove validator stake (after full unstaking)
    pub fn remove_validator(&mut self, validator_id: &ValidatorID) -> Result<()> {
        if let Some(stake) = self.stakes.get(validator_id) {
            if stake.active_stake > 0 || stake.unbonding_stake > 0 {
                return Err(Error::InvalidData(format!(
                    "Cannot remove validator {} with active or unbonding stake",
                    validator_id
                )));
            }
        }

        self.stakes.remove(validator_id);
        info!("Removed validator {} from staking", validator_id);

        Ok(())
    }

    /// Get validator tier
    pub fn get_tier(&self, validator_id: &ValidatorID) -> Option<ValidatorTier> {
        self.stakes.get(validator_id).map(|s| s.tier())
    }

    /// Get validator effective voting power
    pub fn get_voting_power(&self, validator_id: &ValidatorID) -> u64 {
        self.stakes
            .get(validator_id)
            .map(|s| s.effective_voting_power())
            .unwrap_or(0)
    }

    /// Get validator reward multiplier
    pub fn get_reward_multiplier(&self, validator_id: &ValidatorID) -> f64 {
        self.stakes
            .get(validator_id)
            .map(|s| s.reward_multiplier())
            .unwrap_or(1.0)
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

    /// Get validators by tier
    pub fn get_validators_by_tier(&self, tier: ValidatorTier) -> Vec<ValidatorID> {
        self.stakes
            .iter()
            .filter(|(_, stake)| stake.tier() == tier)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get tier distribution
    pub fn get_tier_distribution(&self) -> HashMap<ValidatorTier, usize> {
        let mut distribution = HashMap::new();
        
        for tier in ValidatorTier::all_tiers() {
            distribution.insert(tier, 0);
        }
        
        for stake in self.stakes.values() {
            *distribution.entry(stake.tier()).or_insert(0) += 1;
        }
        
        distribution
    }

    /// Get total voting power across all validators
    pub fn total_voting_power(&self) -> u64 {
        self.stakes
            .values()
            .map(|s| s.effective_voting_power())
            .sum()
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) {
        self.current_cycle += 1;
        info!("Advanced staking manager to cycle {}", self.current_cycle);
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }
}

impl Default for StakingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::SilverAddress;

    fn create_test_validator_id(id: u8) -> ValidatorID {
        ValidatorID::new(SilverAddress::new([id; 64]))
    }

    #[test]
    fn test_stake_deposit_minimum() {
        let validator_id = create_test_validator_id(1);
        
        // Below minimum should fail
        let result = StakeDeposit::new(validator_id.clone(), 9_999, vec![0u8; 64]);
        assert!(result.is_err());

        // At minimum should succeed (Bronze tier: 10,000)
        let result = StakeDeposit::new(validator_id.clone(), 10_000, vec![0u8; 64]);
        assert!(result.is_ok());
        let deposit = result.unwrap();
        assert_eq!(deposit.tier, ValidatorTier::Bronze);

        // Silver tier
        let result = StakeDeposit::new(validator_id.clone(), 50_000, vec![0u8; 64]);
        assert!(result.is_ok());
        let deposit = result.unwrap();
        assert_eq!(deposit.tier, ValidatorTier::Silver);
    }

    #[test]
    fn test_unstaking_request() {
        let validator_id = create_test_validator_id(1);
        let request = UnstakingRequest::new(validator_id, 1_000_000);

        assert!(!request.is_unbonded());
        assert!(request.remaining_unbonding_time() > 0);
        assert_eq!(request.unbonds_at - request.requested_at, UNBONDING_PERIOD_SECS);
    }

    #[test]
    fn test_validator_stake() {
        let validator_id = create_test_validator_id(1);
        let mut stake = ValidatorStake::new(validator_id.clone());

        // Add deposit (Platinum tier: 500,000+)
        let deposit = StakeDeposit::new(validator_id.clone(), 500_000, vec![0u8; 64]).unwrap();
        let tier_event = stake.add_deposit(deposit, 0);
        
        assert!(tier_event.is_some()); // Tier changed from Bronze to Platinum
        assert_eq!(stake.total_stake, 500_000);
        assert_eq!(stake.active_stake, 500_000);
        assert_eq!(stake.unbonding_stake, 0);
        assert_eq!(stake.tier(), ValidatorTier::Platinum);

        // Request unstake (should downgrade to Gold)
        let (_request, tier_event) = stake.request_unstake(400_000, 1).unwrap();
        assert!(tier_event.is_some()); // Tier changed from Platinum to Gold
        assert_eq!(stake.active_stake, 100_000);
        assert_eq!(stake.unbonding_stake, 400_000);
        assert_eq!(stake.total_stake, 500_000);
        assert_eq!(stake.tier(), ValidatorTier::Gold);
    }

    #[test]
    fn test_staking_manager() {
        let mut manager = StakingManager::new();
        let validator_id = create_test_validator_id(1);

        // Deposit stake (Platinum tier)
        let tier_event = manager.deposit_stake(validator_id.clone(), 500_000, vec![1u8; 64]).unwrap();
        assert!(tier_event.is_some());
        assert_eq!(manager.total_staked(), 500_000);
        assert_eq!(manager.get_active_stake(&validator_id), 500_000);
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Platinum));

        // Request unstake (should downgrade to Gold)
        let (request, tier_event) = manager.request_unstake(&validator_id, 400_000).unwrap();
        assert!(tier_event.is_some());
        assert_eq!(manager.get_active_stake(&validator_id), 100_000);
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Gold));
        assert!(!request.is_unbonded());
    }

    #[test]
    fn test_minimum_stake_requirement() {
        let mut manager = StakingManager::new();
        let validator_id = create_test_validator_id(1);

        // At minimum (Bronze tier: 10,000)
        manager.deposit_stake(validator_id.clone(), 10_000, vec![1u8; 64]).unwrap();
        assert!(manager.meets_minimum_stake(&validator_id));
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Bronze));

        // Unstake to below minimum
        manager.request_unstake(&validator_id, 5_000).unwrap();
        assert!(!manager.meets_minimum_stake(&validator_id));
    }

    #[test]
    fn test_get_staked_validators() {
        let mut manager = StakingManager::new();
        
        manager.deposit_stake(create_test_validator_id(1), 50_000, vec![1u8; 64]).unwrap();
        manager.deposit_stake(create_test_validator_id(2), 100_000, vec![2u8; 64]).unwrap();

        let validators = manager.get_staked_validators();
        assert_eq!(validators.len(), 2);
    }

    #[test]
    fn test_tier_tracking() {
        let mut manager = StakingManager::new();
        let validator_id = create_test_validator_id(1);

        // Start at Bronze
        manager.deposit_stake(validator_id.clone(), 10_000, vec![1u8; 64]).unwrap();
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Bronze));

        // Upgrade to Silver
        manager.deposit_stake(validator_id.clone(), 40_000, vec![2u8; 64]).unwrap();
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Silver));

        // Upgrade to Gold
        manager.deposit_stake(validator_id.clone(), 50_000, vec![3u8; 64]).unwrap();
        assert_eq!(manager.get_tier(&validator_id), Some(ValidatorTier::Gold));

        // Check tier history
        let changes = manager.get_validator_tier_changes(&validator_id);
        assert_eq!(changes.len(), 2); // Bronze->Silver, Silver->Gold
    }

    #[test]
    fn test_voting_power_calculation() {
        let mut manager = StakingManager::new();
        
        let id1 = create_test_validator_id(1);
        let id2 = create_test_validator_id(2);
        
        // Bronze: 10,000 * 0.5 = 5,000
        manager.deposit_stake(id1.clone(), 10_000, vec![1u8; 64]).unwrap();
        assert_eq!(manager.get_voting_power(&id1), 5_000);
        
        // Gold: 100,000 * 1.5 = 150,000
        manager.deposit_stake(id2.clone(), 100_000, vec![2u8; 64]).unwrap();
        assert_eq!(manager.get_voting_power(&id2), 150_000);
        
        // Total voting power
        assert_eq!(manager.total_voting_power(), 155_000);
    }

    #[test]
    fn test_tier_distribution() {
        let mut manager = StakingManager::new();
        
        manager.deposit_stake(create_test_validator_id(1), 10_000, vec![1u8; 64]).unwrap();
        manager.deposit_stake(create_test_validator_id(2), 50_000, vec![2u8; 64]).unwrap();
        manager.deposit_stake(create_test_validator_id(3), 100_000, vec![3u8; 64]).unwrap();
        manager.deposit_stake(create_test_validator_id(4), 500_000, vec![4u8; 64]).unwrap();
        
        let distribution = manager.get_tier_distribution();
        assert_eq!(distribution[&ValidatorTier::Bronze], 1);
        assert_eq!(distribution[&ValidatorTier::Silver], 1);
        assert_eq!(distribution[&ValidatorTier::Gold], 1);
        assert_eq!(distribution[&ValidatorTier::Platinum], 1);
    }
}
