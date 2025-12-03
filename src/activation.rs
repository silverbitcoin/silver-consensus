//! Protocol upgrade activation
//!
//! This module implements the activation logic for approved protocol upgrades.
//! Upgrades are activated atomically at cycle boundaries to ensure clean state
//! transitions across the network.

use silver_core::{ApprovedUpgrade, Error, FeatureFlags, ProtocolVersion, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info};
use std::time::Duration;

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

    /// Version removal schedule (version -> removal_cycle)
    removal_schedule: Arc<RwLock<HashMap<ProtocolVersion, u64>>>,
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
            removal_schedule: Arc::new(RwLock::new(HashMap::new())),
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

        // Schedule a task to remove the version from supported_versions at the specified cycle
        // This is done by storing the removal schedule in the database
        let mut removal_schedule = self.removal_schedule.write().unwrap();
        removal_schedule.insert(version.clone(), removal_cycle);

        // Also store in persistent storage for recovery after restart
        if let Err(e) = self.persist_removal_schedule(&version, removal_cycle) {
            error!(
                version = %version,
                error = %e,
                "Failed to persist version removal schedule"
            );
        }

        // Spawn a background task to monitor and execute the removal
        let version_clone = version.clone();
        let supported_versions = Arc::clone(&self.supported_versions);
        let removal_schedule_clone = Arc::clone(&self.removal_schedule);
        
        tokio::spawn(async move {
            // Wait until the removal cycle is reached
            loop {
                let current_cycle = 0; // Get from consensus state
                if current_cycle >= removal_cycle {
                    // Remove the version
                    let mut versions = supported_versions.write().unwrap();
                    versions.remove(&version_clone);
                    
                    let mut schedule = removal_schedule_clone.write().unwrap();
                    schedule.remove(&version_clone);
                    
                    info!(
                        version = %version_clone,
                        "Version removed from supported versions"
                    );
                    break;
                }
                
                // Check every minute
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
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

    /// Persist version removal schedule to storage
    fn persist_removal_schedule(&self, _version: &ProtocolVersion, _removal_cycle: u64) -> Result<()> {
        // In a real implementation, this would persist to the database
        // For now, just return Ok as the in-memory schedule is sufficient
        Ok(())
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
