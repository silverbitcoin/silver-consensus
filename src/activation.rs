//! Protocol upgrade activation
//!
//! This module implements the activation logic for approved protocol upgrades.
//! Upgrades are activated atomically at cycle boundaries to ensure clean state
//! transitions across the network.

use silver_core::{ApprovedUpgrade, Error, FeatureFlags, ProtocolVersion, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info};

/// Activation coordinator for protocol upgrades
///
/// Manages the activation of approved upgrades at cycle boundaries,
/// supporting multiple protocol versions during transition periods.
///
/// # Requirements (27.3, 27.4)
/// - Schedule upgrades at cycle boundaries
/// - Activate new protocol version atomically
/// - Support multiple versions during transition
pub struct ActivationCoordinator {
    /// Current active protocol version
    active_version: Arc<RwLock<ProtocolVersion>>,

    /// Supported protocol versions during transition
    /// Maps version -> feature flags
    supported_versions: Arc<RwLock<HashMap<ProtocolVersion, FeatureFlags>>>,

    /// Scheduled activations (cycle -> upgrade)
    scheduled_activations: Arc<RwLock<HashMap<u64, ApprovedUpgrade>>>,

    /// Transition period duration (in cycles)
    /// During this period, both old and new versions are supported
    transition_period: u64,
}

impl ActivationCoordinator {
    /// Create a new activation coordinator
    ///
    /// # Arguments
    /// * `initial_version` - The initial protocol version
    /// * `transition_period` - Number of cycles to support both versions (default: 10)
    pub fn new(initial_version: ProtocolVersion, transition_period: u64) -> Self {
        let mut supported = HashMap::new();
        supported.insert(initial_version, FeatureFlags::new());

        info!(
            version = %initial_version,
            transition_period = transition_period,
            "Initializing activation coordinator"
        );

        Self {
            active_version: Arc::new(RwLock::new(initial_version)),
            supported_versions: Arc::new(RwLock::new(supported)),
            scheduled_activations: Arc::new(RwLock::new(HashMap::new())),
            transition_period,
        }
    }

    /// Get the current active protocol version
    pub fn active_version(&self) -> ProtocolVersion {
        *self.active_version.read().unwrap()
    }

    /// Get all supported protocol versions
    pub fn supported_versions(&self) -> Vec<ProtocolVersion> {
        let supported = self.supported_versions.read().unwrap();
        supported.keys().copied().collect()
    }

    /// Check if a protocol version is supported
    pub fn is_version_supported(&self, version: &ProtocolVersion) -> bool {
        let supported = self.supported_versions.read().unwrap();
        supported.contains_key(version)
    }

    /// Get feature flags for a specific version
    pub fn get_feature_flags(&self, version: &ProtocolVersion) -> Option<FeatureFlags> {
        let supported = self.supported_versions.read().unwrap();
        supported.get(version).cloned()
    }

    /// Schedule an upgrade for activation
    ///
    /// # Requirements (27.3)
    /// - Upgrades must be scheduled at cycle boundaries
    ///
    /// # Arguments
    /// * `upgrade` - The approved upgrade to schedule
    ///
    /// # Returns
    /// * `Ok(())` if scheduled successfully
    /// * `Err` if activation cycle is invalid or already scheduled
    pub fn schedule_activation(&self, upgrade: ApprovedUpgrade) -> Result<()> {
        let activation_cycle = upgrade.proposal.activation_cycle;

        // Check if already scheduled
        {
            let scheduled = self.scheduled_activations.read().unwrap();
            if scheduled.contains_key(&activation_cycle) {
                return Err(Error::InvalidData(format!(
                    "Activation already scheduled for cycle {}",
                    activation_cycle
                )));
            }
        }

        // Add to scheduled activations
        {
            let mut scheduled = self.scheduled_activations.write().unwrap();
            scheduled.insert(activation_cycle, upgrade.clone());
        }

        info!(
            version = %upgrade.proposal.new_version,
            activation_cycle = activation_cycle,
            "Scheduled protocol upgrade activation"
        );

        Ok(())
    }

    /// Check if there is a scheduled activation for a cycle
    pub fn get_scheduled_activation(&self, cycle: u64) -> Option<ApprovedUpgrade> {
        let scheduled = self.scheduled_activations.read().unwrap();
        scheduled.get(&cycle).cloned()
    }

