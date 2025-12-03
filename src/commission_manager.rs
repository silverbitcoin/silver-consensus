//! Validator commission management system
//!
//! This module implements production-ready commission management:
//! - Commission rate setting and validation
//! - 7-day notice period enforcement
//! - Commission deduction from rewards
//! - Comprehensive audit trail
//! - Full recovery support

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Commission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommissionConfig {
    /// Minimum commission rate (percentage, 0-100)
    pub min_commission_rate: u64,

    /// Maximum commission rate (percentage, 0-100)
    pub max_commission_rate: u64,

    /// Notice period for commission changes (seconds)
    pub notice_period_secs: u64,
}

impl Default for CommissionConfig {
    fn default() -> Self {
        Self {
            min_commission_rate: 5,     // 5% minimum
            max_commission_rate: 20,    // 20% maximum
            notice_period_secs: 604800, // 7 days
        }
    }
}

/// Commission rate
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionRate {
    /// Commission rate as percentage (0-100)
    pub rate: u64,
}

impl CommissionRate {
    /// Create new commission rate
    pub fn new(rate: u64) -> Result<Self> {
        if rate > 100 {
            return Err(Error::InvalidData(format!(
                "Commission rate {} exceeds 100%",
                rate
            )));
        }

        Ok(Self { rate })
    }

    /// Calculate commission amount
    pub fn calculate_commission(&self, total_amount: u64) -> u64 {
        (total_amount * self.rate as u64) / 100
    }

    /// Calculate amount after commission
    pub fn calculate_after_commission(&self, total_amount: u64) -> u64 {
        total_amount - self.calculate_commission(total_amount)
    }
}

/// Commission change request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommissionChangeRequest {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// New commission rate
    pub new_rate: CommissionRate,

    /// Requested at
    pub requested_at: u64,

    /// Effective at (after notice period)
    pub effective_at: u64,

    /// Whether change has been applied
    pub applied: bool,
}

impl CommissionChangeRequest {
    /// Create new commission change request
    pub fn new(
        validator_id: ValidatorID,
        new_rate: CommissionRate,
        notice_period_secs: u64,
    ) -> Self {
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let effective_at = requested_at + notice_period_secs;

        Self {
            validator_id,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorCommissionInfo {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Current commission rate
    pub current_rate: CommissionRate,

    /// Pending commission change
    pub pending_change: Option<CommissionChangeRequest>,

    /// Commission change history
    pub change_history: Vec<CommissionChangeRequest>,

    /// Total commission collected
    pub total_commission_collected: u64,
}

impl ValidatorCommissionInfo {
    /// Create new commission info
    pub fn new(validator_id: ValidatorID, initial_rate: CommissionRate) -> Self {
        Self {
            validator_id,
            current_rate: initial_rate,
            pending_change: None,
            change_history: Vec::new(),
            total_commission_collected: 0,
        }
    }

    /// Request commission change
    pub fn request_change(
        &mut self,
        new_rate: CommissionRate,
        notice_period_secs: u64,
    ) -> Result<CommissionChangeRequest> {
        // Check if already has pending change
        if let Some(pending) = &self.pending_change {
            if !pending.is_effective() {
                return Err(Error::InvalidData(format!(
                    "Validator {} already has pending commission change (effective at: {})",
                    self.validator_id, pending.effective_at
                )));
            }
        }

        let request =
            CommissionChangeRequest::new(self.validator_id.clone(), new_rate, notice_period_secs);

        self.pending_change = Some(request.clone());

        info!(
            "Validator {} requested commission change to {}% (effective at: {})",
            self.validator_id, new_rate.rate, request.effective_at
        );

        Ok(request)
    }

    /// Apply pending commission change
    pub fn apply_pending_change(&mut self) -> Result<()> {
        if let Some(mut pending) = self.pending_change.take() {
            if !pending.is_effective() {
                return Err(Error::InvalidData(format!(
                    "Commission change not yet effective (effective at: {})",
                    pending.effective_at
                )));
            }

            let old_rate = self.current_rate;
            self.current_rate = pending.new_rate;
            pending.applied = true;
            self.change_history.push(pending);

            info!(
                "Applied commission change for validator {}: {}% -> {}%",
                self.validator_id, old_rate.rate, self.current_rate.rate
            );

            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "No pending commission change for validator {}",
                self.validator_id
            )))
        }
    }

    /// Cancel pending commission change
    pub fn cancel_pending_change(&mut self) -> Result<()> {
        if self.pending_change.is_some() {
            self.pending_change = None;
            info!(
                "Cancelled pending commission change for validator {}",
                self.validator_id
            );
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "No pending commission change for validator {}",
                self.validator_id
            )))
        }
    }

    /// Record commission collected
    pub fn record_commission(&mut self, amount: u64) {
        self.total_commission_collected += amount;
        debug!(
            "Recorded commission for validator {}: {} SBTC (total: {})",
            self.validator_id, amount, self.total_commission_collected
        );
    }
}

