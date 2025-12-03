//! Comprehensive logging for persistence operations
//!
//! This module provides structured logging for all persistence layer operations including:
//! - Persistence operations (snapshots, validator state, transactions)
//! - Flush operations (column family flush, WAL flush, fsync)
//! - Recovery operations (state recovery, consistency verification)
//! - Error logging with context
//! - Startup information logging

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use tracing::{debug, error, info, warn};

/// Operation type for structured logging
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationType {
    /// Genesis block persistence
    PersistGenesisBlock,
    /// Snapshot persistence
    PersistSnapshot,
    /// Finalized snapshot persistence
    PersistFinalizedSnapshot,
    /// Validator state persistence
    PersistValidatorState,
    /// Staking record persistence
    PersistStakingRecord,
    /// Tier change persistence
    PersistTierChange,
    /// Unstaking request persistence
    PersistUnstakingRequest,
    /// Reward record persistence
    PersistRewardRecord,
    /// Transaction persistence
    PersistTransaction,
    /// Batch transaction persistence
    PersistTransactionBatch,
    /// Atomic batch write
    AtomicBatchWrite,
    /// Column family flush
    FlushColumnFamily,
    /// All column families flush
    FlushAllColumnFamilies,
    /// Write-ahead log flush
    FlushWAL,
    /// Fsync operation
    Fsync,
    /// Database integrity verification
    VerifyDatabaseIntegrity,
    /// Genesis block recovery
    RecoverGenesisBlock,
    /// Snapshots recovery
    RecoverSnapshots,
    /// Validator state recovery
    RecoverValidatorState,
    /// Transactions recovery
    RecoverTransactions,
    /// Consistency verification
    VerifyConsistency,
    /// Graceful shutdown
    GracefulShutdown,
    /// Node startup
    NodeStartup,
}

impl fmt::Display for OperationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationType::PersistGenesisBlock => write!(f, "PersistGenesisBlock"),
            OperationType::PersistSnapshot => write!(f, "PersistSnapshot"),
            OperationType::PersistFinalizedSnapshot => write!(f, "PersistFinalizedSnapshot"),
            OperationType::PersistValidatorState => write!(f, "PersistValidatorState"),
            OperationType::PersistStakingRecord => write!(f, "PersistStakingRecord"),
            OperationType::PersistTierChange => write!(f, "PersistTierChange"),
            OperationType::PersistUnstakingRequest => write!(f, "PersistUnstakingRequest"),
            OperationType::PersistRewardRecord => write!(f, "PersistRewardRecord"),
            OperationType::PersistTransaction => write!(f, "PersistTransaction"),
            OperationType::PersistTransactionBatch => write!(f, "PersistTransactionBatch"),
            OperationType::AtomicBatchWrite => write!(f, "AtomicBatchWrite"),
            OperationType::FlushColumnFamily => write!(f, "FlushColumnFamily"),
            OperationType::FlushAllColumnFamilies => write!(f, "FlushAllColumnFamilies"),
            OperationType::FlushWAL => write!(f, "FlushWAL"),
            OperationType::Fsync => write!(f, "Fsync"),
            OperationType::VerifyDatabaseIntegrity => write!(f, "VerifyDatabaseIntegrity"),
            OperationType::RecoverGenesisBlock => write!(f, "RecoverGenesisBlock"),
            OperationType::RecoverSnapshots => write!(f, "RecoverSnapshots"),
            OperationType::RecoverValidatorState => write!(f, "RecoverValidatorState"),
            OperationType::RecoverTransactions => write!(f, "RecoverTransactions"),
            OperationType::VerifyConsistency => write!(f, "VerifyConsistency"),
            OperationType::GracefulShutdown => write!(f, "GracefulShutdown"),
            OperationType::NodeStartup => write!(f, "NodeStartup"),
        }
    }
}

/// Operation status for structured logging
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationStatus {
    /// Operation started
    Started,
    /// Operation in progress
    InProgress,
    /// Operation completed successfully
    Success,
    /// Operation failed
    Failed,
    /// Operation retrying
    Retrying,
}