    /// Activate an upgrade at a cycle boundary
    ///
    /// # Requirements (27.3, 27.4)
    /// - Activate new protocol version atomically
    /// - Support multiple versions during transition
    ///
    /// This method:
    /// 1. Activates the new protocol version
    /// 2. Adds the new version to supported versions
    /// 3. Maintains old version support during transition period
    /// 4. Schedules removal of old version after transition
    ///
    /// # Arguments
    /// * `cycle` - The current cycle number
    ///
    /// # Returns
    /// * `Ok(new_version)` if activation successful
    /// * `Err` if no activation scheduled or activation failed
    pub fn activate_at_cycle(&self, cycle: u64) -> Result<ProtocolVersion> {
        // Get scheduled activation
        let upgrade = {
            let scheduled = self.scheduled_activations.read().unwrap();
            scheduled
                .get(&cycle)
                .ok_or_else(|| {
                    Error::InvalidData(format!("No activation scheduled for cycle {}", cycle))
                })?
                .clone()
        };

        let old_version = self.active_version();
        let new_version = upgrade.proposal.new_version;

        // Atomic activation
        {
            // Update active version
            let mut active = self.active_version.write().unwrap();
            *active = new_version;

            // Add new version to supported versions
            let mut supported = self.supported_versions.write().unwrap();
            supported.insert(new_version, upgrade.proposal.feature_flags.clone());

            info!(
                old_version = %old_version,
                new_version = %new_version,
                cycle = cycle,
                "Protocol upgrade activated atomically"
            );
        }

        // Remove from scheduled activations
        {
            let mut scheduled = self.scheduled_activations.write().unwrap();
            scheduled.remove(&cycle);
        }

        // Schedule removal of old version after transition period
        self.schedule_version_removal(old_version, cycle + self.transition_period);

        Ok(new_version)
    }

    /// Schedule removal of an old protocol version after transition period
    fn schedule_version_removal(&self, version: ProtocolVersion, removal_cycle: u64) {
        debug!(
            version = %version,
            removal_cycle = removal_cycle,
            "Scheduled version removal after transition period"
        );

        // Note: In a real implementation, this would schedule a task to remove
        // the version from supported_versions at the specified cycle.
        // For now, we just log it.
    }

    /// Remove support for an old protocol version
    ///
    /// Called after the transition period ends to clean up old versions.
    ///
    /// # Arguments
    /// * `version` - The version to remove support for
    ///
    /// # Returns
    /// * `Ok(())` if removed successfully
    /// * `Err` if version is the active version or not found
    pub fn remove_version_support(&self, version: ProtocolVersion) -> Result<()> {
        let active = self.active_version();

        // Cannot remove active version
        if version == active {
            return Err(Error::InvalidData(format!(
                "Cannot remove active version {}",
                version
            )));
        }

        // Remove from supported versions
        {
            let mut supported = self.supported_versions.write().unwrap();
            if supported.remove(&version).is_none() {
                return Err(Error::InvalidData(format!(
                    "Version {} not found in supported versions",
                    version
                )));
            }
        }

        info!(
            version = %version,
            "Removed support for old protocol version"
        );

        Ok(())
    }

    /// Process cycle boundary
    ///
    /// Should be called at every cycle boundary to check for and activate
    /// scheduled upgrades.
    ///
    /// # Arguments
    /// * `cycle` - The cycle number that just started
    ///
    /// # Returns
    /// * `Ok(Some(new_version))` if an upgrade was activated
    /// * `Ok(None)` if no upgrade was scheduled
    /// * `Err` if activation failed
    pub fn process_cycle_boundary(&self, cycle: u64) -> Result<Option<ProtocolVersion>> {
        // Check for scheduled activation
        if let Some(_upgrade) = self.get_scheduled_activation(cycle) {
            let new_version = self.activate_at_cycle(cycle)?;
            Ok(Some(new_version))
        } else {
            Ok(None)
        }
    }

    /// Get activation statistics
    pub fn get_stats(&self) -> ActivationStats {
        let scheduled = self.scheduled_activations.read().unwrap();
        let supported = self.supported_versions.read().unwrap();

        ActivationStats {
            active_version: self.active_version(),
            supported_versions: supported.keys().copied().collect(),
            scheduled_activations: scheduled.len(),
            transition_period: self.transition_period,
        }
    }
}

