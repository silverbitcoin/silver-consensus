//! Protocol upgrade management
//!
//! This module implements the protocol upgrade mechanism including:
//! - Proposal creation and validation
//! - Validator voting
//! - Quorum checking (2/3+ stake approval)
//! - Upgrade activation at cycle boundaries

use silver_core::{
    ApprovedUpgrade, Error, FeatureFlags, ProposalID, ProtocolVersion, Result,
    SnapshotDigest, UpgradeProposal, UpgradeVote, VotingResults,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// Upgrade manager for protocol upgrades
///
/// Manages the lifecycle of protocol upgrade proposals including:
/// - Proposal submission and validation
/// - Vote collection from validators
/// - Quorum checking
/// - Upgrade activation
pub struct UpgradeManager {
    /// Current protocol version
    current_version: Arc<RwLock<ProtocolVersion>>,

    /// Active feature flags
    feature_flags: Arc<RwLock<FeatureFlags>>,

    /// Pending upgrade proposals (proposal_id -> proposal)
    pending_proposals: Arc<RwLock<HashMap<ProposalID, UpgradeProposal>>>,

    /// Voting results for proposals (proposal_id -> results)
    voting_results: Arc<RwLock<HashMap<ProposalID, VotingResults>>>,

    /// Approved upgrades waiting for activation (activation_cycle -> upgrade)
    approved_upgrades: Arc<RwLock<HashMap<u64, ApprovedUpgrade>>>,

    /// Total stake weight in the validator set
    total_stake: Arc<RwLock<u64>>,
}

impl UpgradeManager {
    /// Create a new upgrade manager
    pub fn new(initial_version: ProtocolVersion, total_stake: u64) -> Self {
        info!(
            version = %initial_version,
            total_stake = total_stake,
            "Initializing upgrade manager"
        );

        Self {
            current_version: Arc::new(RwLock::new(initial_version)),
            feature_flags: Arc::new(RwLock::new(FeatureFlags::new())),
            pending_proposals: Arc::new(RwLock::new(HashMap::new())),
            voting_results: Arc::new(RwLock::new(HashMap::new())),
            approved_upgrades: Arc::new(RwLock::new(HashMap::new())),
            total_stake: Arc::new(RwLock::new(total_stake)),
        }
    }

    /// Get the current protocol version
    pub fn current_version(&self) -> ProtocolVersion {
        *self.current_version.read().unwrap()
    }

    /// Get the current feature flags
    pub fn feature_flags(&self) -> FeatureFlags {
        self.feature_flags.read().unwrap().clone()
    }

    /// Check if a feature is enabled
    pub fn is_feature_enabled(&self, feature: &str) -> bool {
        self.feature_flags.read().unwrap().is_enabled(feature)
    }

    /// Update total stake weight
    pub fn update_total_stake(&self, new_total_stake: u64) {
        let mut total_stake = self.total_stake.write().unwrap();
        *total_stake = new_total_stake;

        debug!(
            new_total_stake = new_total_stake,
            "Updated total stake weight"
        );
    }

    /// Submit a new upgrade proposal
    ///
    /// # Requirements
    /// - Proposal must be valid (activation in future, voting deadline before activation)
    /// - Proposer must be a validator
    pub fn submit_proposal(
        &self,
        proposal: UpgradeProposal,
        current_cycle: u64,
    ) -> Result<ProposalID> {
        // Validate proposal
        proposal.validate(current_cycle)?;

        let proposal_id = proposal.proposal_id;

        // Check if proposal already exists
        {
            let pending = self.pending_proposals.read().unwrap();
            if pending.contains_key(&proposal_id) {
                return Err(Error::InvalidData(format!(
                    "Proposal {} already exists",
                    proposal_id
                )));
            }
        }

        // Add to pending proposals
        {
            let mut pending = self.pending_proposals.write().unwrap();
            pending.insert(proposal_id, proposal.clone());
        }

        // Initialize voting results
        {
            let total_stake = *self.total_stake.read().unwrap();
            let mut voting = self.voting_results.write().unwrap();
            voting.insert(proposal_id, VotingResults::new(proposal_id, total_stake));
        }

        info!(
            proposal_id = %proposal_id,
            version = %proposal.new_version,
            activation_cycle = proposal.activation_cycle,
            "Submitted upgrade proposal"
        );

        Ok(proposal_id)
    }

    /// Cast a vote on an upgrade proposal
    ///
    /// # Requirements
    /// - Proposal must exist and voting must be open
    /// - Validator must not have already voted
    /// - Vote must have valid signature
    pub fn cast_vote(&self, vote: UpgradeVote, current_cycle: u64) -> Result<()> {
        // Validate vote
        vote.validate()?;

        // Check if proposal exists
        let proposal = {
            let pending = self.pending_proposals.read().unwrap();
            pending
                .get(&vote.proposal_id)
                .ok_or_else(|| {
                    Error::InvalidData(format!("Proposal {} not found", vote.proposal_id))
                })?
                .clone()
        };

        // Check if voting is still open
        if !proposal.is_voting_open(current_cycle) {
            return Err(Error::InvalidData(format!(
                "Voting closed for proposal {} (deadline: cycle {})",
                vote.proposal_id, proposal.voting_deadline
            )));
        }

        // Add vote to results
        {
            let mut voting = self.voting_results.write().unwrap();
            let results = voting.get_mut(&vote.proposal_id).ok_or_else(|| {
                Error::InvalidData(format!("Voting results not found for {}", vote.proposal_id))
            })?;

            results.add_vote(vote.clone())?;

            debug!(
                proposal_id = %vote.proposal_id,
                validator = %vote.validator,
                approve = vote.approve,
                stake = vote.stake_weight,
                approval_pct = results.approval_percentage(),
                "Vote cast"
            );

            // Check if quorum reached
            if results.has_quorum() {
                info!(
                    proposal_id = %vote.proposal_id,
                    approval_pct = results.approval_percentage(),
                    "Proposal reached quorum"
                );
            }
        }

        Ok(())
    }

    /// Finalize voting for a proposal
    ///
    /// Called when voting deadline is reached. If quorum is achieved,
    /// the proposal is moved to approved upgrades.
    pub fn finalize_voting(
        &self,
        proposal_id: ProposalID,
        current_cycle: u64,
        approval_snapshot: SnapshotDigest,
    ) -> Result<bool> {
        // Get proposal
        let proposal = {
            let pending = self.pending_proposals.read().unwrap();
            pending
                .get(&proposal_id)
                .ok_or_else(|| Error::InvalidData(format!("Proposal {} not found", proposal_id)))?
                .clone()
        };

        // Check if voting deadline reached
        if proposal.is_voting_open(current_cycle) {
            return Err(Error::InvalidData(format!(
                "Voting still open for proposal {} (deadline: cycle {})",
                proposal_id, proposal.voting_deadline
            )));
        }

        // Get voting results
        let results = {
            let voting = self.voting_results.read().unwrap();
            voting
                .get(&proposal_id)
                .ok_or_else(|| {
                    Error::InvalidData(format!("Voting results not found for {}", proposal_id))
                })?
                .clone()
        };

        // Check if quorum reached
        let approved = results.has_quorum();

        if approved {
            // Create approved upgrade
            let approved_upgrade =
                ApprovedUpgrade::new(proposal.clone(), results, approval_snapshot)?;

            // Add to approved upgrades
            {
                let mut approved = self.approved_upgrades.write().unwrap();
                approved.insert(proposal.activation_cycle, approved_upgrade);
            }

            info!(
                proposal_id = %proposal_id,
                version = %proposal.new_version,
                activation_cycle = proposal.activation_cycle,
                "Proposal approved"
            );
        } else {
            warn!(
                proposal_id = %proposal_id,
                approval_pct = results.approval_percentage(),
                "Proposal rejected (insufficient quorum)"
            );
        }

        // Remove from pending
        {
            let mut pending = self.pending_proposals.write().unwrap();
            pending.remove(&proposal_id);
        }

        Ok(approved)
    }

    /// Get a pending proposal by ID
    pub fn get_proposal(&self, proposal_id: &ProposalID) -> Option<UpgradeProposal> {
        let pending = self.pending_proposals.read().unwrap();
        pending.get(proposal_id).cloned()
    }

    /// Get voting results for a proposal
    pub fn get_voting_results(&self, proposal_id: &ProposalID) -> Option<VotingResults> {
        let voting = self.voting_results.read().unwrap();
        voting.get(proposal_id).cloned()
    }

    /// Get all pending proposals
    pub fn get_pending_proposals(&self) -> Vec<UpgradeProposal> {
        let pending = self.pending_proposals.read().unwrap();
        pending.values().cloned().collect()
    }

    /// Get approved upgrade for a specific cycle
    pub fn get_approved_upgrade(&self, cycle: u64) -> Option<ApprovedUpgrade> {
        let approved = self.approved_upgrades.read().unwrap();
        approved.get(&cycle).cloned()
    }

    /// Check if there is an approved upgrade ready for activation
    pub fn check_for_activation(&self, current_cycle: u64) -> Option<ApprovedUpgrade> {
        let approved = self.approved_upgrades.read().unwrap();
        approved.get(&current_cycle).cloned()
    }

    /// Activate an approved upgrade
    ///
    /// This should be called at cycle boundaries when an approved
    /// upgrade's activation cycle is reached.
    ///
    /// # Requirements (27.3)
    /// - Must be called at cycle boundary
    /// - Upgrade must be approved
    pub fn activate_upgrade(&self, cycle: u64) -> Result<ProtocolVersion> {
        // Get approved upgrade
        let upgrade = {
            let approved = self.approved_upgrades.read().unwrap();
            approved
                .get(&cycle)
                .ok_or_else(|| {
                    Error::InvalidData(format!("No approved upgrade for cycle {}", cycle))
                })?
                .clone()
        };

        let old_version = self.current_version();
        let new_version = upgrade.proposal.new_version;

        // Update protocol version
        {
            let mut version = self.current_version.write().unwrap();
            *version = new_version;
        }

        // Update feature flags
        {
            let mut flags = self.feature_flags.write().unwrap();
            *flags = upgrade.proposal.feature_flags.clone();
        }

        // Remove from approved upgrades
        {
            let mut approved = self.approved_upgrades.write().unwrap();
            approved.remove(&cycle);
        }

        info!(
            old_version = %old_version,
            new_version = %new_version,
            cycle = cycle,
            "Protocol upgrade activated"
        );

        Ok(new_version)
    }

    /// Get statistics about pending and approved upgrades
    pub fn get_stats(&self) -> UpgradeStats {
        let pending = self.pending_proposals.read().unwrap();
        let approved = self.approved_upgrades.read().unwrap();

        UpgradeStats {
            current_version: self.current_version(),
            pending_proposals: pending.len(),
            approved_upgrades: approved.len(),
            total_stake: *self.total_stake.read().unwrap(),
        }
    }
}

/// Statistics about upgrade manager state
#[derive(Debug, Clone)]
pub struct UpgradeStats {
    /// Current protocol version
    pub current_version: ProtocolVersion,

    /// Number of pending proposals
    pub pending_proposals: usize,

    /// Number of approved upgrades waiting for activation
    pub approved_upgrades: usize,

    /// Total stake weight
    pub total_stake: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{SignatureScheme, SilverAddress, Signature};

    fn create_test_proposal(activation_cycle: u64) -> UpgradeProposal {
        let version = ProtocolVersion::new(2, 0);
        let flags = FeatureFlags::new();
        let proposer = SilverAddress::new([1u8; 64]);

        UpgradeProposal::new(
            version,
            flags,
            activation_cycle,
            proposer,
            "Test upgrade".to_string(),
            1000,
            activation_cycle - 10,
        )
    }

    fn create_test_vote(
        proposal_id: ProposalID,
        validator_id: u8,
        approve: bool,
        stake: u64,
    ) -> UpgradeVote {
        let validator = SilverAddress::new([validator_id; 64]);
        let signature = Signature {
            scheme: SignatureScheme::Dilithium3,
            bytes: vec![0u8; 100],
        };

        UpgradeVote::new(proposal_id, validator, approve, stake, signature, 1000)
    }

    #[test]
    fn test_submit_proposal() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let result = manager.submit_proposal(proposal.clone(), 50);
        assert!(result.is_ok());

        let proposal_id = result.unwrap();
        assert_eq!(proposal_id, proposal.proposal_id);

        // Check proposal is pending
        let retrieved = manager.get_proposal(&proposal_id);
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_cast_vote() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal, 50).unwrap();

        // Cast vote
        let vote = create_test_vote(proposal_id, 1, true, 700);
        let result = manager.cast_vote(vote, 50);
        assert!(result.is_ok());

        // Check voting results
        let results = manager.get_voting_results(&proposal_id).unwrap();
        assert_eq!(results.approve_stake, 700);
        assert!(results.has_quorum());
    }

    #[test]
    fn test_finalize_voting_approved() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal.clone(), 50).unwrap();

        // Cast approving vote with quorum
        let vote = create_test_vote(proposal_id, 1, true, 700);
        manager.cast_vote(vote, 50).unwrap();

        // Finalize voting after deadline
        let snapshot = SnapshotDigest::new([0u8; 64]);
        let result = manager.finalize_voting(proposal_id, 91, snapshot);
        assert!(result.is_ok());
        assert!(result.unwrap()); // Approved

        // Check approved upgrade exists
        let approved = manager.get_approved_upgrade(100);
        assert!(approved.is_some());
    }

    #[test]
    fn test_finalize_voting_rejected() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal, 50).unwrap();

        // Cast rejecting vote
        let vote = create_test_vote(proposal_id, 1, false, 700);
        manager.cast_vote(vote, 50).unwrap();

        // Finalize voting after deadline
        let snapshot = SnapshotDigest::new([0u8; 64]);
        let result = manager.finalize_voting(proposal_id, 91, snapshot);
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Rejected

        // Check no approved upgrade
        let approved = manager.get_approved_upgrade(100);
        assert!(approved.is_none());
    }

    #[test]
    fn test_activate_upgrade() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal.clone(), 50).unwrap();

        // Approve proposal
        let vote = create_test_vote(proposal_id, 1, true, 700);
        manager.cast_vote(vote, 50).unwrap();

        let snapshot = SnapshotDigest::new([0u8; 64]);
        manager.finalize_voting(proposal_id, 91, snapshot).unwrap();

        // Activate upgrade
        let result = manager.activate_upgrade(100);
        assert!(result.is_ok());

        let new_version = result.unwrap();
        assert_eq!(new_version, ProtocolVersion::new(2, 0));
        assert_eq!(manager.current_version(), ProtocolVersion::new(2, 0));
    }

    #[test]
    fn test_voting_after_deadline() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal, 50).unwrap();

        // Try to vote after deadline
        let vote = create_test_vote(proposal_id, 1, true, 700);
        let result = manager.cast_vote(vote, 91); // After deadline (90)
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_vote() {
        let manager = UpgradeManager::new(ProtocolVersion::new(1, 0), 1000);
        let proposal = create_test_proposal(100);

        let proposal_id = manager.submit_proposal(proposal, 50).unwrap();

        // Cast first vote
        let vote1 = create_test_vote(proposal_id, 1, true, 700);
        manager.cast_vote(vote1, 50).unwrap();

        // Try to cast second vote from same validator
        let vote2 = create_test_vote(proposal_id, 1, false, 700);
        let result = manager.cast_vote(vote2, 50);
        assert!(result.is_err());
    }
}
