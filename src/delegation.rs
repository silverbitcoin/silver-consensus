//! Delegated staking system
//!
//! This module implements delegated staking where users can delegate their
//! stake to validators and earn rewards proportional to their delegation.
//!
//! Features:
//! - Minimum delegation: 10 SBTC
//! - Maximum delegation per validator: 10M SBTC
//! - 7-day unbonding period
//! - Instant redelegation (no unbonding)
//! - Delegation receipt tokens
//! - Prevention of delegation to jailed/inactive validators

use crate::UNBONDING_PERIOD_SECS;
use silver_core::{Error, Result, SilverAddress, ValidatorID};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

/// Minimum delegation amount (10 SBTC)
pub const MIN_DELEGATION_AMOUNT: u64 = 10;

/// Maximum total delegated stake per validator (10,000,000 SBTC)
pub const MAX_DELEGATED_STAKE_PER_VALIDATOR: u64 = 10_000_000;

/// Delegation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delegation {
    /// Delegator address
    pub delegator: SilverAddress,
    
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Delegated amount
    pub amount: u64,
    
    /// Accumulated rewards
    pub accumulated_rewards: u64,
    
    /// Delegation timestamp
    pub delegated_at: u64,
    
    /// Receipt token ID
    pub receipt_token_id: Vec<u8>,
}

impl Delegation {
    /// Create a new delegation
    pub fn new(
        delegator: SilverAddress,
        validator_id: ValidatorID,
        amount: u64,
    ) -> Result<Self> {
        if amount < MIN_DELEGATION_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Delegation amount {} is below minimum {}",
                amount, MIN_DELEGATION_AMOUNT
            )));
        }

        let delegated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Generate receipt token ID
        let mut hasher = blake3::Hasher::new();
        hasher.update(delegator.as_bytes());
        hasher.update(validator_id.address.as_bytes());
        hasher.update(&amount.to_le_bytes());
        hasher.update(&delegated_at.to_le_bytes());
        
        let mut receipt_token_id = vec![0u8; 64];
        hasher.finalize_xof().fill(&mut receipt_token_id);

        Ok(Self {
            delegator,
            validator_id,
            amount,
            accumulated_rewards: 0,
            delegated_at,
            receipt_token_id,
        })
    }

    /// Add rewards to delegation
    pub fn add_rewards(&mut self, rewards: u64) {
        self.accumulated_rewards += rewards;
    }

    /// Claim accumulated rewards
    pub fn claim_rewards(&mut self) -> u64 {
        let rewards = self.accumulated_rewards;
        self.accumulated_rewards = 0;
        rewards
    }

    /// Get total value (delegation + rewards)
    pub fn total_value(&self) -> u64 {
        self.amount + self.accumulated_rewards
    }
}

/// Undelegation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndelegationRequest {
    /// Delegator address
    pub delegator: SilverAddress,
    
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Amount to undelegate
    pub amount: u64,
    
    /// Accumulated rewards at time of undelegation
    pub rewards: u64,
    
    /// Request timestamp
    pub requested_at: u64,
    
    /// Unbonding completion timestamp
    pub unbonds_at: u64,
    
    /// Whether the undelegation is complete
    pub completed: bool,
}

