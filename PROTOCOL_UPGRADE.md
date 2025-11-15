# Protocol Upgrade Mechanism

This document describes the protocol upgrade mechanism implemented for SilverBitcoin blockchain.

## Overview

The protocol upgrade mechanism allows the network to evolve safely through coordinated upgrades that require validator approval. The system ensures:

- **Democratic Governance**: Upgrades require 2/3+ stake approval from validators
- **Safe Activation**: Upgrades activate atomically at cycle boundaries
- **Backward Compatibility**: Multiple protocol versions supported during transition periods
- **Feature Gating**: New features can be enabled/disabled through feature flags

## Components

### 1. Protocol Version (`silver-core/src/protocol.rs`)

Defines the protocol version structure and upgrade proposal types:

- `ProtocolVersion`: Major.minor version numbering
- `FeatureFlags`: Enable/disable specific protocol features
- `UpgradeProposal`: Proposed protocol upgrade with activation cycle
- `UpgradeVote`: Validator vote on a proposal
- `VotingResults`: Aggregated voting results with quorum checking
- `ApprovedUpgrade`: Approved upgrade ready for activation

### 2. Upgrade Manager (`silver-consensus/src/upgrade.rs`)

Manages the upgrade proposal lifecycle:

- Submit new upgrade proposals
- Collect validator votes
- Check for 2/3+ stake quorum
- Finalize voting when deadline reached
- Track approved upgrades

**Key Methods:**
```rust
// Submit a new upgrade proposal
pub fn submit_proposal(&self, proposal: UpgradeProposal, current_cycle: u64) -> Result<ProposalID>

// Cast a validator vote
pub fn cast_vote(&self, vote: UpgradeVote, current_cycle: u64) -> Result<()>

// Finalize voting (called at voting deadline)
pub fn finalize_voting(&self, proposal_id: ProposalID, current_cycle: u64, approval_snapshot: SnapshotDigest) -> Result<bool>

// Activate an approved upgrade
pub fn activate_upgrade(&self, cycle: u64) -> Result<ProtocolVersion>
```

### 3. Activation Coordinator (`silver-consensus/src/activation.rs`)

Coordinates upgrade activation at cycle boundaries:

- Schedule approved upgrades for activation
- Activate upgrades atomically at cycle boundaries
- Support multiple protocol versions during transition
- Remove old versions after transition period

**Key Methods:**
```rust
// Schedule an upgrade for activation
pub fn schedule_activation(&self, upgrade: ApprovedUpgrade) -> Result<()>

// Activate upgrade at cycle boundary
pub fn activate_at_cycle(&self, cycle: u64) -> Result<ProtocolVersion>

// Process cycle boundary (checks for scheduled activations)
pub fn process_cycle_boundary(&self, cycle: u64) -> Result<Option<ProtocolVersion>>

// Remove old version support after transition
pub fn remove_version_support(&self, version: ProtocolVersion) -> Result<()>
```

### 4. Compatibility Checker (`silver-consensus/src/compatibility.rs`)

Validates transaction compatibility during protocol transitions:

- Check if protocol versions are supported
- Validate transactions against active feature flags
- Reject transactions using inactive features
- Extract required features from transactions

**Key Methods:**
```rust
// Check if a feature is enabled
pub fn is_feature_enabled(&self, feature: &str) -> bool

// Validate transaction compatibility
pub fn validate_transaction_compatibility(&self, transaction: &Transaction, required_features: &[String]) -> Result<()>

// Register feature requirements
pub fn register_feature_requirement(&self, feature: String, min_version: ProtocolVersion)
```

## Upgrade Workflow

### Phase 1: Proposal Submission

1. A validator creates an `UpgradeProposal` specifying:
   - New protocol version (e.g., v2.0)
   - Feature flags to enable
   - Activation cycle (when to activate if approved)
   - Voting deadline (cycle number)

2. The proposal is submitted to the `UpgradeManager`

3. The manager validates the proposal and initializes voting

### Phase 2: Voting Period

1. Validators cast votes using `UpgradeVote`:
   - Approve or reject the proposal
   - Include validator signature
   - Include stake weight

2. The `UpgradeManager` aggregates votes:
   - Tracks approve/reject stake weights
   - Checks for 2/3+ quorum
   - Prevents duplicate votes

