//! Downtime tracking and penalty system
//!
//! This module implements production-ready downtime tracking:
//! - Per-validator snapshot participation tracking
//! - Downtime calculation and thresholds
//! - Automatic jailing for excessive downtime
//! - Recovery mechanism with reduced penalties
//! - Comprehensive audit trail
//! - Full recovery support

use serde::{Deserialize, Serialize};
use silver_core::ValidatorID;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Downtime tracking configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowntimeConfig {
    /// Downtime threshold (missed snapshots before jailing)
    pub downtime_threshold: u64,

    /// Recovery period (snapshots to recover from downtime)
    pub recovery_period: u64,

    /// Minimum participation rate to avoid penalty (0.0 to 1.0)
    pub min_participation_rate: f64,

    /// Penalty reduction per recovery snapshot
    pub penalty_reduction_per_snapshot: f64,
}

impl Default for DowntimeConfig {
    fn default() -> Self {
        Self {
            downtime_threshold: 50,               // 50 missed snapshots
            recovery_period: 100,                 // 100 snapshots to recover
            min_participation_rate: 0.9,          // 90% minimum
            penalty_reduction_per_snapshot: 0.01, // 1% reduction per snapshot
        }
    }
}

/// Validator downtime record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorDowntimeRecord {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Total snapshots in current period
    pub total_snapshots: u64,

    /// Snapshots participated in
    pub snapshots_participated: u64,

    /// Snapshots missed
    pub snapshots_missed: u64,

    /// Current participation rate
    pub participation_rate: f64,

    /// Whether validator is in recovery
    pub in_recovery: bool,

    /// Recovery snapshots completed
    pub recovery_snapshots_completed: u64,

    /// Last downtime event
    pub last_downtime_event: Option<DowntimeEvent>,
}

impl ValidatorDowntimeRecord {
    /// Create new downtime record
    pub fn new(validator_id: ValidatorID) -> Self {
        Self {
            validator_id,
            total_snapshots: 0,
            snapshots_participated: 0,
            snapshots_missed: 0,
            participation_rate: 1.0,
            in_recovery: false,
            recovery_snapshots_completed: 0,
            last_downtime_event: None,
        }
    }

    /// Record snapshot participation
    pub fn record_participation(&mut self, participated: bool) {
        self.total_snapshots += 1;

        if participated {
            self.snapshots_participated += 1;
        } else {
            self.snapshots_missed += 1;
        }

        // Update participation rate
        if self.total_snapshots > 0 {
            self.participation_rate =
                self.snapshots_participated as f64 / self.total_snapshots as f64;
        }
    }

    /// Check if downtime threshold exceeded
    pub fn exceeds_threshold(&self, threshold: u64) -> bool {
        self.snapshots_missed >= threshold
    }

    /// Enter recovery mode
    pub fn enter_recovery(&mut self) {
        self.in_recovery = true;
        self.recovery_snapshots_completed = 0;
        debug!(
            "Validator {} entered recovery mode (missed: {})",
            self.validator_id, self.snapshots_missed
        );
    }

    /// Record recovery snapshot
    pub fn record_recovery_snapshot(&mut self) {
        if self.in_recovery {
            self.recovery_snapshots_completed += 1;
        }
    }

    /// Exit recovery mode
    pub fn exit_recovery(&mut self) {
        self.in_recovery = false;
        self.snapshots_missed = 0;
        self.recovery_snapshots_completed = 0;
        info!("Validator {} exited recovery mode", self.validator_id);
    }

    /// Reset for new period
    pub fn reset_period(&mut self) {
        self.total_snapshots = 0;
        self.snapshots_participated = 0;
        self.snapshots_missed = 0;
        self.participation_rate = 1.0;
        self.recovery_snapshots_completed = 0;
    }
}

/// Downtime event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowntimeEvent {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Event type
    pub event_type: DowntimeEventType,

    /// Snapshots missed
    pub snapshots_missed: u64,

    /// Participation rate
    pub participation_rate: f64,

    /// Timestamp
    pub timestamp: u64,

    /// Cycle
    pub cycle: u64,
}

/// Downtime event type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DowntimeEventType {
    /// Downtime threshold exceeded
    ThresholdExceeded,

    /// Entered recovery mode
    EnteredRecovery,

    /// Exited recovery mode
    ExitedRecovery,

    /// Participation warning
    ParticipationWarning,
}

impl std::fmt::Display for DowntimeEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DowntimeEventType::ThresholdExceeded => write!(f, "ThresholdExceeded"),
            DowntimeEventType::EnteredRecovery => write!(f, "EnteredRecovery"),
            DowntimeEventType::ExitedRecovery => write!(f, "ExitedRecovery"),
            DowntimeEventType::ParticipationWarning => write!(f, "ParticipationWarning"),
        }
    }
}

/// Downtime tracker
pub struct DowntimeTracker {
    /// Configuration
    config: DowntimeConfig,

    /// Validator downtime records
    records: HashMap<ValidatorID, ValidatorDowntimeRecord>,

    /// Downtime events
    events: Vec<DowntimeEvent>,