impl fmt::Display for OperationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationStatus::Started => write!(f, "Started"),
            OperationStatus::InProgress => write!(f, "InProgress"),
            OperationStatus::Success => write!(f, "Success"),
            OperationStatus::Failed => write!(f, "Failed"),
            OperationStatus::Retrying => write!(f, "Retrying"),
        }
    }
}

/// Structured log entry for persistence operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceLogEntry {
    /// Operation type
    pub operation_type: OperationType,
    /// Operation status
    pub status: OperationStatus,
    /// Data size in bytes
    pub data_size_bytes: u64,
    /// Timestamp of operation
    pub timestamp: DateTime<Utc>,
    /// Duration in milliseconds (if completed)
    pub duration_ms: Option<u64>,
    /// Additional context
    pub context: String,
    /// Error message (if failed)
    pub error_message: Option<String>,
}

impl fmt::Display for PersistenceLogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "operation={} status={} data_size={} bytes timestamp={} context={}",
            self.operation_type, self.status, self.data_size_bytes, self.timestamp, self.context
        )?;
        if let Some(duration) = self.duration_ms {
            write!(f, " duration={}ms", duration)?;
        }
        if let Some(error) = &self.error_message {
            write!(f, " error={}", error)?;
        }
        Ok(())
    }
}

/// Logger for persistence operations
pub struct PersistenceLogger;

impl PersistenceLogger {
    /// Log a persistence operation start
    pub fn log_operation_start(
        operation_type: OperationType,
        data_size_bytes: u64,
        context: impl Into<String>,
    ) {
        let context = context.into();
        let entry = PersistenceLogEntry {
            operation_type,
            status: OperationStatus::Started,
            data_size_bytes,
            timestamp: Utc::now(),
            duration_ms: None,
            context: context.clone(),
            error_message: None,
        };

        info!(
            operation = %operation_type,
            data_size = data_size_bytes,
            context = %context,
            "Persistence operation started"
        );

        debug!("Persistence log entry: {}", entry);
    }

    /// Log a persistence operation success
    pub fn log_operation_success(
        operation_type: OperationType,
        data_size_bytes: u64,
        duration_ms: u64,
        context: impl Into<String>,
    ) {
        let context = context.into();
        let entry = PersistenceLogEntry {
            operation_type,
            status: OperationStatus::Success,
            data_size_bytes,
            timestamp: Utc::now(),
            duration_ms: Some(duration_ms),
            context: context.clone(),
            error_message: None,
        };

        info!(
            operation = %operation_type,
            data_size = data_size_bytes,
            duration_ms = duration_ms,
            context = %context,
            "Persistence operation completed successfully"
        );

        debug!("Persistence log entry: {}", entry);
    }

    /// Log a persistence operation failure
    pub fn log_operation_failure(
        operation_type: OperationType,
        data_size_bytes: u64,
        duration_ms: u64,
        context: impl Into<String>,
        error_message: impl Into<String>,
    ) {
        let context = context.into();
        let error_message = error_message.into();
        let entry = PersistenceLogEntry {
            operation_type,
            status: OperationStatus::Failed,
            data_size_bytes,
            timestamp: Utc::now(),
            duration_ms: Some(duration_ms),
            context: context.clone(),
            error_message: Some(error_message.clone()),
        };

        error!(
            operation = %operation_type,
            data_size = data_size_bytes,
            duration_ms = duration_ms,
            context = %context,
            error = %error_message,
            "Persistence operation failed"
        );

        debug!("Persistence log entry: {}", entry);
    }

    /// Log a persistence operation retry
    pub fn log_operation_retry(
        operation_type: OperationType,
        data_size_bytes: u64,
        attempt: u32,
        context: impl Into<String>,
        reason: impl Into<String>,
    ) {
        let context = context.into();
        let reason = reason.into();

        warn!(
            operation = %operation_type,
            data_size = data_size_bytes,
            attempt = attempt,
            context = %context,
            reason = %reason,
            "Persistence operation retrying"
        );
    }

