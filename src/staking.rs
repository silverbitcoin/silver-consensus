//! Validator staking operations
//!
//! This module handles:
//! - Validator stake deposits (minimum 1M SBTC)
//! - Unstaking with 7-day unbonding period
//! - Stake tracking and validation

use silver_core::{Error, Result, SilverAddress, ValidatorID, ValidatorMetadata};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Minimum stake amount (1,000,000 SBTC)
pub const MIN_STAKE_AMOUNT: u64 = 1_000_000;

/// Unbonding period in seconds (7 days)
pub const UNBONDING_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;

/// Stake deposit record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeDeposit {
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Stake amount in SBTC
    pub amount: u64,
    
    /// Deposit timestamp
    pub deposited_at: u64,
    
    /// Transaction digest that deposited the stake
    pub deposit_tx: Vec<u8>,
}

impl StakeDeposit {
    /// Create a new stake deposit
    pub fn new(
        validator_id: ValidatorID,
        amount: u64,
        deposit_tx: Vec<u8>,
    ) -> Result<Self> {
        if amount < MIN_STAKE_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Stake amount {} is below minimum {}",
                amount, MIN_STAKE_AMOUNT
            )));
        }

        if deposit_tx.len() != 64 {
            return Err(Error::InvalidData(format!(
                "Transaction digest must be 64 bytes, got {}",
                deposit_tx.len()
            )));
        }

        let deposited_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(Self {
            validator_id,
            amount,
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

/// Validator stake information
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
    
    /// Stake deposits
    pub deposits: Vec<StakeDeposit>,
    
    /// Pending unstaking requests
    pub unstaking_requests: Vec<UnstakingRequest>,
}

impl ValidatorStake {
    /// Create new validator stake
    pub fn new(validator_id: ValidatorID) -> Self {
        Self {
            validator_id,
            total_stake: 0,
            active_stake: 0,
            unbonding_stake: 0,
            deposits: Vec::new(),
            unstaking_requests: Vec::new(),
        }
    }

    /// Add a stake deposit
    pub fn add_deposit(&mut self, deposit: StakeDeposit) {
        self.total_stake += deposit.amount;
        self.active_stake += deposit.amount;
        self.deposits.push(deposit);
    }

    /// Request unstaking
    pub fn request_unstake(&mut self, amount: u64) -> Result<UnstakingRequest> {
        if amount > self.active_stake {
            return Err(Error::InvalidData(format!(
                "Cannot unstake {} SBTC, only {} active",
                amount, self.active_stake
            )));
        }

        let request = UnstakingRequest::new(self.validator_id.clone(), amount);
        
        self.active_stake -= amount;
        self.unbonding_stake += amount;
        self.unstaking_requests.push(request.clone());

        Ok(request)
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
}

/// Staking manager
///
/// Manages all validator staking operations including deposits,
/// unstaking requests, and unbonding periods.
pub struct StakingManager {
    /// Validator stakes indexed by validator ID
    stakes: HashMap<ValidatorID, ValidatorStake>,
    
    /// Total staked amount across all validators
    total_staked: u64,
}

impl StakingManager {
    /// Create a new staking manager
    pub fn new() -> Self {
        Self {
            stakes: HashMap::new(),
            total_staked: 0,
        }
    }

    /// Deposit stake for a validator
    pub fn deposit_stake(
        &mut self,
        validator_id: ValidatorID,
        amount: u64,
        deposit_tx: Vec<u8>,
    ) -> Result<()> {
        if amount < MIN_STAKE_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Stake amount {} is below minimum {}",
                amount, MIN_STAKE_AMOUNT
            )));
        }

        let deposit = StakeDeposit::new(validator_id.clone(), amount, deposit_tx)?;

        let stake = self.stakes
            .entry(validator_id.clone())
            .or_insert_with(|| ValidatorStake::new(validator_id.clone()));

        stake.add_deposit(deposit);
        self.total_staked += amount;

        info!(
            "Validator {} deposited {} SBTC stake (total: {})",
            validator_id, amount, stake.total_stake
        );

        Ok(())
    }

    /// Request unstaking for a validator
    pub fn request_unstake(
        &mut self,
        validator_id: &ValidatorID,
        amount: u64,
    ) -> Result<UnstakingRequest> {
        let stake = self.stakes
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!(
                "Validator {} has no stake",
                validator_id
            )))?;

        let request = stake.request_unstake(amount)?;

        info!(
            "Validator {} requested unstaking {} SBTC (unbonds at: {})",
            validator_id, amount, request.unbonds_at
        );

        Ok(request)
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
        let result = StakeDeposit::new(validator_id.clone(), 999_999, vec![0u8; 64]);
        assert!(result.is_err());

        // At minimum should succeed
        let result = StakeDeposit::new(validator_id, 1_000_000, vec![0u8; 64]);
        assert!(result.is_ok());
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

        // Add deposit
        let deposit = StakeDeposit::new(validator_id.clone(), 2_000_000, vec![0u8; 64]).unwrap();
        stake.add_deposit(deposit);

        assert_eq!(stake.total_stake, 2_000_000);
        assert_eq!(stake.active_stake, 2_000_000);
        assert_eq!(stake.unbonding_stake, 0);

        // Request unstake
        let request = stake.request_unstake(500_000).unwrap();
        assert_eq!(stake.active_stake, 1_500_000);
        assert_eq!(stake.unbonding_stake, 500_000);
        assert_eq!(stake.total_stake, 2_000_000);
    }

    #[test]
    fn test_staking_manager() {
        let mut manager = StakingManager::new();
        let validator_id = create_test_validator_id(1);

        // Deposit stake
        manager.deposit_stake(validator_id.clone(), 2_000_000, vec![1u8; 64]).unwrap();
        assert_eq!(manager.total_staked(), 2_000_000);
        assert_eq!(manager.get_active_stake(&validator_id), 2_000_000);

        // Request unstake
        let request = manager.request_unstake(&validator_id, 500_000).unwrap();
        assert_eq!(manager.get_active_stake(&validator_id), 1_500_000);
        assert!(!request.is_unbonded());
    }

    #[test]
    fn test_minimum_stake_requirement() {
        let mut manager = StakingManager::new();
        let validator_id = create_test_validator_id(1);

        // Below minimum
        manager.deposit_stake(validator_id.clone(), 1_000_000, vec![1u8; 64]).unwrap();
        assert!(manager.meets_minimum_stake(&validator_id));

        // Unstake to below minimum
        manager.request_unstake(&validator_id, 500_000).unwrap();
        assert!(!manager.meets_minimum_stake(&validator_id));
    }

    #[test]
    fn test_get_staked_validators() {
        let mut manager = StakingManager::new();
        
        manager.deposit_stake(create_test_validator_id(1), 1_000_000, vec![1u8; 64]).unwrap();
        manager.deposit_stake(create_test_validator_id(2), 2_000_000, vec![2u8; 64]).unwrap();

        let validators = manager.get_staked_validators();
        assert_eq!(validators.len(), 2);
    }
}
