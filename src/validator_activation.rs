//! Validator activation and lifecycle management
//!
//! This module implements production-ready validator activation:
//! - Activation requirements and validation
//! - Deactivation process with cooldown
//! - Activation voting mechanism
//! - Status tracking and transitions
//! - Comprehensive audit trail
//! - Full recovery support

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Validator activation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationConfig {
    /// Minimum stake required for activation (SBTC)
    pub min_stake_for_activation: u64,

    /// Minimum voting power required for activation
    pub min_voting_power: u64,

    /// Deactivation cooldown period (seconds)
    pub deactivation_cooldown_secs: u64,

    /// Reactivation cooldown period (seconds)
    pub reactivation_cooldown_secs: u64,

    /// Votes required for activation (percentage of total stake)
    pub activation_votes_required: f64,

    /// Votes required for deactivation (percentage of total stake)
    pub deactivation_votes_required: f64,
}

impl Default for ActivationConfig {
    fn default() -> Self {
        Self {
            min_stake_for_activation: 10_000,   // 10,000 SBTC
            min_voting_power: 5_000,            // 5,000 voting power
            deactivation_cooldown_secs: 86400,  // 24 hours
            reactivation_cooldown_secs: 604800, // 7 days
            activation_votes_required: 0.66,    // 66% (2/3)
            deactivation_votes_required: 0.66,  // 66% (2/3)
        }
    }
}

/// Validator status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidatorStatus {
    /// Pending activation (not yet active)
    Pending,

    /// Active validator
    Active,

    /// Deactivated validator
    Deactivated,

    /// Suspended (due to violations)
    Suspended,

    /// Removed from validator set
    Removed,
}

impl std::fmt::Display for ValidatorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidatorStatus::Pending => write!(f, "Pending"),
            ValidatorStatus::Active => write!(f, "Active"),
            ValidatorStatus::Deactivated => write!(f, "Deactivated"),
            ValidatorStatus::Suspended => write!(f, "Suspended"),
            ValidatorStatus::Removed => write!(f, "Removed"),
        }
    }
}

/// Activation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationRequest {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Request type
    pub request_type: ActivationRequestType,

    /// Requested at
    pub requested_at: u64,

    /// Votes in favor
    pub votes_in_favor: u64,

    /// Total votes
    pub total_votes: u64,

    /// Status
    pub status: ActivationRequestStatus,
}

/// Activation request type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationRequestType {
    /// Activation request
    Activation,

    /// Deactivation request
    Deactivation,
}

impl std::fmt::Display for ActivationRequestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivationRequestType::Activation => write!(f, "Activation"),
            ActivationRequestType::Deactivation => write!(f, "Deactivation"),
        }
    }
}

/// Activation request status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationRequestStatus {
    /// Pending voting
    Pending,

    /// Approved
    Approved,

    /// Rejected
    Rejected,

    /// Expired
    Expired,
}

impl std::fmt::Display for ActivationRequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivationRequestStatus::Pending => write!(f, "Pending"),
            ActivationRequestStatus::Approved => write!(f, "Approved"),
            ActivationRequestStatus::Rejected => write!(f, "Rejected"),
            ActivationRequestStatus::Expired => write!(f, "Expired"),
        }
    }
}

/// Validator activation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorActivationRecord {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Current status
    pub status: ValidatorStatus,

    /// Activated at
    pub activated_at: Option<u64>,

    /// Deactivated at
    pub deactivated_at: Option<u64>,

    /// Can reactivate at
    pub can_reactivate_at: Option<u64>,

    /// Activation count
    pub activation_count: u64,

    /// Deactivation count
    pub deactivation_count: u64,
}

impl ValidatorActivationRecord {
    /// Create new activation record
    pub fn new(validator_id: ValidatorID) -> Self {
        Self {
            validator_id,
            status: ValidatorStatus::Pending,
            activated_at: None,
            deactivated_at: None,
            can_reactivate_at: None,
            activation_count: 0,
            deactivation_count: 0,
        }
    }