impl UndelegationRequest {
    /// Create a new undelegation request
    pub fn new(
        delegator: SilverAddress,
        validator_id: ValidatorID,
        amount: u64,
        rewards: u64,
    ) -> Self {
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let unbonds_at = requested_at + UNBONDING_PERIOD_SECS;

        Self {
            delegator,
            validator_id,
            amount,
            rewards,
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

    /// Get total return (amount + rewards)
    pub fn total_return(&self) -> u64 {
        self.amount + self.rewards
    }
}

/// Validator delegation info
#[derive(Debug, Clone)]
pub struct ValidatorDelegationInfo {
    /// Validator ID
    pub validator_id: ValidatorID,
    
    /// Total delegated stake
    pub total_delegated: u64,
    
    /// Active delegations
    pub delegations: HashMap<SilverAddress, Delegation>,
    
    /// Pending undelegation requests
    pub undelegation_requests: Vec<UndelegationRequest>,
    
    /// Whether validator is active
    pub is_active: bool,
    
    /// Whether validator is jailed
    pub is_jailed: bool,
}

impl ValidatorDelegationInfo {
    /// Create new validator delegation info
    pub fn new(validator_id: ValidatorID) -> Self {
        Self {
            validator_id,
            total_delegated: 0,
            delegations: HashMap::new(),
            undelegation_requests: Vec::new(),
            is_active: true,
            is_jailed: false,
        }
    }

    /// Add a delegation
    pub fn add_delegation(&mut self, delegation: Delegation) -> Result<()> {
        if !self.is_active {
            return Err(Error::InvalidData(format!(
                "Cannot delegate to inactive validator {}",
                self.validator_id
            )));
        }

        if self.is_jailed {
            return Err(Error::InvalidData(format!(
                "Cannot delegate to jailed validator {}",
                self.validator_id
            )));
        }

        let new_total = self.total_delegated + delegation.amount;
        if new_total > MAX_DELEGATED_STAKE_PER_VALIDATOR {
            return Err(Error::InvalidData(format!(
                "Delegation would exceed maximum {} SBTC for validator {}",
                MAX_DELEGATED_STAKE_PER_VALIDATOR, self.validator_id
            )));
        }

        let delegator = delegation.delegator;
        let amount = delegation.amount;

        // Add to existing delegation or create new
        if let Some(existing) = self.delegations.get_mut(&delegator) {
            existing.amount += amount;
        } else {
            self.delegations.insert(delegator, delegation);
        }

        self.total_delegated += amount;

        Ok(())
    }

    /// Request undelegation
    pub fn request_undelegation(
        &mut self,
        delegator: &SilverAddress,
        amount: u64,
    ) -> Result<UndelegationRequest> {
        let delegation = self.delegations
            .get_mut(delegator)
            .ok_or_else(|| Error::InvalidData(format!(
                "No delegation found for delegator {}",
                delegator
            )))?;

        if amount > delegation.amount {
            return Err(Error::InvalidData(format!(
                "Cannot undelegate {} SBTC, only {} delegated",
                amount, delegation.amount
            )));
        }

        let rewards = if amount == delegation.amount {
            // Full undelegation - include all rewards
            delegation.claim_rewards()
        } else {
            // Partial undelegation - proportional rewards
            let reward_share = (delegation.accumulated_rewards as f64 * amount as f64 / delegation.amount as f64) as u64;
            delegation.accumulated_rewards -= reward_share;
            reward_share
        };

        delegation.amount -= amount;
        self.total_delegated -= amount;

        // Remove delegation if fully undelegated
        if delegation.amount == 0 {
            self.delegations.remove(delegator);
        }

        let request = UndelegationRequest::new(
            *delegator,
            self.validator_id.clone(),
            amount,
            rewards,
        );

        self.undelegation_requests.push(request.clone());

        Ok(request)
    }

    /// Redelegate to another validator (instant, no unbonding)
    pub fn redelegate(
        &mut self,
        delegator: &SilverAddress,
        amount: u64,
    ) -> Result<(u64, u64)> {
        let delegation = self.delegations
            .get_mut(delegator)
            .ok_or_else(|| Error::InvalidData(format!(
                "No delegation found for delegator {}",
                delegator
            )))?;

        if amount > delegation.amount {
            return Err(Error::InvalidData(format!(
                "Cannot redelegate {} SBTC, only {} delegated",
                amount, delegation.amount
            )));
        }

        let rewards = if amount == delegation.amount {
            // Full redelegation - include all rewards
            delegation.claim_rewards()
        } else {
            // Partial redelegation - proportional rewards
            let reward_share = (delegation.accumulated_rewards as f64 * amount as f64 / delegation.amount as f64) as u64;
            delegation.accumulated_rewards -= reward_share;
            reward_share
        };

        delegation.amount -= amount;
        self.total_delegated -= amount;

        // Remove delegation if fully redelegated
        if delegation.amount == 0 {
            self.delegations.remove(delegator);
        }

        Ok((amount, rewards))
    }

    /// Process completed undelegation requests
    pub fn process_undelegations(&mut self) -> Vec<UndelegationRequest> {
        let mut completed = Vec::new();

        for request in &mut self.undelegation_requests {
            if !request.completed && request.is_unbonded() {
                request.completed = true;
                completed.push(request.clone());
            }
        }

        // Remove completed requests
        self.undelegation_requests.retain(|r| !r.completed);

        completed
    }

    /// Distribute rewards to delegators
    pub fn distribute_rewards(&mut self, total_rewards: u64) {
        if self.total_delegated == 0 {
            return;
        }

        for delegation in self.delegations.values_mut() {
            let share = (total_rewards as f64 * delegation.amount as f64 / self.total_delegated as f64) as u64;
            delegation.add_rewards(share);
        }
    }

    /// Get delegation for delegator
    pub fn get_delegation(&self, delegator: &SilverAddress) -> Option<&Delegation> {
        self.delegations.get(delegator)
    }

    /// Get delegator count
    pub fn delegator_count(&self) -> usize {
        self.delegations.len()
    }
}

/// Delegation manager
///
/// Manages all delegations across validators
pub struct DelegationManager {
    /// Validator delegation info indexed by validator ID
    validator_delegations: HashMap<ValidatorID, ValidatorDelegationInfo>,
    
    /// Total delegated stake across all validators
    total_delegated: u64,
}

impl DelegationManager {
    /// Create a new delegation manager
    pub fn new() -> Self {
        Self {
            validator_delegations: HashMap::new(),
            total_delegated: 0,
        }
    }

    /// Delegate stake to a validator
    pub fn delegate(
        &mut self,
        delegator: SilverAddress,
        validator_id: ValidatorID,
        amount: u64,
    ) -> Result<Delegation> {
        if amount < MIN_DELEGATION_AMOUNT {
            return Err(Error::InvalidData(format!(
                "Delegation amount {} is below minimum {}",
                amount, MIN_DELEGATION_AMOUNT
            )));
        }

        let delegation = Delegation::new(delegator, validator_id.clone(), amount)?;

        let validator_info = self.validator_delegations
            .entry(validator_id.clone())
            .or_insert_with(|| ValidatorDelegationInfo::new(validator_id.clone()));

        validator_info.add_delegation(delegation.clone())?;
        self.total_delegated += amount;

        info!(
            "Delegator {} delegated {} SBTC to validator {}",
            delegator,
            amount,
            validator_id
        );

        Ok(delegation)
    }

    /// Undelegate stake from a validator
    pub fn undelegate(
        &mut self,
        delegator: &SilverAddress,
        validator_id: &ValidatorID,
        amount: u64,
    ) -> Result<UndelegationRequest> {
        let validator_info = self.validator_delegations
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!(
                "No delegations found for validator {}",
                validator_id
            )))?;

        let request = validator_info.request_undelegation(delegator, amount)?;
        self.total_delegated -= amount;

        info!(
            "Delegator {} requested undelegation of {} SBTC from validator {} (unbonds at: {})",
            delegator, amount, validator_id, request.unbonds_at
        );

        Ok(request)
    }

