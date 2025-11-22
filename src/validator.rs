//! Validator set management
//!
//! This module manages the validator set for consensus, including:
//! - Validator registration and stake tracking
//! - Stake-weighted voting
//! - Validator set reconfiguration at cycle boundaries
//! - Validator lifecycle management
//! - Participation tracking and penalties

use silver_core::{Error, Result, SilverAddress, ValidatorID, ValidatorMetadata};
use crate::staking::StakingManager;
use crate::delegation::DelegationManager;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, warn};

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

/// Validator set change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSetChangeEvent {
    /// Cycle when change occurred
    pub cycle: u64,
    
    /// Validators added
    pub added: Vec<ValidatorID>,
    
    /// Validators removed
    pub removed: Vec<ValidatorID>,
    
    /// Total validators after change
    pub total_validators: usize,
    
    /// Total stake after change
    pub total_stake: u64,
    
    /// Timestamp of change
    pub timestamp: u64,
}

impl ValidatorSetChangeEvent {
    /// Create new validator set change event
    pub fn new(
        cycle: u64,
        added: Vec<ValidatorID>,
        removed: Vec<ValidatorID>,
        total_validators: usize,
        total_stake: u64,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            cycle,
            added,
            removed,
            total_validators,
            total_stake,
            timestamp,
        }
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
    
    /// Validator set change history
    change_history: Arc<RwLock<Vec<ValidatorSetChangeEvent>>>,
    
    /// Minimum stake required to be a validator
    min_stake: u64,
}

impl ValidatorSet {
    /// Create a new validator set with default minimum stake (10,000 SBTC)
    pub fn new() -> Self {
        Self::with_min_stake(10_000)
    }
    