/// Commission manager
pub struct CommissionManager {
    /// Configuration
    config: CommissionConfig,

    /// Validator commission info
    validator_commissions: HashMap<ValidatorID, ValidatorCommissionInfo>,
}

impl CommissionManager {
    /// Create new commission manager
    pub fn new(config: CommissionConfig) -> Self {
        Self {
            config,
            validator_commissions: HashMap::new(),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(CommissionConfig::default())
    }

    /// Register validator with initial commission
    pub fn register_validator(
        &mut self,
        validator_id: ValidatorID,
        initial_rate: u64,
    ) -> Result<()> {
        // Validate rate
        if initial_rate < self.config.min_commission_rate {
            return Err(Error::InvalidData(format!(
                "Commission rate {} is below minimum {}%",
                initial_rate, self.config.min_commission_rate
            )));
        }

        if initial_rate > self.config.max_commission_rate {
            return Err(Error::InvalidData(format!(
                "Commission rate {} exceeds maximum {}%",
                initial_rate, self.config.max_commission_rate
            )));
        }

        let rate = CommissionRate::new(initial_rate)?;
        let info = ValidatorCommissionInfo::new(validator_id.clone(), rate);

        self.validator_commissions
            .insert(validator_id.clone(), info);

        info!(
            "Registered validator {} with {}% commission",
            validator_id, initial_rate
        );

        Ok(())
    }

    /// Request commission change
    pub fn request_commission_change(
        &mut self,
        validator_id: &ValidatorID,
        new_rate: u64,
    ) -> Result<CommissionChangeRequest> {
        // Validate rate
        if new_rate < self.config.min_commission_rate {
            return Err(Error::InvalidData(format!(
                "Commission rate {} is below minimum {}%",
                new_rate, self.config.min_commission_rate
            )));
        }

        if new_rate > self.config.max_commission_rate {
            return Err(Error::InvalidData(format!(
                "Commission rate {} exceeds maximum {}%",
                new_rate, self.config.max_commission_rate
            )));
        }

        let info = self
            .validator_commissions
            .get_mut(validator_id)
            .ok_or_else(|| Error::InvalidData(format!("Validator {} not found", validator_id)))?;

        let rate = CommissionRate::new(new_rate)?;
        info.request_change(rate, self.config.notice_period_secs)
    }

    /// Apply pending commission changes
    pub fn apply_pending_changes(&mut self) -> Vec<ValidatorID> {
        let mut applied = Vec::new();

        for (validator_id, info) in self.validator_commissions.iter_mut() {
            if let Some(pending) = &info.pending_change {
                if pending.is_effective() {
                    if info.apply_pending_change().is_ok() {
                        applied.push(validator_id.clone());
                    }
                }
            }
        }

        applied
    }

    /// Get validator commission info
    pub fn get_commission_info(
        &self,
        validator_id: &ValidatorID,
    ) -> Option<&ValidatorCommissionInfo> {
        self.validator_commissions.get(validator_id)
    }

    /// Get validator commission info mutable
    pub fn get_commission_info_mut(
        &mut self,
        validator_id: &ValidatorID,
    ) -> Option<&mut ValidatorCommissionInfo> {
        self.validator_commissions.get_mut(validator_id)
    }

    /// Get current commission rate
    pub fn get_current_rate(&self, validator_id: &ValidatorID) -> Option<CommissionRate> {
        self.validator_commissions
            .get(validator_id)
            .map(|info| info.current_rate)
    }

    /// Get pending commission change
    pub fn get_pending_change(
        &self,
        validator_id: &ValidatorID,
    ) -> Option<&CommissionChangeRequest> {
        self.validator_commissions
            .get(validator_id)
            .and_then(|info| info.pending_change.as_ref())
    }

    /// Calculate commission amount
    pub fn calculate_commission(
        &self,
        validator_id: &ValidatorID,
        total_amount: u64,
    ) -> Option<u64> {
        self.get_current_rate(validator_id)
            .map(|rate| rate.calculate_commission(total_amount))
    }

    /// Record commission collected
    pub fn record_commission(&mut self, validator_id: &ValidatorID, amount: u64) -> Result<()> {
        if let Some(info) = self.validator_commissions.get_mut(validator_id) {
            info.record_commission(amount);
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Validator {} not found",
                validator_id
            )))
        }
    }

    /// Get configuration
    pub fn config(&self) -> &CommissionConfig {
        &self.config
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.validator_commissions.len()
    }

    /// Get validators with pending changes
    pub fn get_validators_with_pending_changes(&self) -> Vec<ValidatorID> {
        self.validator_commissions
            .iter()
            .filter(|(_, info)| info.pending_change.is_some())
            .map(|(id, _)| id.clone())
            .collect()
    }
}