    /// Log a flush operation start
    pub fn log_flush_start(column_family: impl Into<String>) {
        let cf = column_family.into();
        info!(
            column_family = %cf,
            "Flush operation started"
        );
    }

    /// Log a flush operation completion
    pub fn log_flush_completion(
        column_family: impl Into<String>,
        duration_ms: u64,
        data_flushed_bytes: u64,
    ) {
        let cf = column_family.into();
        info!(
            column_family = %cf,
            duration_ms = duration_ms,
            data_flushed = data_flushed_bytes,
            "Flush operation completed"
        );
    }

    /// Log a flush operation failure
    pub fn log_flush_failure(
        column_family: impl Into<String>,
        duration_ms: u64,
        error_message: impl Into<String>,
    ) {
        let cf = column_family.into();
        let error = error_message.into();
        error!(
            column_family = %cf,
            duration_ms = duration_ms,
            error = %error,
            "Flush operation failed"
        );
    }

    /// Log recovery operation start
    pub fn log_recovery_start(recovery_type: impl Into<String>) {
        let recovery = recovery_type.into();
        info!(
            recovery_type = %recovery,
            "Recovery operation started"
        );
    }

    /// Log recovery operation progress
    pub fn log_recovery_progress(
        recovery_type: impl Into<String>,
        items_recovered: u64,
        context: impl Into<String>,
    ) {
        let recovery = recovery_type.into();
        let ctx = context.into();
        info!(
            recovery_type = %recovery,
            items_recovered = items_recovered,
            context = %ctx,
            "Recovery operation in progress"
        );
    }

    /// Log recovery operation completion
    pub fn log_recovery_completion(
        recovery_type: impl Into<String>,
        items_recovered: u64,
        duration_ms: u64,
    ) {
        let recovery = recovery_type.into();
        info!(
            recovery_type = %recovery,
            items_recovered = items_recovered,
            duration_ms = duration_ms,
            "Recovery operation completed"
        );
    }

    /// Log consistency check start
    pub fn log_consistency_check_start(check_type: impl Into<String>) {
        let check = check_type.into();
        info!(
            check_type = %check,
            "Consistency check started"
        );
    }

    /// Log consistency check result
    pub fn log_consistency_check_result(
        check_type: impl Into<String>,
        is_consistent: bool,
        details: impl Into<String>,
    ) {
        let check = check_type.into();
        let detail = details.into();

        if is_consistent {
            info!(
                check_type = %check,
                details = %detail,
                "Consistency check passed"
            );
        } else {
            error!(
                check_type = %check,
                details = %detail,
                "Consistency check failed"
            );
        }
    }

    /// Log error with context
    pub fn log_error(
        error_type: impl Into<String>,
        error_message: impl Into<String>,
        context: impl Into<String>,
    ) {
        let err_type = error_type.into();
        let err_msg = error_message.into();
        let ctx = context.into();

        error!(
            error_type = %err_type,
            error = %err_msg,
            context = %ctx,
            "Persistence error occurred"
        );
    }

    /// Log error with recovery action
    pub fn log_error_with_recovery(
        error_type: impl Into<String>,
        error_message: impl Into<String>,
        recovery_action: impl Into<String>,
    ) {
        let err_type = error_type.into();
        let err_msg = error_message.into();
        let recovery = recovery_action.into();

        error!(
            error_type = %err_type,
            error = %err_msg,
            recovery_action = %recovery,
            "Persistence error occurred, attempting recovery"
        );
    }

    /// Log startup information
    pub fn log_startup_info(
        genesis_block_hash: impl Into<String>,
        latest_snapshot_sequence: u64,
        latest_block_number: u64,
        validator_count: u64,
        total_stake: u128,
    ) {
        let genesis_hash = genesis_block_hash.into();

        info!(
            genesis_block_hash = %genesis_hash,
            latest_snapshot_sequence = latest_snapshot_sequence,
            latest_block_number = latest_block_number,
            validator_count = validator_count,
            total_stake = total_stake,
            "Node startup information"
        );
    }