    /// Redelegate stake to another validator (instant, no unbonding)
    pub fn redelegate(
        &mut self,
        delegator: &SilverAddress,
        from_validator: &ValidatorID,
        to_validator: ValidatorID,
        amount: u64,
    ) -> Result<Delegation> {
        // Remove from source validator
        let from_info = self.validator_delegations
            .get_mut(from_validator)
            .ok_or_else(|| Error::InvalidData(format!(
                "No delegations found for validator {}",
                from_validator
            )))?;

        let (redelegated_amount, rewards) = from_info.redelegate(delegator, amount)?;

        // Add to target validator
        let total_amount = redelegated_amount + rewards;
        let delegation = Delegation::new(*delegator, to_validator.clone(), total_amount)?;

        let to_info = self.validator_delegations
            .entry(to_validator.clone())
            .or_insert_with(|| ValidatorDelegationInfo::new(to_validator.clone()));

        to_info.add_delegation(delegation.clone())?;

        info!(
            "Delegator {} redelegated {} SBTC (+ {} rewards) from {} to {}",
            delegator, redelegated_amount, rewards, from_validator, to_validator
        );

        Ok(delegation)
    }

    /// Process all undelegation requests
    pub fn process_undelegations(&mut self) -> HashMap<ValidatorID, Vec<UndelegationRequest>> {
        let mut completed_by_validator = HashMap::new();

        for (validator_id, info) in &mut self.validator_delegations {
            let completed = info.process_undelegations();
            
            if !completed.is_empty() {
                info!(
                    "Validator {} completed {} undelegation requests",
                    validator_id,
                    completed.len()
                );
                
                completed_by_validator.insert(validator_id.clone(), completed);
            }
        }

        completed_by_validator
    }

