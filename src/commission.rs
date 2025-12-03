//! Validator commission system
//!
//! This module implements validator commission rates with:
//! - Commission rate range: 5-20%
//! - 7-day notice period for rate changes
//! - Commission rate history tracking

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

/// Minimum commission rate (5% = 500 basis points)
pub const MIN_COMMISSION_RATE: u16 = 500;

/// Maximum commission rate (20% = 2000 basis points)
pub const MAX_COMMISSION_RATE: u16 = 2000;

/// Commission rate change notice period (7 days in seconds)
pub const COMMISSION_CHANGE_NOTICE_PERIOD: u64 = 7 * 24 * 60 * 60;

/// Commission rate (in basis points, 1 bp = 0.01%)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionRate(pub u16);

impl CommissionRate {
    /// Create a new commission rate
    pub fn new(rate_bp: u16) -> Result<Self> {
        if rate_bp < MIN_COMMISSION_RATE {
            return Err(Error::InvalidData(format!(
                "Commission rate {} is below minimum {} (5%)",
                rate_bp, MIN_COMMISSION_RATE
            )));
        }

        if rate_bp > MAX_COMMISSION_RATE {
            return Err(Error::InvalidData(format!(
                "Commission rate {} exceeds maximum {} (20%)",
                rate_bp, MAX_COMMISSION_RATE
            )));
        }

        Ok(Self(rate_bp))
    }

    /// Get rate in basis points
    pub fn basis_points(&self) -> u16 {
        self.0
    }

    /// Get rate as percentage (e.g., 500 bp = 5.0%)
    pub fn as_percentage(&self) -> f64 {
        self.0 as f64 / 100.0
    }

    /// Get rate as decimal (e.g., 500 bp = 0.05)
    pub fn as_decimal(&self) -> f64 {
        self.0 as f64 / 10000.0
    }

    /// Calculate commission amount from total rewards
    pub fn calculate_commission(&self, total_rewards: u64) -> u64 {
        (total_rewards as f64 * self.as_decimal()) as u64
    }

    /// Calculate delegator share after commission
    pub fn calculate_delegator_share(&self, total_rewards: u64) -> u64 {
        total_rewards - self.calculate_commission(total_rewards)
    }
}

impl Default for CommissionRate {
    fn default() -> Self {
        Self(MIN_COMMISSION_RATE) // Default to 5%
    }
}

/// Commission rate change request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommissionRateChange {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Current rate
    pub old_rate: CommissionRate,

    /// New rate
    pub new_rate: CommissionRate,

    /// Request timestamp
    pub requested_at: u64,

    /// Effective timestamp (after notice period)
    pub effective_at: u64,

    /// Whether the change has been applied
    pub applied: bool,
}

impl CommissionRateChange {
    /// Create a new commission rate change request
    pub fn new(
        validator_id: ValidatorID,
        old_rate: CommissionRate,
        new_rate: CommissionRate,
    ) -> Self {
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let effective_at = requested_at + COMMISSION_CHANGE_NOTICE_PERIOD;

        Self {
            validator_id,
            old_rate,
            new_rate,
            requested_at,
            effective_at,
            applied: false,
        }
    }

    /// Check if change is effective
    pub fn is_effective(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now >= self.effective_at
    }

    /// Get remaining notice period in seconds
    pub fn remaining_notice_period(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if now >= self.effective_at {
            0
        } else {
            self.effective_at - now
        }
    }
}

/// Validator commission info
#[derive(Debug, Clone)]
pub struct ValidatorCommissionInfo {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Current commission rate
    pub current_rate: CommissionRate,

    /// Pending rate change
    pub pending_change: Option<CommissionRateChange>,

    /// Commission rate history
    pub rate_history: Vec<CommissionRateChange>,

    /// Total commission earned
    pub total_commission_earned: u64,
}

impl ValidatorCommissionInfo {
    /// Create new validator commission info
    pub fn new(validator_id: ValidatorID, initial_rate: CommissionRate) -> Self {
        Self {
            validator_id,
            current_rate: initial_rate,
            pending_change: None,
            rate_history: Vec::new(),
            total_commission_earned: 0,
        }
    }

    /// Request commission rate change
    pub fn request_rate_change(
        &mut self,
        new_rate: CommissionRate,
    ) -> Result<CommissionRateChange> {
        if self.pending_change.is_some() {
            return Err(Error::InvalidData(format!(
                "Validator {} already has a pending commission rate change",
                self.validator_id
            )));
        }

        if new_rate == self.current_rate {
            return Err(Error::InvalidData(
                "New commission rate is the same as current rate".to_string(),
            ));
        }

        let change =
            CommissionRateChange::new(self.validator_id.clone(), self.current_rate, new_rate);

        self.pending_change = Some(change.clone());

        info!(
            "Validator {} requested commission rate change from {}% to {}% (effective at: {})",
            self.validator_id,
            self.current_rate.as_percentage(),
            new_rate.as_percentage(),
            change.effective_at
        );

        Ok(change)
    }

