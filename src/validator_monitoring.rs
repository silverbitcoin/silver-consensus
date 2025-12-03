//! Validator operations monitoring and performance tracking
//!
//! This module implements production-ready validator monitoring:
//! - Performance metric collection
//! - Uptime and participation tracking
//! - Performance degradation detection
//! - Alert generation
//! - History tracking

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Validator performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorMetrics {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Total snapshots participated
    pub snapshots_participated: u64,

    /// Total snapshots in period
    pub total_snapshots: u64,

    /// Participation rate (0.0 to 1.0)
    pub participation_rate: f64,

    /// Average response time (milliseconds)
    pub avg_response_time_ms: u64,

    /// Minimum response time (milliseconds)
    pub min_response_time_ms: u64,

    /// Maximum response time (milliseconds)
    pub max_response_time_ms: u64,

    /// Uptime percentage (0.0 to 100.0)
    pub uptime_percentage: f64,

    /// Last seen timestamp
    pub last_seen: u64,

    /// Consecutive failures
    pub consecutive_failures: u64,

    /// Total failures
    pub total_failures: u64,
}

impl ValidatorMetrics {
    /// Create new metrics
    pub fn new(validator_id: ValidatorID) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            validator_id,
            snapshots_participated: 0,
            total_snapshots: 0,
            participation_rate: 1.0,
            avg_response_time_ms: 0,
            min_response_time_ms: u64::MAX,
            max_response_time_ms: 0,
            uptime_percentage: 100.0,
            last_seen: now,
            consecutive_failures: 0,
            total_failures: 0,
        }
    }

    /// Record snapshot participation
    pub fn record_snapshot(&mut self, participated: bool, response_time_ms: u64) {
        self.total_snapshots += 1;
        self.last_seen = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if participated {
            self.snapshots_participated += 1;
            self.consecutive_failures = 0;

            // Update response time metrics
            if response_time_ms < self.min_response_time_ms {
                self.min_response_time_ms = response_time_ms;
            }
            if response_time_ms > self.max_response_time_ms {
                self.max_response_time_ms = response_time_ms;
            }

            // Update average response time
            if self.snapshots_participated > 0 {
                self.avg_response_time_ms = (self.avg_response_time_ms
                    * (self.snapshots_participated - 1)
                    + response_time_ms)
                    / self.snapshots_participated;
            }
        } else {
            self.consecutive_failures += 1;
            self.total_failures += 1;
        }

        // Update participation rate
        if self.total_snapshots > 0 {
            self.participation_rate =
                self.snapshots_participated as f64 / self.total_snapshots as f64;
        }

        // Update uptime percentage
        if self.total_snapshots > 0 {
            self.uptime_percentage =
                (self.snapshots_participated as f64 / self.total_snapshots as f64) * 100.0;
        }
    }

    /// Check if performance is degraded
    pub fn is_performance_degraded(&self, threshold: f64) -> bool {
        self.participation_rate < threshold
    }

    /// Check if validator is offline
    pub fn is_offline(&self, timeout_secs: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now - self.last_seen > timeout_secs
    }

    /// Reset metrics for new period
    pub fn reset_period(&mut self) {
        self.snapshots_participated = 0;
        self.total_snapshots = 0;
        self.participation_rate = 1.0;
        self.avg_response_time_ms = 0;
        self.min_response_time_ms = u64::MAX;
        self.max_response_time_ms = 0;
        self.consecutive_failures = 0;
    }
}

/// Performance alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceAlert {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Alert type
    pub alert_type: AlertType,

    /// Alert message
    pub message: String,

    /// Severity level
    pub severity: AlertSeverity,

    /// Timestamp
    pub timestamp: u64,

    /// Metric value
    pub metric_value: f64,

    /// Threshold value
    pub threshold_value: f64,
}