    /// Create a new validator set with custom minimum stake
    pub fn with_min_stake(min_stake: u64) -> Self {
        Self {
            validators: Arc::new(DashMap::new()),
            total_stake: Arc::new(RwLock::new(0)),
            current_cycle: Arc::new(RwLock::new(0)),
            change_history: Arc::new(RwLock::new(Vec::new())),
            min_stake,
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

    /// Reconfigure validator set at cycle end
    ///
    /// This is the core validator set reconfiguration logic that:
    /// 1. Removes validators below minimum stake
    /// 2. Adds new validators from staking manager
    /// 3. Updates total stake
    /// 4. Records the change event
    ///
    /// # Arguments
    /// * `staking_manager` - Staking manager with current validator stakes
    /// * `delegation_manager` - Delegation manager for delegated stake info
    ///
    /// # Returns
    /// ValidatorSetChangeEvent with details of changes
    pub fn reconfigure_at_cycle_end(
        &mut self,
        staking_manager: &StakingManager,
        delegation_manager: &DelegationManager,
    ) -> Result<ValidatorSetChangeEvent> {
        let cycle = *self.current_cycle.read();
        
        debug!("Starting validator set reconfiguration for cycle {}", cycle);

        // Step 1: Get all validators with active stake from staking manager
        let staked_validators = staking_manager.get_staked_validators();
        
        if staked_validators.is_empty() {
            return Err(Error::InvalidData(
                "No validators with active stake found".to_string()
            ));
        }

        debug!(
            "Found {} validators with active stake",
            staked_validators.len()
        );

        // Step 2: Identify validators to remove (below minimum stake or not in staking manager)
        let mut validators_to_remove = Vec::new();
        
        for entry in self.validators.iter() {
            let validator_id = entry.key();
            
            // Check if validator still has minimum stake
            if !staking_manager.meets_minimum_stake(validator_id) {
                validators_to_remove.push(validator_id.clone());
                debug!(
                    "Validator {} below minimum stake, marking for removal",
                    validator_id
                );
            }
        }

        // Step 3: Remove validators below minimum stake
        let mut removed_count = 0;
        let mut total_removed_stake = 0u64;
        
        for validator_id in &validators_to_remove {
            if let Some((_, info)) = self.validators.remove(validator_id) {
                total_removed_stake += info.stake;
                removed_count += 1;
                
                info!(
                    "Removed validator {} (stake: {} SBTC) - below minimum",
                    validator_id,
                    info.stake
                );
            }
        }

        // Step 4: Add new validators from staking manager
        let mut added_count = 0;
        let mut total_added_stake = 0u64;
        let mut added_validators = Vec::new();
        
        for validator_id in &staked_validators {
            if !self.validators.contains_key(validator_id) {
                // New validator - add to set
                let active_stake = staking_manager.get_active_stake(validator_id);
                let delegated_stake = delegation_manager.get_validator_delegated_stake(validator_id);
                let total_stake = active_stake + delegated_stake;
                
                // Create validator info with updated stake
                if let Some(existing_info) = self.validators.get(validator_id) {
                    let mut new_info = existing_info.clone();
                    new_info.stake = total_stake;
                    self.validators.insert(validator_id.clone(), new_info);
                } else {
                    // This shouldn't happen for new validators, but handle gracefully
                    debug!(
                        "New validator {} with stake {} SBTC",
                        validator_id,
                        total_stake
                    );
                }
                
                total_added_stake += total_stake;
                added_count += 1;
                added_validators.push(validator_id.clone());
                
                info!(
                    "Added validator {} (active: {}, delegated: {}, total: {} SBTC)",
                    validator_id,
                    active_stake,
                    delegated_stake,
                    total_stake
                );
            }
        }

        // Step 5: Update stake amounts for existing validators
        let mut total_stake = 0u64;
        
        for mut entry in self.validators.iter_mut() {
            let validator_id = entry.key();
            let active_stake = staking_manager.get_active_stake(validator_id);
            let delegated_stake = delegation_manager.get_validator_delegated_stake(validator_id);
            let new_total_stake = active_stake + delegated_stake;
            
            entry.value_mut().stake = new_total_stake;
            total_stake += new_total_stake;
            
            debug!(
                "Updated validator {} stake: active={}, delegated={}, total={}",
                validator_id,
                active_stake,
                delegated_stake,
                new_total_stake
            );
        }

        // Step 6: Update total stake
        *self.total_stake.write() = total_stake;

        // Step 7: Create and record change event
        let event = ValidatorSetChangeEvent::new(
            cycle,
            added_validators.clone(),
            validators_to_remove.clone(),
            self.validators.len(),
            total_stake,
        );

        self.change_history.write().push(event.clone());

        // Step 8: Log summary
        info!(
            "Validator set reconfiguration complete for cycle {}:",
            cycle
        );
        info!(
            "  Added: {} validators (+{} SBTC)",
            added_count,
            total_added_stake
        );
        info!(
            "  Removed: {} validators (-{} SBTC)",
            removed_count,
            total_removed_stake
        );
        info!(
            "  Total validators: {}",
            self.validators.len()
        );
        info!(
            "  Total stake: {} SBTC",
            total_stake
        );

        Ok(event)
    }

    /// Get validator set change history
    pub fn get_change_history(&self) -> Vec<ValidatorSetChangeEvent> {
        self.change_history.read().clone()
    }

    /// Get changes for specific cycle
    pub fn get_cycle_changes(&self, cycle: u64) -> Vec<ValidatorSetChangeEvent> {
        self.change_history
            .read()
            .iter()
            .filter(|event| event.cycle == cycle)
            .cloned()
            .collect()
    }

    /// Get minimum stake requirement
    pub fn min_stake(&self) -> u64 {
        self.min_stake
    }

    /// Set minimum stake requirement
    pub fn set_min_stake(&mut self, min_stake: u64) {
        self.min_stake = min_stake;
        info!("Updated minimum stake requirement to {} SBTC", min_stake);
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
    use crate::staking::StakingManager;
    use crate::delegation::DelegationManager;

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

    fn create_test_validator_id(id: u8) -> ValidatorID {
        create_test_validator(id, 50_000).id()
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
        assert!(set.has_quorum(&[val1.clone(), val2]));

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

    #[test]
    fn test_reconfiguration_add_validators() {
        let mut validator_set = ValidatorSet::with_min_stake(10_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        // Add validators to staking manager
        let val1_id = create_test_validator_id(1);
        let val2_id = create_test_validator_id(2);
        let val3_id = create_test_validator_id(3);

        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![1u8; 64]).unwrap();
        staking_manager.deposit_stake(val2_id.clone(), 100_000, vec![2u8; 64]).unwrap();
        staking_manager.deposit_stake(val3_id.clone(), 150_000, vec![3u8; 64]).unwrap();

        // Reconfigure
        let event = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();

        // Verify changes
        assert_eq!(event.added.len(), 3);
        assert_eq!(event.removed.len(), 0);
        assert_eq!(event.total_validators, 3);
        assert_eq!(event.total_stake, 300_000);
        assert_eq!(validator_set.validator_count(), 3);
    }

    #[test]
    fn test_reconfiguration_remove_validators() {
        let mut validator_set = ValidatorSet::with_min_stake(10_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        // Add initial validators
        let val1_id = create_test_validator_id(1);
        let val2_id = create_test_validator_id(2);
        let val3_id = create_test_validator_id(3);

        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![1u8; 64]).unwrap();
        staking_manager.deposit_stake(val2_id.clone(), 100_000, vec![2u8; 64]).unwrap();
        staking_manager.deposit_stake(val3_id.clone(), 150_000, vec![3u8; 64]).unwrap();

        // First reconfiguration
        validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        assert_eq!(validator_set.validator_count(), 3);

        // Unstake one validator below minimum
        staking_manager.request_unstake(&val2_id, 95_000).unwrap();

        // Second reconfiguration - should remove val2
        let event = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();

        assert_eq!(event.removed.len(), 1);
        assert_eq!(event.removed[0], val2_id);
        assert_eq!(validator_set.validator_count(), 2);
        assert_eq!(validator_set.total_stake(), 200_000);
    }

    #[test]
    fn test_reconfiguration_stake_updates() {
        let mut validator_set = ValidatorSet::with_min_stake(10_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        let val1_id = create_test_validator_id(1);

        // Initial stake
        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![1u8; 64]).unwrap();
        validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();

        let info = validator_set.get_validator(&val1_id).unwrap();
        assert_eq!(info.stake, 50_000);

        // Add more stake
        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![2u8; 64]).unwrap();
        validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();

        let info = validator_set.get_validator(&val1_id).unwrap();
        assert_eq!(info.stake, 100_000);
        assert_eq!(validator_set.total_stake(), 100_000);
    }

    #[test]
    fn test_reconfiguration_change_history() {
        let mut validator_set = ValidatorSet::with_min_stake(10_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        let val1_id = create_test_validator_id(1);
        let val2_id = create_test_validator_id(2);

        // First cycle
        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![1u8; 64]).unwrap();
        let event1 = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        assert_eq!(event1.cycle, 0);

        // Advance cycle
        validator_set.advance_cycle();

        // Second cycle
        staking_manager.deposit_stake(val2_id.clone(), 100_000, vec![2u8; 64]).unwrap();
        let event2 = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        assert_eq!(event2.cycle, 1);

        // Check history
        let history = validator_set.get_change_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].cycle, 0);
        assert_eq!(history[1].cycle, 1);

        // Check cycle-specific changes
        let cycle0_changes = validator_set.get_cycle_changes(0);
        assert_eq!(cycle0_changes.len(), 1);
        assert_eq!(cycle0_changes[0].added.len(), 1);
    }

    #[test]
    fn test_reconfiguration_multiple_cycles() {
        let mut validator_set = ValidatorSet::with_min_stake(10_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        // Cycle 0: Add 3 validators
        for i in 1..=3 {
            let val_id = create_test_validator_id(i);
            staking_manager.deposit_stake(val_id, 50_000 * i as u64, vec![i; 64]).unwrap();
        }
        let event0 = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        assert_eq!(event0.added.len(), 3);
        assert_eq!(event0.total_validators, 3);

        // Cycle 1: Add 1 more, remove 1
        validator_set.advance_cycle();
        let val4_id = create_test_validator_id(4);
        staking_manager.deposit_stake(val4_id, 200_000, vec![4; 64]).unwrap();
        
        let val1_id = create_test_validator_id(1);
        staking_manager.request_unstake(&val1_id, 45_000).unwrap();

        let event1 = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        assert_eq!(event1.added.len(), 1);
        assert_eq!(event1.removed.len(), 1);
        assert_eq!(event1.total_validators, 3);

        // Verify history
        let history = validator_set.get_change_history();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_reconfiguration_empty_staking_manager() {
        let mut validator_set = ValidatorSet::new();
        let staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        // Should fail with no validators
        let result = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager);
        assert!(result.is_err());
    }

    #[test]
    fn test_min_stake_requirement() {
        let mut validator_set = ValidatorSet::with_min_stake(50_000);
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        let val1_id = create_test_validator_id(1);
        
        // Stake below custom minimum
        staking_manager.deposit_stake(val1_id.clone(), 30_000, vec![1u8; 64]).unwrap();
        
        // Should fail - below minimum
        let result = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager);
        assert!(result.is_err());

        // Stake at minimum
        staking_manager.deposit_stake(val1_id.clone(), 20_000, vec![2u8; 64]).unwrap();
        
        // Should succeed now
        let result = validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reconfiguration_preserves_participation() {
        let mut validator_set = ValidatorSet::new();
        let mut staking_manager = StakingManager::new();
        let delegation_manager = DelegationManager::new();

        let val1_id = create_test_validator_id(1);
        staking_manager.deposit_stake(val1_id.clone(), 50_000, vec![1u8; 64]).unwrap();

        // Add validator
        validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();

        // Record participation
        validator_set.record_participation(&val1_id, true);
        validator_set.record_participation(&val1_id, true);
        validator_set.record_participation(&val1_id, false);

        let info = validator_set.get_validator(&val1_id).unwrap();
        assert_eq!(info.snapshots_participated, 2);
        assert_eq!(info.total_snapshots, 3);

        // Reconfigure again - participation should be preserved
        validator_set.reconfigure_at_cycle_end(&staking_manager, &delegation_manager).unwrap();
        
        let info = validator_set.get_validator(&val1_id).unwrap();
        assert_eq!(info.snapshots_participated, 2);
        assert_eq!(info.total_snapshots, 3);
    }
}