    /// Current cycle
    current_cycle: u64,
}

impl DowntimeTracker {
    /// Create new downtime tracker
    pub fn new(config: DowntimeConfig) -> Self {
        Self {
            config,
            records: HashMap::new(),
            events: Vec::new(),
            current_cycle: 0,
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(DowntimeConfig::default())
    }

    /// Record snapshot participation
    pub fn record_participation(&mut self, validator_id: ValidatorID, participated: bool) {
        let record = self
            .records
            .entry(validator_id.clone())
            .or_insert_with(|| ValidatorDowntimeRecord::new(validator_id.clone()));

        record.record_participation(participated);

        debug!(
            "Recorded participation for validator {}: {} (missed: {})",
            validator_id, participated, record.snapshots_missed
        );
    }

    /// Check for downtime violations
    pub fn check_downtime_violations(&mut self) -> Vec<ValidatorID> {
        let mut violations = Vec::new();

        for (validator_id, record) in self.records.iter_mut() {
            // Check if threshold exceeded
            if record.exceeds_threshold(self.config.downtime_threshold) && !record.in_recovery {
                violations.push(validator_id.clone());
                record.enter_recovery();

                let event = DowntimeEvent {
                    validator_id: validator_id.clone(),
                    event_type: DowntimeEventType::ThresholdExceeded,
                    snapshots_missed: record.snapshots_missed,
                    participation_rate: record.participation_rate,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    cycle: self.current_cycle,
                };

                self.events.push(event);

                warn!(
                    "Validator {} exceeded downtime threshold: {} missed snapshots",
                    validator_id, record.snapshots_missed
                );
            }

            // Check participation rate
            if record.participation_rate < self.config.min_participation_rate && !record.in_recovery
            {
                let event = DowntimeEvent {
                    validator_id: validator_id.clone(),
                    event_type: DowntimeEventType::ParticipationWarning,
                    snapshots_missed: record.snapshots_missed,
                    participation_rate: record.participation_rate,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    cycle: self.current_cycle,
                };

                self.events.push(event);

                warn!(
                    "Validator {} participation rate below minimum: {:.2}%",
                    validator_id,
                    record.participation_rate * 100.0
                );
            }
        }

        violations
    }

    /// Process recovery snapshots
    pub fn process_recovery(&mut self) -> Vec<ValidatorID> {
        let mut recovered = Vec::new();

        for (validator_id, record) in self.records.iter_mut() {
            if record.in_recovery {
                record.record_recovery_snapshot();

                // Check if recovery complete
                if record.recovery_snapshots_completed >= self.config.recovery_period {
                    record.exit_recovery();
                    recovered.push(validator_id.clone());

                    let event = DowntimeEvent {
                        validator_id: validator_id.clone(),
                        event_type: DowntimeEventType::ExitedRecovery,
                        snapshots_missed: 0,
                        participation_rate: record.participation_rate,
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        cycle: self.current_cycle,
                    };

                    self.events.push(event);

                    info!("Validator {} recovered from downtime", validator_id);
                }
            }
        }

        recovered
    }

    /// Get validator downtime record
    pub fn get_record(&self, validator_id: &ValidatorID) -> Option<&ValidatorDowntimeRecord> {
        self.records.get(validator_id)
    }

    /// Get validator downtime record mutable
    pub fn get_record_mut(
        &mut self,
        validator_id: &ValidatorID,
    ) -> Option<&mut ValidatorDowntimeRecord> {
        self.records.get_mut(validator_id)
    }

    /// Get validators in recovery
    pub fn get_validators_in_recovery(&self) -> Vec<ValidatorID> {
        self.records
            .iter()
            .filter(|(_, record)| record.in_recovery)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get validators with low participation
    pub fn get_low_participation_validators(&self) -> Vec<ValidatorID> {
        self.records
            .iter()
            .filter(|(_, record)| record.participation_rate < self.config.min_participation_rate)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get downtime events
    pub fn get_events(&self) -> &[DowntimeEvent] {
        &self.events
    }

    /// Get events for validator
    pub fn get_validator_events(&self, validator_id: &ValidatorID) -> Vec<&DowntimeEvent> {
        self.events
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .collect()
    }

    /// Get penalty reduction for validator
    pub fn get_penalty_reduction(&self, validator_id: &ValidatorID) -> f64 {
        if let Some(record) = self.records.get(validator_id) {
            if record.in_recovery {
                record.recovery_snapshots_completed as f64
                    * self.config.penalty_reduction_per_snapshot
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) {
        self.current_cycle += 1;

        // Reset period statistics
        for record in self.records.values_mut() {
            if !record.in_recovery {
                record.reset_period();
            }
        }

        debug!("Advanced downtime tracker to cycle {}", self.current_cycle);
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get configuration
    pub fn config(&self) -> &DowntimeConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: DowntimeConfig) {
        self.config = config;
        info!("Updated downtime tracker configuration");
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.records.len()
    }

    /// Get validators in recovery count
    pub fn recovery_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.in_recovery)
            .count()
    }

    /// Get event count
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}