/// Alert type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertType {
    /// Low participation rate
    LowParticipation,

    /// High response time
    HighResponseTime,

    /// Validator offline
    ValidatorOffline,

    /// Consecutive failures
    ConsecutiveFailures,

    /// Uptime degradation
    UptimeDegradation,
}

impl std::fmt::Display for AlertType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertType::LowParticipation => write!(f, "LowParticipation"),
            AlertType::HighResponseTime => write!(f, "HighResponseTime"),
            AlertType::ValidatorOffline => write!(f, "ValidatorOffline"),
            AlertType::ConsecutiveFailures => write!(f, "ConsecutiveFailures"),
            AlertType::UptimeDegradation => write!(f, "UptimeDegradation"),
        }
    }
}

/// Alert severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// Info level
    Info,

    /// Warning level
    Warning,

    /// Critical level
    Critical,
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Info => write!(f, "Info"),
            AlertSeverity::Warning => write!(f, "Warning"),
            AlertSeverity::Critical => write!(f, "Critical"),
        }
    }
}

/// Validator monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Participation rate threshold for warning
    pub participation_warning_threshold: f64,

    /// Participation rate threshold for critical
    pub participation_critical_threshold: f64,

    /// Response time threshold (milliseconds)
    pub response_time_threshold_ms: u64,

    /// Offline timeout (seconds)
    pub offline_timeout_secs: u64,

    /// Consecutive failures threshold
    pub consecutive_failures_threshold: u64,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            participation_warning_threshold: 0.95,  // 95%
            participation_critical_threshold: 0.90, // 90%
            response_time_threshold_ms: 1000,       // 1 second
            offline_timeout_secs: 300,              // 5 minutes
            consecutive_failures_threshold: 10,     // 10 failures
        }
    }
}

/// Validator monitor
pub struct ValidatorMonitor {
    /// Configuration
    config: MonitoringConfig,

    /// Validator metrics
    metrics: HashMap<ValidatorID, ValidatorMetrics>,

    /// Performance alerts
    alerts: Vec<PerformanceAlert>,

    /// Alert history
    alert_history: Vec<PerformanceAlert>,
}

