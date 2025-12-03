//! Slashing and jailing system for validator penalties
//!
//! This module implements production-ready slashing and jailing:
//! - Double signing detection and slashing
//! - Downtime tracking and penalties
//! - Automatic jailing for violations
//! - Jail recovery mechanism
//! - Slash amount calculation
//! - Comprehensive audit trail
//! - Full recovery support

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

/// Slashing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingConfig {
    /// Percentage of stake slashed for double signing (0-100)
    pub slash_double_signing: u64,

    /// Percentage of stake slashed for downtime (0-100)
    pub slash_downtime: u64,

    /// Jail duration in seconds
    pub jail_duration_secs: u64,

    /// Downtime threshold (missed snapshots before jailing)
    pub downtime_threshold: u64,

    /// Recovery period in snapshots (to reduce jail time)
    pub recovery_period: u64,
}

impl Default for SlashingConfig {
    fn default() -> Self {
        Self {
            slash_double_signing: 10,  // 10% for double signing
            slash_downtime: 1,         // 1% for downtime
            jail_duration_secs: 86400, // 24 hours
            downtime_threshold: 50,    // 50 missed snapshots
            recovery_period: 100,      // 100 snapshots to recover
        }
    }
}

/// Jail reason
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JailReason {
    /// Double signing detected
    DoubleSigning,

    /// Excessive downtime
    ExcessiveDowntime,

    /// Manual jailing by governance
    Manual,
}

impl std::fmt::Display for JailReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailReason::DoubleSigning => write!(f, "DoubleSigning"),
            JailReason::ExcessiveDowntime => write!(f, "ExcessiveDowntime"),
            JailReason::Manual => write!(f, "Manual"),
        }
    }
}

/// Jail record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailRecord {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// When jailed
    pub jailed_at: u64,

    /// When jail expires
    pub jail_expires_at: u64,

    /// Reason for jailing
    pub reason: JailReason,

    /// Amount slashed
    pub slash_amount: u64,

    /// Snapshots missed (for downtime)
    pub snapshots_missed: Option<u64>,

    /// Whether jail was manually lifted
    pub manually_lifted: bool,
}

impl JailRecord {
    /// Create new jail record
    pub fn new(
        validator_id: ValidatorID,
        reason: JailReason,
        slash_amount: u64,
        jail_duration_secs: u64,
    ) -> Self {
        let jailed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let jail_expires_at = jailed_at + jail_duration_secs;

        Self {
            validator_id,
            jailed_at,
            jail_expires_at,
            reason,
            slash_amount,
            snapshots_missed: None,
            manually_lifted: false,
        }
    }

    /// Check if jail is active
    pub fn is_jailed(&self) -> bool {
        if self.manually_lifted {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now < self.jail_expires_at
    }

    /// Get remaining jail time in seconds
    pub fn remaining_jail_time(&self) -> u64 {
        if self.manually_lifted {
            return 0;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if now >= self.jail_expires_at {
            0
        } else {
            self.jail_expires_at - now
        }
    }

    /// Manually lift jail
    pub fn lift_jail(&mut self) {
        self.manually_lifted = true;
        info!(
            "Manually lifted jail for validator {} (reason: {})",
            self.validator_id, self.reason
        );
    }
}

/// Slash event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashEvent {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Slash reason
    pub reason: JailReason,

    /// Amount slashed
    pub slash_amount: u64,

    /// Timestamp
    pub timestamp: u64,

    /// Cycle (if applicable)
    pub cycle: Option<u64>,
}

/// Slashing manager
pub struct SlashingManager {
    /// Configuration
    config: SlashingConfig,

    /// Jailed validators
    jailed_validators: HashMap<ValidatorID, JailRecord>,

    /// Slash history
    slash_history: Vec<SlashEvent>,