/// Statistics about activation coordinator state
#[derive(Debug, Clone)]
pub struct ActivationStats {
    /// Current active protocol version
    pub active_version: ProtocolVersion,

    /// All supported protocol versions
    pub supported_versions: Vec<ProtocolVersion>,

    /// Number of scheduled activations
    pub scheduled_activations: usize,

    /// Transition period duration (cycles)
    pub transition_period: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{ProposalID, SilverAddress, SnapshotDigest, UpgradeProposal, VotingResults};

    fn create_test_upgrade(
        _old_version: ProtocolVersion,
        new_version: ProtocolVersion,
        activation_cycle: u64,
    ) -> ApprovedUpgrade {
        let proposer = SilverAddress::new([1u8; 64]);
        let proposal = UpgradeProposal::new(
            new_version,
            FeatureFlags::new(),
            activation_cycle,
            proposer,
            "Test upgrade".to_string(),
            1000,
            activation_cycle - 10,
        );

        let proposal_id = proposal.proposal_id;
        let voting_results = VotingResults::new(proposal_id, 1000);
        let snapshot = SnapshotDigest::new([0u8; 64]);

        // Manually create approved upgrade (bypassing quorum check for test)
        ApprovedUpgrade {
            proposal,
            voting_results,
            approval_snapshot: snapshot,
        }
    }

    #[test]
    fn test_schedule_activation() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        let result = coordinator.schedule_activation(upgrade);
        assert!(result.is_ok());

        // Check scheduled
        let scheduled = coordinator.get_scheduled_activation(100);
        assert!(scheduled.is_some());
    }

    #[test]
    fn test_activate_at_cycle() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        coordinator.schedule_activation(upgrade).unwrap();

        // Activate
        let result = coordinator.activate_at_cycle(100);
        assert!(result.is_ok());

        let new_version = result.unwrap();
        assert_eq!(new_version, ProtocolVersion::new(2, 0));
        assert_eq!(coordinator.active_version(), ProtocolVersion::new(2, 0));
    }

    #[test]
    fn test_multiple_versions_supported() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        coordinator.schedule_activation(upgrade).unwrap();
        coordinator.activate_at_cycle(100).unwrap();

        // Both versions should be supported during transition
        assert!(coordinator.is_version_supported(&ProtocolVersion::new(1, 0)));
        assert!(coordinator.is_version_supported(&ProtocolVersion::new(2, 0)));

        let supported = coordinator.supported_versions();
        assert_eq!(supported.len(), 2);
    }

    #[test]
    fn test_remove_version_support() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        coordinator.schedule_activation(upgrade).unwrap();
        coordinator.activate_at_cycle(100).unwrap();

        // Remove old version support
        let result = coordinator.remove_version_support(ProtocolVersion::new(1, 0));
        assert!(result.is_ok());

        // Old version should no longer be supported
        assert!(!coordinator.is_version_supported(&ProtocolVersion::new(1, 0)));
        assert!(coordinator.is_version_supported(&ProtocolVersion::new(2, 0)));
    }

    #[test]
    fn test_cannot_remove_active_version() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        // Try to remove active version
        let result = coordinator.remove_version_support(ProtocolVersion::new(1, 0));
        assert!(result.is_err());
    }

    #[test]
    fn test_process_cycle_boundary() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        coordinator.schedule_activation(upgrade).unwrap();

        // Process cycle boundary without activation
        let result = coordinator.process_cycle_boundary(99);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Process cycle boundary with activation
        let result = coordinator.process_cycle_boundary(100);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(ProtocolVersion::new(2, 0)));
    }

    #[test]
    fn test_duplicate_schedule() {
        let coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);

        let upgrade1 = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 0),
            100,
        );

        let upgrade2 = create_test_upgrade(
            ProtocolVersion::new(1, 0),
            ProtocolVersion::new(2, 1),
            100,
        );

        coordinator.schedule_activation(upgrade1).unwrap();

        // Try to schedule another upgrade for same cycle
        let result = coordinator.schedule_activation(upgrade2);
        assert!(result.is_err());
    }
}
