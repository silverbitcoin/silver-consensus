//! Persistence layer for SilverBitcoin blockchain
//!
//! This module provides comprehensive persistence functionality for the blockchain,
//! including:
//! - Genesis block initialization and persistence
//! - Snapshot persistence and recovery
//! - Validator state persistence
//! - Transaction persistence with snapshot linkage
//! - Atomic batch operations
//! - Flush and durability guarantees
//! - State recovery and verification
//! - Graceful shutdown
//! - Comprehensive logging and monitoring
//! - Consensus-storage integration

pub mod consensus_integration;
pub mod logging;
pub mod node_startup;

pub use consensus_integration::{ConsensusStorageIntegration, IntegrationStatus};
pub use logging::{
    OperationStatus, OperationType, PersistenceLogEntry, PersistenceLogger,
};
pub use node_startup::{NodeStartupManager, StartupConfig, StartupResult};