    /// Apply pending rate change if effective
    pub fn apply_pending_change(&mut self) -> Option<CommissionRateChange> {
        if let Some(mut change) = self.pending_change.take() {
            if change.is_effective() {
                change.applied = true;
                self.current_rate = change.new_rate;
                self.rate_history.push(change.clone());

                info!(
                    "Applied commission rate change for validator {}: {}% -> {}%",
                    self.validator_id,
                    change.old_rate.as_percentage(),
                    change.new_rate.as_percentage()
                );

                return Some(change);
            } else {
                // Put it back if not effective yet
                self.pending_change = Some(change);
            }
        }

        None
    }

    /// Cancel pending rate change
    pub fn cancel_pending_change(&mut self) -> Result<()> {
        if self.pending_change.is_none() {
            return Err(Error::InvalidData(format!(
                "Validator {} has no pending commission rate change",
                self.validator_id
            )));
        }

        self.pending_change = None;

        info!(
            "Cancelled pending commission rate change for validator {}",
            self.validator_id
        );

        Ok(())
    }

    /// Calculate commission from rewards
    pub fn calculate_commission(&mut self, total_rewards: u64) -> u64 {
        let commission = self.current_rate.calculate_commission(total_rewards);
        self.total_commission_earned += commission;
        commission
    }

    /// Get current rate
    pub fn current_rate(&self) -> CommissionRate {
        self.current_rate
    }

    /// Get rate history
    pub fn rate_history(&self) -> &[CommissionRateChange] {
        &self.rate_history
    }

    /// Get total commission earned
    pub fn total_commission_earned(&self) -> u64 {
        self.total_commission_earned
    }
}

/// Commission manager
///
/// Manages commission rates for all validators
pub struct CommissionManager {
    /// Validator commission info indexed by validator ID
    validators: HashMap<ValidatorID, ValidatorCommissionInfo>,
}

impl CommissionManager {
    /// Create a new commission manager
    pub fn new() -> Self {
        Self {
            validators: HashMap::new(),
        }
    }

    /// Register validator with initial commission rate
    pub fn register_validator(
        &mut self,
        validator_id: ValidatorID,
        initial_rate: CommissionRate,
    ) -> Result<()> {
        if self.validators.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} already registered",
                validator_id
            )));
        }

        let info = ValidatorCommissionInfo::new(validator_id.clone(), initial_rate);
        self.validators.insert(validator_id.clone(), info);

        info!(
            "Registered validator {} with {}% commission rate",
            validator_id,
            initial_rate.as_percentage()
        );

        Ok(())
    }

    /// Set validator commission rate
    pub fn set_commission_rate(
        &mut self,
        validator_id: &ValidatorID,
        new_rate: CommissionRate,
    ) -> Result<CommissionRateChange> {
        let info = self
            .validators
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        info.request_rate_change(new_rate)
    }

    /// Cancel pending commission rate change
    pub fn cancel_rate_change(&mut self, validator_id: &ValidatorID) -> Result<()> {
        let info = self
            .validators
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        info.cancel_pending_change()
    }

    /// Process all pending commission rate changes
    pub fn process_pending_changes(&mut self) -> Vec<CommissionRateChange> {
        let mut applied_changes = Vec::new();

        for info in self.validators.values_mut() {
            if let Some(change) = info.apply_pending_change() {
                applied_changes.push(change);
            }
        }

        if !applied_changes.is_empty() {
            info!("Applied {} commission rate changes", applied_changes.len());
        }

        applied_changes
    }

    /// Calculate commission for validator
    pub fn calculate_commission(
        &mut self,
        validator_id: &ValidatorID,
        total_rewards: u64,
    ) -> Result<u64> {
        let info = self
            .validators
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        Ok(info.calculate_commission(total_rewards))
    }

    /// Get validator commission rate
    pub fn get_commission_rate(&self, validator_id: &ValidatorID) -> Option<CommissionRate> {
        self.validators
            .get(validator_id)
            .map(|info| info.current_rate())
    }

    /// Get validator commission info
    pub fn get_commission_info(
        &self,
        validator_id: &ValidatorID,
    ) -> Option<&ValidatorCommissionInfo> {
        self.validators.get(validator_id)
    }

    /// Get all validators with pending rate changes
    pub fn get_pending_changes(&self) -> Vec<&CommissionRateChange> {
        self.validators
            .values()
            .filter_map(|info| info.pending_change.as_ref())
            .collect()
    }

    /// Remove validator
    pub fn remove_validator(&mut self, validator_id: &ValidatorID) -> Result<()> {
        self.validators
            .remove(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        info!("Removed validator {} from commission system", validator_id);
        Ok(())
    }
}

impl Default for CommissionManager {
    fn default() -> Self {
        Self::new()
    }
}