    /// Distribute rewards to delegators of a validator
    pub fn distribute_validator_rewards(
        &mut self,
        validator_id: &ValidatorID,
        total_rewards: u64,
    ) {
        if let Some(info) = self.validator_delegations.get_mut(validator_id) {
            info.distribute_rewards(total_rewards);
            
            info!(
                "Distributed {} SBTC rewards to {} delegators of validator {}",
                total_rewards,
                info.delegator_count(),
                validator_id
            );
        }
    }

    /// Get delegation for delegator and validator
    pub fn get_delegation(
        &self,
        delegator: &SilverAddress,
        validator_id: &ValidatorID,
    ) -> Option<&Delegation> {
        self.validator_delegations
            .get(validator_id)
            .and_then(|info| info.get_delegation(delegator))
    }

    /// Get total delegated stake for validator
    pub fn get_validator_delegated_stake(&self, validator_id: &ValidatorID) -> u64 {
        self.validator_delegations
            .get(validator_id)
            .map(|info| info.total_delegated)
            .unwrap_or(0)
    }

    /// Get total delegated stake across all validators
    pub fn total_delegated(&self) -> u64 {
        self.total_delegated
    }

    /// Set validator active status
    pub fn set_validator_active(&mut self, validator_id: &ValidatorID, active: bool) {
        if let Some(info) = self.validator_delegations.get_mut(validator_id) {
            info.is_active = active;
            
            if !active {
                warn!("Validator {} set to inactive - no new delegations allowed", validator_id);
            }
        }
    }

    /// Set validator jailed status
    pub fn set_validator_jailed(&mut self, validator_id: &ValidatorID, jailed: bool) {
        if let Some(info) = self.validator_delegations.get_mut(validator_id) {
            info.is_jailed = jailed;
            
            if jailed {
                warn!("Validator {} jailed - no new delegations allowed", validator_id);
            }
        }
    }