    /// Double signing evidence (to prevent duplicate slashing)
    double_signing_evidence: HashMap<ValidatorID, Vec<u8>>,
}

impl SlashingManager {
    /// Create new slashing manager
    pub fn new(config: SlashingConfig) -> Self {
        Self {
            config,
            jailed_validators: HashMap::new(),
            slash_history: Vec::new(),
            double_signing_evidence: HashMap::new(),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(SlashingConfig::default())
    }

    /// Slash validator for double signing
    pub fn slash_for_double_signing(
        &mut self,
        validator_id: ValidatorID,
        stake: u64,
        evidence: Vec<u8>,
    ) -> Result<(u64, JailRecord)> {
        // Check if already slashed for this evidence
        if let Some(existing_evidence) = self.double_signing_evidence.get(&validator_id) {
            if existing_evidence == &evidence {
                return Err(Error::InvalidData(format!(
                    "Validator {} already slashed for this double signing evidence",
                    validator_id
                )));
            }
        }

        // Check if already jailed
        if self.is_jailed(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} is already jailed",
                validator_id
            )));
        }

        // Calculate slash amount
        let slash_amount = (stake * self.config.slash_double_signing) / 100;

        if slash_amount == 0 {
            return Err(Error::InvalidData(
                "Slash amount cannot be zero".to_string(),
            ));
        }

        // Create jail record
        let jail_record = JailRecord::new(
            validator_id.clone(),
            JailReason::DoubleSigning,
            slash_amount,
            self.config.jail_duration_secs,
        );

        // Store evidence
        self.double_signing_evidence
            .insert(validator_id.clone(), evidence);

        // Record slash event
        let event = SlashEvent {
            validator_id: validator_id.clone(),
            reason: JailReason::DoubleSigning,
            slash_amount,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            cycle: None,
        };

        self.slash_history.push(event);
        self.jailed_validators
            .insert(validator_id.clone(), jail_record.clone());

        error!(
            "Slashed validator {} for double signing: {} SBTC ({}% of stake)",
            validator_id, slash_amount, self.config.slash_double_signing
        );

        Ok((slash_amount, jail_record))
    }

    /// Slash validator for downtime
    pub fn slash_for_downtime(
        &mut self,
        validator_id: ValidatorID,
        stake: u64,
        snapshots_missed: u64,
        cycle: u64,
    ) -> Result<(u64, JailRecord)> {
        // Check if already jailed
        if self.is_jailed(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} is already jailed",
                validator_id
            )));
        }

        // Check if downtime exceeds threshold
        if snapshots_missed < self.config.downtime_threshold {
            return Err(Error::InvalidData(format!(
                "Downtime {} is below threshold {}",
                snapshots_missed, self.config.downtime_threshold
            )));
        }

        // Calculate slash amount
        let slash_amount = (stake * self.config.slash_downtime) / 100;

        if slash_amount == 0 {
            return Err(Error::InvalidData(
                "Slash amount cannot be zero".to_string(),
            ));
        }

        // Create jail record
        let mut jail_record = JailRecord::new(
            validator_id.clone(),
            JailReason::ExcessiveDowntime,
            slash_amount,
            self.config.jail_duration_secs,
        );

        jail_record.snapshots_missed = Some(snapshots_missed);

        // Record slash event
        let event = SlashEvent {
            validator_id: validator_id.clone(),
            reason: JailReason::ExcessiveDowntime,
            slash_amount,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            cycle: Some(cycle),
        };

        self.slash_history.push(event);
        self.jailed_validators
            .insert(validator_id.clone(), jail_record.clone());

        warn!(
            "Slashed validator {} for downtime: {} SBTC ({}% of stake, {} snapshots missed)",
            validator_id, slash_amount, self.config.slash_downtime, snapshots_missed
        );

        Ok((slash_amount, jail_record))
    }

    /// Manually jail validator (governance)
    pub fn jail_validator_manual(
        &mut self,
        validator_id: ValidatorID,
        _stake: u64,
    ) -> Result<JailRecord> {
        // Check if already jailed
        if self.is_jailed(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} is already jailed",
                validator_id
            )));
        }

        // Calculate slash amount (0% for manual jailing)
        let slash_amount = 0;

        // Create jail record
        let jail_record = JailRecord::new(
            validator_id.clone(),
            JailReason::Manual,
            slash_amount,
            self.config.jail_duration_secs,
        );

        // Record slash event
        let event = SlashEvent {
            validator_id: validator_id.clone(),
            reason: JailReason::Manual,
            slash_amount,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            cycle: None,
        };

        self.slash_history.push(event);
        self.jailed_validators
            .insert(validator_id.clone(), jail_record.clone());

        info!("Manually jailed validator {} by governance", validator_id);

        Ok(jail_record)
    }

    /// Check if validator is jailed
    pub fn is_jailed(&self, validator_id: &ValidatorID) -> bool {
        self.jailed_validators
            .get(validator_id)
            .map(|record| record.is_jailed())
            .unwrap_or(false)
    }

    /// Get jail record
    pub fn get_jail_record(&self, validator_id: &ValidatorID) -> Option<&JailRecord> {
        self.jailed_validators.get(validator_id)
    }

    /// Get jail record mutable
    pub fn get_jail_record_mut(&mut self, validator_id: &ValidatorID) -> Option<&mut JailRecord> {
        self.jailed_validators.get_mut(validator_id)
    }

    /// Lift jail manually
    pub fn lift_jail(&mut self, validator_id: &ValidatorID) -> Result<()> {
        if let Some(record) = self.jailed_validators.get_mut(validator_id) {
            record.lift_jail();
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Validator {} is not jailed",
                validator_id
            )))
        }
    }

    /// Process expired jails (remove from jailed set)
    pub fn process_expired_jails(&mut self) -> Vec<ValidatorID> {
        let mut expired = Vec::new();

        for (validator_id, record) in self.jailed_validators.iter() {
            if !record.is_jailed() {
                expired.push(validator_id.clone());
            }
        }

        for validator_id in &expired {
            self.jailed_validators.remove(validator_id);
            info!("Validator {} jail expired", validator_id);
        }

        expired
    }

    /// Get all jailed validators
    pub fn get_jailed_validators(&self) -> Vec<ValidatorID> {
        self.jailed_validators
            .iter()
            .filter(|(_, record)| record.is_jailed())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get slash history
    pub fn get_slash_history(&self) -> &[SlashEvent] {
        &self.slash_history
    }

    /// Get slash history for validator
    pub fn get_validator_slash_history(&self, validator_id: &ValidatorID) -> Vec<&SlashEvent> {
        self.slash_history
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .collect()
    }

    /// Get total slashed amount for validator
    pub fn get_total_slashed(&self, validator_id: &ValidatorID) -> u64 {
        self.slash_history
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .map(|event| event.slash_amount)
            .sum()
    }

    /// Get configuration
    pub fn config(&self) -> &SlashingConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: SlashingConfig) {
        self.config = config;
        info!("Updated slashing configuration");
    }

    /// Get jailed validator count
    pub fn jailed_count(&self) -> usize {
        self.jailed_validators
            .values()
            .filter(|record| record.is_jailed())
            .count()
    }

    /// Get total slash events
    pub fn slash_event_count(&self) -> usize {
        self.slash_history.len()
    }
}