    /// Activate validator
    pub fn activate(&mut self) -> Result<()> {
        if self.status == ValidatorStatus::Active {
            return Err(Error::InvalidData(format!(
                "Validator {} is already active",
                self.validator_id
            )));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Check reactivation cooldown
        if let Some(reactivate_at) = self.can_reactivate_at {
            if now < reactivate_at {
                return Err(Error::InvalidData(format!(
                    "Validator {} cannot reactivate until {}",
                    self.validator_id, reactivate_at
                )));
            }
        }

        self.status = ValidatorStatus::Active;
        self.activated_at = Some(now);
        self.can_reactivate_at = None;
        self.activation_count += 1;

        info!(
            "Activated validator {} (activation count: {})",
            self.validator_id, self.activation_count
        );

        Ok(())
    }

    /// Deactivate validator
    pub fn deactivate(&mut self, cooldown_secs: u64) -> Result<()> {
        if self.status != ValidatorStatus::Active {
            return Err(Error::InvalidData(format!(
                "Validator {} is not active (status: {})",
                self.validator_id, self.status
            )));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.status = ValidatorStatus::Deactivated;
        self.deactivated_at = Some(now);
        self.can_reactivate_at = Some(now + cooldown_secs);
        self.deactivation_count += 1;

        info!(
            "Deactivated validator {} (can reactivate at: {}, deactivation count: {})",
            self.validator_id,
            self.can_reactivate_at.unwrap(),
            self.deactivation_count
        );

        Ok(())
    }

    /// Suspend validator
    pub fn suspend(&mut self) -> Result<()> {
        if self.status == ValidatorStatus::Suspended {
            return Err(Error::InvalidData(format!(
                "Validator {} is already suspended",
                self.validator_id
            )));
        }

        self.status = ValidatorStatus::Suspended;

        warn!("Suspended validator {}", self.validator_id);

        Ok(())
    }

    /// Remove validator
    pub fn remove(&mut self) -> Result<()> {
        self.status = ValidatorStatus::Removed;

        info!("Removed validator {}", self.validator_id);

        Ok(())
    }

    /// Check if can reactivate
    pub fn can_reactivate(&self) -> bool {
        if let Some(reactivate_at) = self.can_reactivate_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now >= reactivate_at
        } else {
            false
        }
    }

    /// Get remaining cooldown time
    pub fn remaining_cooldown(&self) -> u64 {
        if let Some(reactivate_at) = self.can_reactivate_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now >= reactivate_at {
                0
            } else {
                reactivate_at - now
            }
        } else {
            0
        }
    }
}

/// Activation event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationEvent {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Event type
    pub event_type: ActivationEventType,

    /// Timestamp
    pub timestamp: u64,

    /// Cycle
    pub cycle: u64,
}

/// Activation event type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationEventType {
    /// Validator activated
    Activated,

    /// Validator deactivated
    Deactivated,

    /// Validator suspended
    Suspended,

    /// Validator removed
    Removed,
}

impl std::fmt::Display for ActivationEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivationEventType::Activated => write!(f, "Activated"),
            ActivationEventType::Deactivated => write!(f, "Deactivated"),
            ActivationEventType::Suspended => write!(f, "Suspended"),
            ActivationEventType::Removed => write!(f, "Removed"),
        }
    }
}

/// Validator activation manager
pub struct ValidatorActivationManager {
    /// Configuration
    config: ActivationConfig,

    /// Activation records
    records: HashMap<ValidatorID, ValidatorActivationRecord>,

    /// Pending activation requests
    pending_requests: HashMap<ValidatorID, ActivationRequest>,

    /// Activation events
    events: Vec<ActivationEvent>,

    /// Current cycle
    current_cycle: u64,
}