    /// Get all delegators for a validator
    pub fn get_validator_delegators(&self, validator_id: &ValidatorID) -> Vec<SilverAddress> {
        self.validator_delegations
            .get(validator_id)
            .map(|info| info.delegations.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Get delegator count for validator
    pub fn get_validator_delegator_count(&self, validator_id: &ValidatorID) -> usize {
        self.validator_delegations
            .get(validator_id)
            .map(|info| info.delegator_count())
            .unwrap_or(0)
    }
}

impl Default for DelegationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_address(id: u8) -> SilverAddress {
        SilverAddress::new([id; 64])
    }

    fn create_test_validator_id(id: u8) -> ValidatorID {
        ValidatorID::new(create_test_address(id))
    }

    #[test]
    fn test_delegation_creation() {
        let delegator = create_test_address(1);
        let validator_id = create_test_validator_id(2);
        
        // Below minimum should fail
        let result = Delegation::new(delegator, validator_id.clone(), 9);
        assert!(result.is_err());

        // At minimum should succeed
        let result = Delegation::new(delegator, validator_id, 10);
        assert!(result.is_ok());
    }

    #[test]
    fn test_delegation_rewards() {
        let delegator = create_test_address(1);
        let validator_id = create_test_validator_id(2);
        let mut delegation = Delegation::new(delegator, validator_id, 100).unwrap();

        delegation.add_rewards(50);
        assert_eq!(delegation.accumulated_rewards, 50);
        assert_eq!(delegation.total_value(), 150);

        let claimed = delegation.claim_rewards();
        assert_eq!(claimed, 50);
        assert_eq!(delegation.accumulated_rewards, 0);
    }

    #[test]
    fn test_delegation_manager() {
        let mut manager = DelegationManager::new();
        let delegator = create_test_address(1);
        let validator_id = create_test_validator_id(2);

        // Delegate
        let delegation = manager.delegate(delegator, validator_id.clone(), 1000).unwrap();
        assert_eq!(delegation.amount, 1000);
        assert_eq!(manager.total_delegated(), 1000);
        assert_eq!(manager.get_validator_delegated_stake(&validator_id), 1000);

        // Undelegate
        let request = manager.undelegate(&delegator, &validator_id, 500).unwrap();
        assert_eq!(request.amount, 500);
        assert_eq!(manager.get_validator_delegated_stake(&validator_id), 500);
    }

    #[test]
    fn test_redelegation() {
        let mut manager = DelegationManager::new();
        let delegator = create_test_address(1);
        let validator1 = create_test_validator_id(2);
        let validator2 = create_test_validator_id(3);

        // Initial delegation
        manager.delegate(delegator, validator1.clone(), 1000).unwrap();

        // Redelegate
        let delegation = manager.redelegate(&delegator, &validator1, validator2.clone(), 600).unwrap();
        assert_eq!(delegation.amount, 600);
        assert_eq!(manager.get_validator_delegated_stake(&validator1), 400);
        assert_eq!(manager.get_validator_delegated_stake(&validator2), 600);
    }

    #[test]
    fn test_max_delegation_limit() {
        let mut manager = DelegationManager::new();
        let validator_id = create_test_validator_id(1);

        // Delegate up to max
        manager.delegate(create_test_address(1), validator_id.clone(), MAX_DELEGATED_STAKE_PER_VALIDATOR).unwrap();

        // Exceeding max should fail
        let result = manager.delegate(create_test_address(2), validator_id, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_jailed_validator_delegation() {
        let mut manager = DelegationManager::new();
        let delegator = create_test_address(1);
        let validator_id = create_test_validator_id(2);

        // Initial delegation
        manager.delegate(delegator, validator_id.clone(), 1000).unwrap();

        // Jail validator
        manager.set_validator_jailed(&validator_id, true);

        // New delegation should fail
        let result = manager.delegate(create_test_address(3), validator_id, 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_reward_distribution() {
        let mut manager = DelegationManager::new();
        let validator_id = create_test_validator_id(1);

        // Two delegators
        manager.delegate(create_test_address(2), validator_id.clone(), 600).unwrap();
        manager.delegate(create_test_address(3), validator_id.clone(), 400).unwrap();

        // Distribute 1000 rewards
        manager.distribute_validator_rewards(&validator_id, 1000);

        // Check proportional distribution
        let del1 = manager.get_delegation(&create_test_address(2), &validator_id).unwrap();
        let del2 = manager.get_delegation(&create_test_address(3), &validator_id).unwrap();

        assert_eq!(del1.accumulated_rewards, 600); // 60% of 1000
        assert_eq!(del2.accumulated_rewards, 400); // 40% of 1000
    }
}
