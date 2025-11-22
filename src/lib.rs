//! # SilverBitcoin Consensus
//!
//! Mercury Protocol consensus engine with Cascade mempool.
//!
//! This crate implements:
//! - Cascade mempool (graph-flow transaction ordering)
//! - Mercury Protocol (DRP consensus algorithm)
//! - Validator set management
//! - Snapshot creation and certification
//! - Byzantine fault tolerance (up to 1/3 malicious validators)

#![warn(missing_docs, rust_2018_idioms)]
#![forbid(unsafe_code)]
#![allow(missing_docs)] // Internal implementation details

pub mod cascade;
pub mod mercury;
pub mod validator;
pub mod validator_keys;
pub mod validator_tiers;
pub mod staking;
pub mod delegation;
pub mod commission;
pub mod validator_ops;
pub mod snapshot;
pub mod flow_graph;
pub mod rewards;
pub mod upgrade;
pub mod activation;
pub mod compatibility;
pub mod optimizations; // OPTIMIZATION: Consensus optimizations (Task 35.4)
pub mod slashing;
pub mod downtime_tracker;
pub mod validator_activation;
pub mod key_rotation;
pub mod commission_manager;
pub mod validator_monitoring;

pub use cascade::CascadeMempool;
pub use mercury::MercuryProtocol;
pub use validator::{ValidatorSet, ValidatorInfo, ValidatorSetChangeEvent};
pub use validator_keys::{
    ValidatorKeyManager, ValidatorKeySet, ValidatorPrivateKey,
    EncryptedValidatorKeys, KeyRotationRecord,
};
pub use validator_tiers::{
    ValidatorTier, ValidatorTierInfo, ValidatorTierManager,
    TierChangeEvent,
};
pub use staking::{
    StakingManager, ValidatorStake, StakeDeposit, UnstakingRequest,
    MIN_STAKE_AMOUNT, UNBONDING_PERIOD_SECS,
};
pub use delegation::{
    DelegationManager, Delegation, UndelegationRequest, ValidatorDelegationInfo,
    MIN_DELEGATION_AMOUNT, MAX_DELEGATED_STAKE_PER_VALIDATOR,
};
pub use commission::{
    CommissionManager, CommissionRate, CommissionRateChange, ValidatorCommissionInfo,
    MIN_COMMISSION_RATE, MAX_COMMISSION_RATE, COMMISSION_CHANGE_NOTICE_PERIOD,
};
pub use validator_ops::{ValidatorOperations, DOWNTIME_THRESHOLD};
pub use snapshot::{SnapshotManager, SnapshotCertificate, ValidatorSignatureInfo};
pub use flow_graph::FlowGraph;
pub use rewards::{
    FuelFeeCollector, RewardDistributor, CycleRewardsManager,
    ValidatorReward, TransactionFee,
};
pub use upgrade::{UpgradeManager, UpgradeStats};
pub use activation::{ActivationCoordinator, ActivationStats};
pub use compatibility::{CompatibilityChecker, CompatibilityStats, FeatureExtractor};
pub use optimizations::{
    BatchPipeline, FlowGraphCache, SnapshotOptimizer,
    PipelineStats, CacheStats, SnapshotStats,
}; // OPTIMIZATION exports
pub use slashing::{
    SlashingManager, SlashingConfig, JailRecord, JailReason, SlashEvent,
};
pub use downtime_tracker::{
    DowntimeTracker, DowntimeConfig, ValidatorDowntimeRecord, DowntimeEvent, DowntimeEventType,
};
pub use validator_activation::{
    ValidatorActivationManager, ActivationConfig, ValidatorActivationRecord, ActivationRequest,
    ValidatorStatus, ActivationEvent, ActivationEventType, ActivationRequestType, ActivationRequestStatus,
};
pub use key_rotation::{
    KeyRotationManager, KeyRotationConfig, KeyRecord, KeyStatus, KeyRotationRequest,
    KeyRotationRequestStatus, KeyRotationEvent, KeyRotationEventType,
};
pub use commission_manager::{
    CommissionManager, CommissionConfig, CommissionRate, CommissionChangeRequest,
    ValidatorCommissionInfo,
};
pub use validator_monitoring::{
    ValidatorMonitor, ValidatorMetrics, PerformanceAlert, AlertType, AlertSeverity,
    MonitoringConfig, HealthStatus, Status,
};