    /// Log database initialization
    pub fn log_database_initialized(
        database_path: impl Into<String>,
        column_families: u32,
    ) {
        let path = database_path.into();
        info!(
            database_path = %path,
            column_families = column_families,
            "Database initialized successfully"
        );
    }

    /// Log database initialization failure
    pub fn log_database_initialization_failure(
        database_path: impl Into<String>,
        error_message: impl Into<String>,
    ) {
        let path = database_path.into();
        let error = error_message.into();
        error!(
            database_path = %path,
            error = %error,
            "Database initialization failed"
        );
    }

    /// Log graceful shutdown start
    pub fn log_shutdown_start() {
        info!("Graceful shutdown initiated");
    }

    /// Log graceful shutdown progress
    pub fn log_shutdown_progress(step: impl Into<String>) {
        let s = step.into();
        info!(
            step = %s,
            "Graceful shutdown in progress"
        );
    }

    /// Log graceful shutdown completion
    pub fn log_shutdown_completion(duration_ms: u64) {
        info!(
            duration_ms = duration_ms,
            "Graceful shutdown completed"
        );
    }

    /// Log graceful shutdown timeout
    pub fn log_shutdown_timeout(timeout_secs: u64) {
        warn!(
            timeout_secs = timeout_secs,
            "Graceful shutdown timeout exceeded, forcing shutdown"
        );
    }

    /// Log state snapshot information
    pub fn log_snapshot_info(
        sequence_number: u64,
        transaction_count: u64,
        validator_count: u64,
        state_root: impl Into<String>,
    ) {
        let root = state_root.into();
        info!(
            sequence_number = sequence_number,
            transaction_count = transaction_count,
            validator_count = validator_count,
            state_root = %root,
            "Snapshot information"
        );
    }

    /// Log validator state information
    pub fn log_validator_state_info(
        validator_id: impl Into<String>,
        stake: u128,
        tier: impl Into<String>,
        deposit_count: u64,
    ) {
        let id = validator_id.into();
        let t = tier.into();
        info!(
            validator_id = %id,
            stake = stake,
            tier = %t,
            deposit_count = deposit_count,
            "Validator state information"
        );
    }

    /// Log transaction information
    pub fn log_transaction_info(
        transaction_digest: impl Into<String>,
        sender: impl Into<String>,
        recipient: impl Into<String>,
        amount: u128,
        fuel_used: u64,
    ) {
        let digest = transaction_digest.into();
        let s = sender.into();
        let r = recipient.into();
        info!(
            transaction_digest = %digest,
            sender = %s,
            recipient = %r,
            amount = amount,
            fuel_used = fuel_used,
            "Transaction information"
        );
    }

    /// Log batch operation information
    pub fn log_batch_operation_info(
        batch_size: u64,
        total_data_size: u64,
        operation_type: impl Into<String>,
    ) {
        let op_type = operation_type.into();
        info!(
            batch_size = batch_size,
            total_data_size = total_data_size,
            operation_type = %op_type,
            "Batch operation information"
        );
    }

    /// Log performance metric
    pub fn log_performance_metric(
        metric_name: impl Into<String>,
        value: f64,
        unit: impl Into<String>,
    ) {
        let name = metric_name.into();
        let u = unit.into();
        debug!(
            metric_name = %name,
            value = value,
            unit = %u,
            "Performance metric"
        );
    }