impl ValidatorMonitor {
    /// Create new validator monitor
    pub fn new(config: MonitoringConfig) -> Self {
        Self {
            config,
            metrics: HashMap::new(),
            alerts: Vec::new(),
            alert_history: Vec::new(),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(MonitoringConfig::default())
    }

    /// Register validator for monitoring
    pub fn register_validator(&mut self, validator_id: ValidatorID) {
        let metrics = ValidatorMetrics::new(validator_id.clone());
        self.metrics.insert(validator_id, metrics);
    }

    /// Record snapshot participation
    pub fn record_snapshot(
        &mut self,
        validator_id: &ValidatorID,
        participated: bool,
        response_time_ms: u64,
    ) -> Result<()> {
        if let Some(metrics) = self.metrics.get_mut(validator_id) {
            metrics.record_snapshot(participated, response_time_ms);

            // Check for alerts
            self.check_alerts(validator_id);

            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Validator {} not registered for monitoring",
                validator_id
            )))
        }
    }

    /// Check for performance alerts
    fn check_alerts(&mut self, validator_id: &ValidatorID) {
        if let Some(metrics) = self.metrics.get(validator_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Check participation rate
            if metrics.participation_rate < self.config.participation_critical_threshold {
                let alert = PerformanceAlert {
                    validator_id: validator_id.clone(),
                    alert_type: AlertType::LowParticipation,
                    message: format!(
                        "Critical: Participation rate {:.2}% below threshold {:.2}%",
                        metrics.participation_rate * 100.0,
                        self.config.participation_critical_threshold * 100.0
                    ),
                    severity: AlertSeverity::Critical,
                    timestamp: now,
                    metric_value: metrics.participation_rate,
                    threshold_value: self.config.participation_critical_threshold,
                };
                self.alerts.push(alert.clone());
                self.alert_history.push(alert);
                warn!(
                    "Critical alert for validator {}: {}",
                    validator_id, "Low participation"
                );
            } else if metrics.participation_rate < self.config.participation_warning_threshold {
                let alert = PerformanceAlert {
                    validator_id: validator_id.clone(),
                    alert_type: AlertType::LowParticipation,
                    message: format!(
                        "Warning: Participation rate {:.2}% below threshold {:.2}%",
                        metrics.participation_rate * 100.0,
                        self.config.participation_warning_threshold * 100.0
                    ),
                    severity: AlertSeverity::Warning,
                    timestamp: now,
                    metric_value: metrics.participation_rate,
                    threshold_value: self.config.participation_warning_threshold,
                };
                self.alerts.push(alert.clone());
                self.alert_history.push(alert);
                debug!(
                    "Warning alert for validator {}: {}",
                    validator_id, "Low participation"
                );
            }

            // Check response time
            if metrics.avg_response_time_ms > self.config.response_time_threshold_ms {
                let alert = PerformanceAlert {
                    validator_id: validator_id.clone(),
                    alert_type: AlertType::HighResponseTime,
                    message: format!(
                        "Warning: Average response time {}ms exceeds threshold {}ms",
                        metrics.avg_response_time_ms, self.config.response_time_threshold_ms
                    ),
                    severity: AlertSeverity::Warning,
                    timestamp: now,
                    metric_value: metrics.avg_response_time_ms as f64,
                    threshold_value: self.config.response_time_threshold_ms as f64,
                };
                self.alerts.push(alert.clone());
                self.alert_history.push(alert);
                debug!(
                    "Warning alert for validator {}: {}",
                    validator_id, "High response time"
                );
            }

            // Check if offline
            if metrics.is_offline(self.config.offline_timeout_secs) {
                let alert = PerformanceAlert {
                    validator_id: validator_id.clone(),
                    alert_type: AlertType::ValidatorOffline,
                    message: format!(
                        "Critical: Validator offline for more than {} seconds",
                        self.config.offline_timeout_secs
                    ),
                    severity: AlertSeverity::Critical,
                    timestamp: now,
                    metric_value: (now - metrics.last_seen) as f64,
                    threshold_value: self.config.offline_timeout_secs as f64,
                };
                self.alerts.push(alert.clone());
                self.alert_history.push(alert);
                warn!(
                    "Critical alert for validator {}: {}",
                    validator_id, "Validator offline"
                );
            }

            // Check consecutive failures
            if metrics.consecutive_failures >= self.config.consecutive_failures_threshold {
                let alert = PerformanceAlert {
                    validator_id: validator_id.clone(),
                    alert_type: AlertType::ConsecutiveFailures,
                    message: format!(
                        "Warning: {} consecutive failures (threshold: {})",
                        metrics.consecutive_failures, self.config.consecutive_failures_threshold
                    ),
                    severity: AlertSeverity::Warning,
                    timestamp: now,
                    metric_value: metrics.consecutive_failures as f64,
                    threshold_value: self.config.consecutive_failures_threshold as f64,
                };
                self.alerts.push(alert.clone());
                self.alert_history.push(alert);
                debug!(
                    "Warning alert for validator {}: {}",
                    validator_id, "Consecutive failures"
                );
            }
        }
    }

    /// Get metrics for validator
    pub fn get_metrics(&self, validator_id: &ValidatorID) -> Option<ValidatorMetrics> {
        self.metrics.get(validator_id).cloned()
    }

    /// Get all metrics
    pub fn get_all_metrics(&self) -> Vec<ValidatorMetrics> {
        self.metrics.values().cloned().collect()
    }

    /// Get active alerts
    pub fn get_active_alerts(&self) -> Vec<PerformanceAlert> {
        self.alerts.clone()
    }

    /// Get alerts for specific validator
    pub fn get_validator_alerts(&self, validator_id: &ValidatorID) -> Vec<PerformanceAlert> {
        self.alerts
            .iter()
            .filter(|a| &a.validator_id == validator_id)
            .cloned()
            .collect()
    }

    /// Clear active alerts
    pub fn clear_alerts(&mut self) {
        self.alerts.clear();
    }

    /// Get alert history
    pub fn get_alert_history(&self) -> Vec<PerformanceAlert> {
        self.alert_history.clone()
    }

    /// Get alert history for validator
    pub fn get_validator_alert_history(&self, validator_id: &ValidatorID) -> Vec<PerformanceAlert> {
        self.alert_history
            .iter()
            .filter(|a| &a.validator_id == validator_id)
            .cloned()
            .collect()
    }

    /// Get validators with critical alerts
    pub fn get_critical_validators(&self) -> Vec<ValidatorID> {
        self.alerts
            .iter()
            .filter(|a| a.severity == AlertSeverity::Critical)
            .map(|a| a.validator_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Get validators with warning alerts
    pub fn get_warning_validators(&self) -> Vec<ValidatorID> {
        self.alerts
            .iter()
            .filter(|a| a.severity == AlertSeverity::Warning)
            .map(|a| a.validator_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Update monitoring configuration
    pub fn update_config(&mut self, config: MonitoringConfig) {
        self.config = config;
        info!("Monitoring configuration updated");
    }

    /// Get monitoring configuration
    pub fn get_config(&self) -> &MonitoringConfig {
        &self.config
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.metrics.len()
    }

    /// Get average participation rate
    pub fn get_average_participation(&self) -> f64 {
        if self.metrics.is_empty() {
            return 0.0;
        }

        let sum: f64 = self.metrics.values().map(|m| m.participation_rate).sum();
        sum / self.metrics.len() as f64
    }

    /// Get average uptime
    pub fn get_average_uptime(&self) -> f64 {
        if self.metrics.is_empty() {
            return 0.0;
        }

        let sum: f64 = self.metrics.values().map(|m| m.uptime_percentage).sum();
        sum / self.metrics.len() as f64
    }

    /// Get average response time
    pub fn get_average_response_time(&self) -> u64 {
        if self.metrics.is_empty() {
            return 0;
        }

        let sum: u64 = self.metrics.values().map(|m| m.avg_response_time_ms).sum();
        sum / self.metrics.len() as u64
    }

    /// Reset period for all validators
    pub fn reset_period(&mut self) {
        for metrics in self.metrics.values_mut() {
            metrics.reset_period();
        }
        self.alerts.clear();
        info!("Monitoring period reset for all validators");
    }

    /// Get health status
    pub fn get_health_status(&self) -> HealthStatus {
        let critical_count = self.get_critical_validators().len();
        let warning_count = self.get_warning_validators().len();
        let total_count = self.validator_count();

        let status = if critical_count > 0 {
            Status::Critical
        } else if warning_count > 0 {
            Status::Warning
        } else {
            Status::Healthy
        };

        HealthStatus {
            status,
            total_validators: total_count,
            critical_validators: critical_count,
            warning_validators: warning_count,
            average_participation: self.get_average_participation(),
            average_uptime: self.get_average_uptime(),
            average_response_time_ms: self.get_average_response_time(),
        }
    }
}

/// Health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Overall status
    pub status: Status,

    /// Total validators
    pub total_validators: usize,

    /// Critical validators
    pub critical_validators: usize,

    /// Warning validators
    pub warning_validators: usize,

    /// Average participation rate
    pub average_participation: f64,

    /// Average uptime
    pub average_uptime: f64,

    /// Average response time
    pub average_response_time_ms: u64,
}

/// Overall status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Healthy
    Healthy,

    /// Warning
    Warning,

    /// Critical
    Critical,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Healthy => write!(f, "Healthy"),
            Status::Warning => write!(f, "Warning"),
            Status::Critical => write!(f, "Critical"),
        }
    }
}
