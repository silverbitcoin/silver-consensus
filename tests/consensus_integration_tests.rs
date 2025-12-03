//! Integration tests for consensus-storage integration
//!
//! Tests the integration between the consensus layer and the storage layer,
//! verifying that all callbacks are properly connected and the system is ready
//! for operation.

use silver_consensus::persistence::ConsensusStorageIntegration;

#[test]
fn test_consensus_storage_integration_initialization() {
    let integration = ConsensusStorageIntegration::new();
    let status = integration.get_status();

    // Initially, nothing should be initialized
    assert!(!status.storage_initialized);
    assert!(!status.snapshot_callback_connected);
    assert!(!status.finalization_callback_connected);
    assert!(!status.validator_state_callback_connected);
    assert!(!status.shutdown_callback_connected);
    assert_eq!(status.snapshots_persisted, 0);
    assert_eq!(status.validator_updates_persisted, 0);
    assert_eq!(status.transactions_persisted, 0);
    assert!(!status.all_connected());
}

#[test]
fn test_consensus_storage_integration_full_setup() {
    let integration = ConsensusStorageIntegration::new();

    // Initialize storage
    assert!(integration.initialize_storage().is_ok());
    let status = integration.get_status();
    assert!(status.storage_initialized);

    // Register all callbacks
    assert!(integration.register_snapshot_creation_callback().is_ok());
    assert!(integration.register_snapshot_finalization_callback().is_ok());
    assert!(integration.register_validator_state_callback().is_ok());
    assert!(integration.register_shutdown_callback().is_ok());

    // Verify all connections
    assert!(integration.verify_all_connections().is_ok());

    let status = integration.get_status();
    assert!(status.all_connected());
}

#[test]
fn test_consensus_storage_integration_persistence_tracking() {
    let integration = ConsensusStorageIntegration::new();

    // Track snapshot persistence
    assert_eq!(integration.snapshots_persisted(), 0);
    integration.record_snapshot_persisted();
    assert_eq!(integration.snapshots_persisted(), 1);
    integration.record_snapshot_persisted();
    assert_eq!(integration.snapshots_persisted(), 2);

    // Track validator updates
    assert_eq!(integration.validator_updates_persisted(), 0);
    integration.record_validator_update_persisted();
    assert_eq!(integration.validator_updates_persisted(), 1);
    integration.record_validator_update_persisted();
    assert_eq!(integration.validator_updates_persisted(), 2);

    // Track transactions
    assert_eq!(integration.transactions_persisted(), 0);
    integration.record_transactions_persisted(10);
    assert_eq!(integration.transactions_persisted(), 10);
    integration.record_transactions_persisted(5);
    assert_eq!(integration.transactions_persisted(), 15);
}

#[test]
fn test_consensus_storage_integration_callback_order() {
    let integration = ConsensusStorageIntegration::new();

    // Should fail to register callbacks before storage init
    assert!(integration.register_snapshot_creation_callback().is_err());
    assert!(integration.register_snapshot_finalization_callback().is_err());
    assert!(integration.register_validator_state_callback().is_err());
    assert!(integration.register_shutdown_callback().is_err());

    // Initialize storage
    assert!(integration.initialize_storage().is_ok());

    // Now callbacks should succeed
    assert!(integration.register_snapshot_creation_callback().is_ok());
    assert!(integration.register_snapshot_finalization_callback().is_ok());
    assert!(integration.register_validator_state_callback().is_ok());
    assert!(integration.register_shutdown_callback().is_ok());
}

#[test]
fn test_consensus_storage_integration_verification_incomplete() {
    let integration = ConsensusStorageIntegration::new();

    // Initialize storage
    assert!(integration.initialize_storage().is_ok());

    // Register only some callbacks
    assert!(integration.register_snapshot_creation_callback().is_ok());
    assert!(integration.register_snapshot_finalization_callback().is_ok());

    // Verification should fail because not all callbacks are registered
    assert!(integration.verify_all_connections().is_err());
}

#[test]
fn test_consensus_storage_integration_verification_complete() {
    let integration = ConsensusStorageIntegration::new();

    // Initialize storage
    assert!(integration.initialize_storage().is_ok());

    // Register all callbacks
    assert!(integration.register_snapshot_creation_callback().is_ok());
    assert!(integration.register_snapshot_finalization_callback().is_ok());
    assert!(integration.register_validator_state_callback().is_ok());
    assert!(integration.register_shutdown_callback().is_ok());

    // Verification should succeed
    assert!(integration.verify_all_connections().is_ok());

    let status = integration.get_status();
    assert!(status.all_connected());
}

#[test]
fn test_consensus_storage_integration_status_logging() {
    let integration = ConsensusStorageIntegration::new();

    // Initialize and register all callbacks
    assert!(integration.initialize_storage().is_ok());
    assert!(integration.register_snapshot_creation_callback().is_ok());
    assert!(integration.register_snapshot_finalization_callback().is_ok());
    assert!(integration.register_validator_state_callback().is_ok());
    assert!(integration.register_shutdown_callback().is_ok());

    // Log status (should not panic)
    integration.log_status();

    // Verify status
    let status = integration.get_status();
    assert!(status.all_connected());
}

#[test]
fn test_consensus_storage_integration_multiple_instances() {
    let integration1 = ConsensusStorageIntegration::new();
    let integration2 = ConsensusStorageIntegration::new();

    // Each instance should be independent
    assert!(integration1.initialize_storage().is_ok());
    let status1 = integration1.get_status();
    assert!(status1.storage_initialized);

    let status2 = integration2.get_status();
    assert!(!status2.storage_initialized);
}

#[test]
fn test_consensus_storage_integration_persistence_counters() {
    let integration = ConsensusStorageIntegration::new();

    // Test snapshot counter
    for i in 1..=5 {
        integration.record_snapshot_persisted();
        assert_eq!(integration.snapshots_persisted(), i);
    }

    // Test validator update counter
    for i in 1..=3 {
        integration.record_validator_update_persisted();
        assert_eq!(integration.validator_updates_persisted(), i);
    }

    // Test transaction counter
    let mut total = 0;
    for batch_size in &[10, 20, 15, 5] {
        integration.record_transactions_persisted(*batch_size);
        total += batch_size;
        assert_eq!(integration.transactions_persisted(), total);
    }
}

#[test]
fn test_consensus_storage_integration_default_creation() {
    let integration = ConsensusStorageIntegration::default();
    let status = integration.get_status();

    assert!(!status.storage_initialized);
    assert!(!status.all_connected());
}