    /// Log database statistics
    pub fn log_database_stats(
        total_size_bytes: u64,
        live_data_size_bytes: u64,
        column_family_count: u32,
    ) {
        info!(
            total_size = total_size_bytes,
            live_data_size = live_data_size_bytes,
            column_family_count = column_family_count,
            "Database statistics"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_type_display() {
        assert_eq!(OperationType::PersistGenesisBlock.to_string(), "PersistGenesisBlock");
        assert_eq!(OperationType::PersistSnapshot.to_string(), "PersistSnapshot");
        assert_eq!(OperationType::FlushColumnFamily.to_string(), "FlushColumnFamily");
        assert_eq!(OperationType::RecoverGenesisBlock.to_string(), "RecoverGenesisBlock");
    }

    #[test]
    fn test_operation_status_display() {
        assert_eq!(OperationStatus::Started.to_string(), "Started");
        assert_eq!(OperationStatus::Success.to_string(), "Success");
        assert_eq!(OperationStatus::Failed.to_string(), "Failed");
        assert_eq!(OperationStatus::Retrying.to_string(), "Retrying");
    }

    #[test]
    fn test_persistence_log_entry_display() {
        let entry = PersistenceLogEntry {
            operation_type: OperationType::PersistSnapshot,
            status: OperationStatus::Success,
            data_size_bytes: 1024,
            timestamp: Utc::now(),
            duration_ms: Some(100),
            context: "test_context".to_string(),
            error_message: None,
        };

        let display_str = entry.to_string();
        assert!(display_str.contains("PersistSnapshot"));
        assert!(display_str.contains("Success"));
        assert!(display_str.contains("1024"));
        assert!(display_str.contains("100ms"));
    }

    #[test]
    fn test_persistence_log_entry_with_error() {
        let entry = PersistenceLogEntry {
            operation_type: OperationType::PersistSnapshot,
            status: OperationStatus::Failed,
            data_size_bytes: 512,
            timestamp: Utc::now(),
            duration_ms: Some(50),
            context: "test_context".to_string(),
            error_message: Some("Database write failed".to_string()),
        };

        let display_str = entry.to_string();
        assert!(display_str.contains("Failed"));
        assert!(display_str.contains("Database write failed"));
    }

    #[test]
    fn test_logger_methods_compile() {
        // These tests verify that all logger methods compile and can be called
        PersistenceLogger::log_operation_start(
            OperationType::PersistGenesisBlock,
            1024,
            "test",
        );

        PersistenceLogger::log_operation_success(
            OperationType::PersistGenesisBlock,
            1024,
            100,
            "test",
        );

        PersistenceLogger::log_operation_failure(
            OperationType::PersistGenesisBlock,
            1024,
            100,
            "test",
            "error",
        );

        PersistenceLogger::log_operation_retry(
            OperationType::PersistGenesisBlock,
            1024,
            1,
            "test",
            "reason",
        );

        PersistenceLogger::log_flush_start("CF_BLOCKS");
        PersistenceLogger::log_flush_completion("CF_BLOCKS", 100, 2048);
        PersistenceLogger::log_flush_failure("CF_BLOCKS", 100, "error");

        PersistenceLogger::log_recovery_start("genesis");
        PersistenceLogger::log_recovery_progress("genesis", 1, "context");
        PersistenceLogger::log_recovery_completion("genesis", 1, 100);

        PersistenceLogger::log_consistency_check_start("snapshot_sequence");
        PersistenceLogger::log_consistency_check_result("snapshot_sequence", true, "all valid");

        PersistenceLogger::log_error("test_error", "message", "context");
        PersistenceLogger::log_error_with_recovery("test_error", "message", "recovery");

        PersistenceLogger::log_startup_info("abc123", 100, 50, 10, 1000000);
        PersistenceLogger::log_database_initialized("/path/to/db", 7);
        PersistenceLogger::log_database_initialization_failure("/path/to/db", "error");

        PersistenceLogger::log_shutdown_start();
        PersistenceLogger::log_shutdown_progress("flushing");
        PersistenceLogger::log_shutdown_completion(1000);
        PersistenceLogger::log_shutdown_timeout(30);

        PersistenceLogger::log_snapshot_info(1, 100, 10, "root_hash");
        PersistenceLogger::log_validator_state_info("validator1", 1000000, "Gold", 5);
        PersistenceLogger::log_transaction_info("tx_digest", "sender", "recipient", 100, 50);
        PersistenceLogger::log_batch_operation_info(10, 10240, "persist");
        PersistenceLogger::log_performance_metric("throughput", 1000.5, "ops/sec");
        PersistenceLogger::log_database_stats(1024000, 512000, 7);
    }
}