impl ValidatorActivationManager {
    /// Create new activation manager
    pub fn new(config: ActivationConfig) -> Self {
        Self {
            config,
            records: HashMap::new(),
            pending_requests: HashMap::new(),
            events: Vec::new(),
            current_cycle: 0,
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(ActivationConfig::default())
    }

    /// Request activation
    pub fn request_activation(
        &mut self,
        validator_id: ValidatorID,
        stake: u64,
        voting_power: u64,
    ) -> Result<ActivationRequest> {
        // Check minimum requirements
        if stake < self.config.min_stake_for_activation {
            return Err(Error::InvalidData(format!(
                "Validator {} stake {} is below minimum {}",
                validator_id, stake, self.config.min_stake_for_activation
            )));
        }

        if voting_power < self.config.min_voting_power {
            return Err(Error::InvalidData(format!(
                "Validator {} voting power {} is below minimum {}",
                validator_id, voting_power, self.config.min_voting_power
            )));
        }

        // Check if already active
        if let Some(record) = self.records.get(&validator_id) {
            if record.status == ValidatorStatus::Active {
                return Err(Error::InvalidData(format!(
                    "Validator {} is already active",
                    validator_id
                )));
            }
        }

        // Create activation request
        let request = ActivationRequest {
            validator_id: validator_id.clone(),
            request_type: ActivationRequestType::Activation,
            requested_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            votes_in_favor: 0,
            total_votes: 0,
            status: ActivationRequestStatus::Pending,
        };

        self.pending_requests
            .insert(validator_id.clone(), request.clone());

        info!(
            "Requested activation for validator {} (stake: {}, voting power: {})",
            validator_id, stake, voting_power
        );

        Ok(request)
    }

    /// Request deactivation
    pub fn request_deactivation(&mut self, validator_id: ValidatorID) -> Result<ActivationRequest> {
        // Check if active
        if let Some(record) = self.records.get(&validator_id) {
            if record.status != ValidatorStatus::Active {
                return Err(Error::InvalidData(format!(
                    "Validator {} is not active (status: {})",
                    validator_id, record.status
                )));
            }
        } else {
            return Err(Error::InvalidData(format!(
                "Validator {} not found",
                validator_id
            )));
        }

        // Create deactivation request
        let request = ActivationRequest {
            validator_id: validator_id.clone(),
            request_type: ActivationRequestType::Deactivation,
            requested_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            votes_in_favor: 0,
            total_votes: 0,
            status: ActivationRequestStatus::Pending,
        };

        self.pending_requests
            .insert(validator_id.clone(), request.clone());

        info!("Requested deactivation for validator {}", validator_id);

        Ok(request)
    }

    /// Vote on activation request
    pub fn vote_on_request(
        &mut self,
        validator_id: &ValidatorID,
        vote_in_favor: bool,
        voting_power: u64,
    ) -> Result<()> {
        let request = self.pending_requests.get_mut(validator_id).ok_or_else(|| {
            Error::InvalidData(format!("No pending request for validator {}", validator_id))
        })?;

        request.total_votes += voting_power;
        if vote_in_favor {
            request.votes_in_favor += voting_power;
        }

        debug!(
            "Recorded vote for validator {}: {} votes in favor, {} total",
            validator_id, request.votes_in_favor, request.total_votes
        );

        Ok(())
    }

    /// Finalize activation request
    pub fn finalize_request(
        &mut self,
        validator_id: &ValidatorID,
        total_stake: u64,
    ) -> Result<ActivationRequestStatus> {
        let request = self.pending_requests.get_mut(validator_id).ok_or_else(|| {
            Error::InvalidData(format!("No pending request for validator {}", validator_id))
        })?;

        // Calculate required votes
        let required_votes = match request.request_type {
            ActivationRequestType::Activation => {
                (total_stake as f64 * self.config.activation_votes_required) as u64
            }
            ActivationRequestType::Deactivation => {
                (total_stake as f64 * self.config.deactivation_votes_required) as u64
            }
        };

        // Determine status
        let status = if request.votes_in_favor >= required_votes {
            ActivationRequestStatus::Approved
        } else {
            ActivationRequestStatus::Rejected
        };

        request.status = status;

        // Process approved requests
        if status == ActivationRequestStatus::Approved {
            match request.request_type {
                ActivationRequestType::Activation => {
                    let record = self
                        .records
                        .entry(validator_id.clone())
                        .or_insert_with(|| ValidatorActivationRecord::new(validator_id.clone()));

                    record.activate()?;

                    let event = ActivationEvent {
                        validator_id: validator_id.clone(),
                        event_type: ActivationEventType::Activated,
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        cycle: self.current_cycle,
                    };

                    self.events.push(event);

                    info!(
                        "Approved activation for validator {} ({}/{} votes)",
                        validator_id, request.votes_in_favor, required_votes
                    );
                }
                ActivationRequestType::Deactivation => {
                    if let Some(record) = self.records.get_mut(validator_id) {
                        record.deactivate(self.config.deactivation_cooldown_secs)?;

                        let event = ActivationEvent {
                            validator_id: validator_id.clone(),
                            event_type: ActivationEventType::Deactivated,
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            cycle: self.current_cycle,
                        };

                        self.events.push(event);

                        info!(
                            "Approved deactivation for validator {} ({}/{} votes)",
                            validator_id, request.votes_in_favor, required_votes
                        );
                    }
                }
            }
        } else {
            warn!(
                "Rejected {} request for validator {} ({}/{} votes)",
                request.request_type, validator_id, request.votes_in_favor, required_votes
            );
        }

        self.pending_requests.remove(validator_id);

        Ok(status)
    }

    /// Get validator status
    pub fn get_status(&self, validator_id: &ValidatorID) -> ValidatorStatus {
        self.records
            .get(validator_id)
            .map(|r| r.status)
            .unwrap_or(ValidatorStatus::Pending)
    }

    /// Get activation record
    pub fn get_record(&self, validator_id: &ValidatorID) -> Option<&ValidatorActivationRecord> {
        self.records.get(validator_id)
    }

    /// Get activation record mutable
    pub fn get_record_mut(
        &mut self,
        validator_id: &ValidatorID,
    ) -> Option<&mut ValidatorActivationRecord> {
        self.records.get_mut(validator_id)
    }

    /// Get pending request
    pub fn get_pending_request(&self, validator_id: &ValidatorID) -> Option<&ActivationRequest> {
        self.pending_requests.get(validator_id)
    }

    /// Get all active validators
    pub fn get_active_validators(&self) -> Vec<ValidatorID> {
        self.records
            .iter()
            .filter(|(_, record)| record.status == ValidatorStatus::Active)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get all pending requests
    pub fn get_pending_requests(&self) -> Vec<&ActivationRequest> {
        self.pending_requests.values().collect()
    }

    /// Get activation events
    pub fn get_events(&self) -> &[ActivationEvent] {
        &self.events
    }

    /// Get events for validator
    pub fn get_validator_events(&self, validator_id: &ValidatorID) -> Vec<&ActivationEvent> {
        self.events
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .collect()
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) {
        self.current_cycle += 1;
        debug!(
            "Advanced activation manager to cycle {}",
            self.current_cycle
        );
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get configuration
    pub fn config(&self) -> &ActivationConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: ActivationConfig) {
        self.config = config;
        info!("Updated activation manager configuration");
    }

    /// Get active validator count
    pub fn active_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.status == ValidatorStatus::Active)
            .count()
    }

    /// Get pending request count
    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.len()
    }

    /// Get event count
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}
