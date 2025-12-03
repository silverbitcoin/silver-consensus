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

pub mod activation;
pub mod block_creator;
pub mod cascade;
pub mod commission;
pub mod commission_manager;
pub mod compatibility;
pub mod delegation;
pub mod downtime_tracker;
pub mod flow_graph;
pub mod key_rotation;
pub mod mercury;
pub mod optimizations; // OPTIMIZATION: Consensus optimizations (Task 35.4)
pub mod persistence;
pub mod rewards;
pub mod slashing;
pub mod snapshot;
pub mod staking;
pub mod upgrade;
pub mod validator;
pub mod validator_activation;
pub mod validator_keys;
pub mod validator_monitoring;
pub mod validator_ops;
pub mod validator_tiers;

pub use activation::{ActivationCoordinator, ActivationStats};
pub use block_creator::BlockCreator;
pub use cascade::CascadeMempool;
pub use commission::{
    CommissionManager, CommissionRate, CommissionRateChange, ValidatorCommissionInfo,
    COMMISSION_CHANGE_NOTICE_PERIOD, MAX_COMMISSION_RATE, MIN_COMMISSION_RATE,
};
pub use commission_manager::{CommissionChangeRequest, CommissionConfig};
pub use compatibility::{CompatibilityChecker, CompatibilityStats, FeatureExtractor};
pub use delegation::{
    Delegation, DelegationManager, UndelegationRequest, ValidatorDelegationInfo,
    MAX_DELEGATED_STAKE_PER_VALIDATOR, MIN_DELEGATION_AMOUNT,
};
pub use downtime_tracker::{
    DowntimeConfig, DowntimeEvent, DowntimeEventType, DowntimeTracker, ValidatorDowntimeRecord,
};
pub use flow_graph::FlowGraph;
pub use key_rotation::{
    KeyRecord, KeyRotationConfig, KeyRotationEvent, KeyRotationEventType, KeyRotationManager,
    KeyRotationRequest, KeyRotationRequestStatus, KeyStatus,
};
pub use mercury::MercuryProtocol;
pub use optimizations::{
    BatchPipeline, CacheStats, FlowGraphCache, PipelineStats, SnapshotOptimizer, SnapshotStats,
}; // OPTIMIZATION exports
pub use persistence::{
    OperationStatus, OperationType, PersistenceLogEntry, PersistenceLogger,
};
pub use rewards::{
    CycleRewardsManager, FuelFeeCollector, RewardDistributor, TransactionFee, ValidatorReward,
};
pub use slashing::{JailReason, JailRecord, SlashEvent, SlashingConfig, SlashingManager};
pub use snapshot::{SnapshotCertificate, SnapshotManager, ValidatorSignatureInfo};
pub use staking::{
    StakeDeposit, StakingManager, UnstakingRequest, ValidatorStake, MIN_STAKE_AMOUNT,
    UNBONDING_PERIOD_SECS,
};
pub use upgrade::{UpgradeManager, UpgradeStats};
pub use validator::{ValidatorInfo, ValidatorSet, ValidatorSetChangeEvent};
pub use validator_activation::{
    ActivationConfig, ActivationEvent, ActivationEventType, ActivationRequest,
    ActivationRequestStatus, ActivationRequestType, ValidatorActivationManager,
    ValidatorActivationRecord, ValidatorStatus,
};
pub use validator_keys::{
    EncryptedValidatorKeys, KeyRotationRecord, ValidatorKeyManager, ValidatorKeySet,
    ValidatorPrivateKey,
};
pub use validator_monitoring::{
    AlertSeverity, AlertType, HealthStatus, MonitoringConfig, PerformanceAlert, Status,
    ValidatorMetrics, ValidatorMonitor,
};
pub use validator_ops::{ValidatorOperations, DOWNTIME_THRESHOLD};
pub use validator_tiers::{
    TierChangeEvent, ValidatorTier, ValidatorTierInfo, ValidatorTierManager,
};