3. Voting continues until the deadline cycle

### Phase 3: Finalization

1. At the voting deadline, `finalize_voting()` is called:
   - Checks if quorum (2/3+ stake) was reached
   - If approved: creates `ApprovedUpgrade` and schedules activation
   - If rejected: removes proposal from pending

2. Approved upgrades are stored with their activation cycle

### Phase 4: Activation

1. At the activation cycle boundary:
   - `ActivationCoordinator.process_cycle_boundary()` detects scheduled upgrade
   - Upgrade is activated atomically:
     - Active version updated to new version
     - New version added to supported versions
     - Old version remains supported during transition

2. During transition period (default: 10 cycles):
   - Both old and new versions are supported
   - Transactions can use either version
   - `CompatibilityChecker` validates transactions

3. After transition period:
   - Old version support is removed
   - Only new version remains active

## Example Usage

```rust
use silver_consensus::{UpgradeManager, ActivationCoordinator, CompatibilityChecker};
use silver_core::{ProtocolVersion, FeatureFlags, UpgradeProposal};

// Initialize managers
let upgrade_manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1_000_000);
let activation_coordinator = ActivationCoordinator::new(ProtocolVersion::new(1, 0), 10);
let compatibility_checker = CompatibilityChecker::new(ProtocolVersion::new(1, 0));

// Create upgrade proposal
let mut feature_flags = FeatureFlags::new();
feature_flags.enable("advanced_feature".to_string());

let proposal = UpgradeProposal::new(
    ProtocolVersion::new(2, 0),
    feature_flags,
    100, // activation cycle
    proposer_address,
    "Upgrade to v2.0 with advanced features".to_string(),
    current_timestamp,
    90, // voting deadline
);

// Submit proposal
let proposal_id = upgrade_manager.submit_proposal(proposal, current_cycle)?;

// Validators vote
let vote = UpgradeVote::new(
    proposal_id,
    validator_address,
    true, // approve
    validator_stake,
    validator_signature,
    current_timestamp,
);
upgrade_manager.cast_vote(vote, current_cycle)?;

// Finalize voting at deadline
let approved = upgrade_manager.finalize_voting(proposal_id, 90, snapshot_digest)?;

if approved {
    // Get approved upgrade
    let upgrade = upgrade_manager.get_approved_upgrade(100).unwrap();
    
    // Schedule activation
    activation_coordinator.schedule_activation(upgrade)?;
    
    // At cycle 100, activate upgrade
    let new_version = activation_coordinator.activate_at_cycle(100)?;
    
    // Update compatibility checker
    compatibility_checker.update_active_version(new_version);
    compatibility_checker.add_supported_version(new_version, feature_flags);
}
```

## Requirements Satisfied

### Requirement 27.1: Protocol Version Numbers
✅ Implemented `ProtocolVersion` with major.minor versioning in snapshots

### Requirement 27.2: 2/3+ Stake Approval
✅ Implemented voting with quorum checking in `UpgradeManager`

### Requirement 27.3: Cycle Boundary Activation
✅ Implemented atomic activation at cycle boundaries in `ActivationCoordinator`

### Requirement 27.4: Multiple Version Support
✅ Implemented transition period with multiple supported versions

### Requirement 27.5: Feature Rejection
✅ Implemented feature validation in `CompatibilityChecker`

## Testing

All components include comprehensive unit tests:

- `silver-core/src/protocol.rs`: Protocol version and voting tests
- `silver-consensus/src/upgrade.rs`: Upgrade manager tests
- `silver-consensus/src/activation.rs`: Activation coordinator tests
- `silver-consensus/src/compatibility.rs`: Compatibility checker tests

Run tests with:
```bash
cargo test --package silver-core protocol::tests
cargo test --package silver-consensus upgrade::tests
cargo test --package silver-consensus activation::tests
cargo test --package silver-consensus compatibility::tests
```

## Future Enhancements

1. **Automatic Transition Cleanup**: Automatically remove old version support after transition period
2. **Upgrade Rollback**: Support for emergency rollback if issues detected
3. **Gradual Rollout**: Activate features for subset of validators first
4. **Upgrade Notifications**: Notify node operators of pending upgrades
5. **Upgrade History**: Track all historical upgrades in blockchain state
