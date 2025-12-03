//! Snapshot management
//!
//! This module handles snapshot creation, certification, and verification
//! for the Mercury Protocol consensus.
//!
//! Features:
//! - Snapshot creation with transaction batches
//! - Signature collection from validators
//! - 2/3+ quorum validation
//! - Snapshot finalization and certification
//! - Timeout handling for pending snapshots
//! - Snapshot history and verification

use serde::{Deserialize, Serialize};
use silver_core::{Error, Result, Signature, Snapshot, SnapshotDigest, ValidatorID};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Pending snapshot with timeout tracking
#[derive(Debug, Clone)]
struct PendingSnapshot {
    /// Snapshot data
    snapshot: Snapshot,

    /// Certificate for this snapshot
    certificate: SnapshotCertificate,

    /// When this snapshot was created
    created_at: Instant,

    /// Finalization timeout
    timeout: Duration,
}

impl PendingSnapshot {
    /// Check if snapshot has timed out
    fn is_timed_out(&self) -> bool {
        self.created_at.elapsed() > self.timeout
    }

    /// Get time remaining before timeout
    fn time_remaining(&self) -> Duration {
        self.timeout.saturating_sub(self.created_at.elapsed())
    }

    /// Log timeout status
    fn log_timeout_status(&self) {
        let remaining = self.time_remaining();
        if remaining.as_secs() < 10 {
            warn!(
                "Snapshot timeout approaching: {}s remaining",
                remaining.as_secs()
            );
        }
    }
}

/// Snapshot manager
///
/// Manages snapshot creation, certification, and storage with:
/// - Signature collection from validators
/// - 2/3+ quorum validation
/// - Timeout handling
/// - Finalization and certification
/// - History tracking
pub struct SnapshotManager {
    /// Latest snapshot sequence number
    latest_sequence: u64,

    /// Pending snapshots awaiting finalization
    pending_snapshots: HashMap<u64, PendingSnapshot>,

    /// Finalized snapshots with certificates
    finalized_snapshots: HashMap<u64, (Snapshot, SnapshotCertificate)>,

    /// Snapshot finalization timeout (default: 5 seconds)
    finalization_timeout: Duration,

    /// Maximum pending snapshots before backpressure
    max_pending: usize,
}

impl SnapshotManager {
    /// Create a new snapshot manager with default timeout (5 seconds)
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(5))
    }

    /// Create a new snapshot manager with custom timeout
    pub fn with_timeout(finalization_timeout: Duration) -> Self {
        Self {
            latest_sequence: 0,
            pending_snapshots: HashMap::new(),
            finalized_snapshots: HashMap::new(),
            finalization_timeout,
            max_pending: 1000,
        }
    }

    /// Create a new snapshot
    ///
    /// Creates a snapshot with the given state root and transactions.
    /// The snapshot is added to pending snapshots awaiting signatures.
    pub fn create_snapshot(
        &mut self,
        state_root: [u8; 64],
        transaction_digests: Vec<[u8; 64]>,
        cycle: u64,
        total_stake: u64,
    ) -> Result<Snapshot> {
        use silver_core::{StateDigest, TransactionDigest};

        // Check backpressure
        if self.pending_snapshots.len() >= self.max_pending {
            return Err(Error::InvalidData(format!(
                "Too many pending snapshots ({}), backpressure applied",
                self.pending_snapshots.len()
            )));
        }

        let sequence = self.latest_sequence + 1;

        // Get previous snapshot digest
        let previous_digest = if sequence > 1 {
            self.finalized_snapshots
                .get(&(sequence - 1))
                .map(|(s, _)| s.digest)
                .unwrap_or_else(|| SnapshotDigest::new([0u8; 64]))
        } else {
            SnapshotDigest::new([0u8; 64])
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let tx_digests: Vec<TransactionDigest> = transaction_digests
            .into_iter()
            .map(TransactionDigest::new)
            .collect();

        let snapshot = Snapshot::new(
            sequence,
            timestamp,
            previous_digest,
            StateDigest::new(state_root),
            tx_digests.clone(),
            cycle,
            Vec::new(),
            0,
        );

        // Create certificate
        let snapshot_digest = SnapshotDigest::new(state_root);
        let certificate = SnapshotCertificate::new(sequence, snapshot_digest, total_stake);

        // Add to pending
        self.pending_snapshots.insert(
            sequence,
            PendingSnapshot {
                snapshot: snapshot.clone(),
                certificate,
                created_at: Instant::now(),
                timeout: self.finalization_timeout,
            },
        );

        info!(
            "Created snapshot {} with {} transactions (pending: {})",
            sequence,
            tx_digests.len(),
            self.pending_snapshots.len()
        );

        Ok(snapshot)
    }

    /// Add validator signature to snapshot
    ///
    /// Adds a validator's signature to the snapshot certificate.
    /// Returns true if signature was added, false if validator already signed.
    pub fn add_signature(
        &mut self,
        sequence: u64,
        validator_id: ValidatorID,
        signature: Signature,
        stake_weight: u64,
    ) -> Result<bool> {
        let pending = self.pending_snapshots.get_mut(&sequence).ok_or_else(|| {
            Error::InvalidData(format!(
                "Snapshot {} not found or already finalized",
                sequence
            ))
        })?;

        // Check timeout
        if pending.is_timed_out() {
            return Err(Error::InvalidData(format!(
                "Snapshot {} finalization timed out",
                sequence
            )));
        }

        pending
            .certificate
            .add_signature(validator_id, signature, stake_weight)
    }

    /// Check if snapshot has quorum
    pub fn has_quorum(&self, sequence: u64) -> bool {
        self.pending_snapshots
            .get(&sequence)
            .map(|p| p.certificate.has_quorum)
            .unwrap_or(false)
    }

    /// Get quorum percentage for snapshot
    pub fn get_quorum_percentage(&self, sequence: u64) -> Option<f64> {
        self.pending_snapshots
            .get(&sequence)
            .map(|p| p.certificate.quorum_percentage())
    }

    /// Finalize snapshot with certificate
    ///
    /// Finalizes a snapshot by verifying quorum and moving it from
    /// pending to finalized snapshots.
    pub fn finalize_snapshot(&mut self, sequence: u64) -> Result<SnapshotCertificate> {
        let mut pending = self
            .pending_snapshots
            .remove(&sequence)
            .ok_or_else(|| Error::InvalidData(format!("Snapshot {} not found", sequence)))?;

        // Verify quorum
        if !pending.certificate.has_quorum {
            return Err(Error::InvalidData(format!(
                "Snapshot {} does not have quorum ({}/{} stake)",
                sequence, pending.certificate.stake_weight, pending.certificate.total_stake
            )));
        }

        // Finalize certificate
        pending.certificate.finalize()?;

        // Store finalized snapshot
        let certificate = pending.certificate.clone();
        self.finalized_snapshots
            .insert(sequence, (pending.snapshot, certificate.clone()));
        self.latest_sequence = sequence;

        info!(
            "Finalized snapshot {} with {} signatures in {:.2}s",
            sequence,
            certificate.signature_count(),
            pending.created_at.elapsed().as_secs_f64()
        );

        Ok(certificate)
    }

    /// Process timed-out snapshots
    ///
    /// Removes snapshots that have exceeded the finalization timeout.
    /// Returns list of timed-out snapshot sequences.
    pub fn process_timeouts(&mut self) -> Vec<u64> {
        let mut timed_out = Vec::new();

        let sequences: Vec<u64> = self
            .pending_snapshots
            .iter()
            .filter(|(_, p)| p.is_timed_out())
            .map(|(seq, _)| *seq)
            .collect();

        for sequence in sequences {
            if let Some(pending) = self.pending_snapshots.remove(&sequence) {
                pending.log_timeout_status();
                warn!(
                    "Snapshot {} timed out after {:.2}s with {}/{} stake",
                    sequence,
                    pending.created_at.elapsed().as_secs_f64(),
                    pending.certificate.stake_weight,
                    pending.certificate.total_stake
                );
                timed_out.push(sequence);
            }
        }

        timed_out
    }

    /// Check pending snapshot timeouts
    pub fn check_pending_timeouts(&self) {
        for (seq, pending) in &self.pending_snapshots {
            let remaining = pending.time_remaining();
            if remaining.as_secs() < 30 {
                info!("Snapshot {} timeout in {}s", seq, remaining.as_secs());
            }
        }
    }

    /// Get latest snapshot sequence
    pub fn latest_sequence(&self) -> u64 {
        self.latest_sequence
    }

    /// Get finalized snapshot by sequence
    pub fn get_snapshot(&self, sequence: u64) -> Option<&Snapshot> {
        self.finalized_snapshots.get(&sequence).map(|(s, _)| s)
    }

    /// Get finalized snapshot with certificate
    pub fn get_snapshot_with_certificate(
        &self,
        sequence: u64,
    ) -> Option<(&Snapshot, &SnapshotCertificate)> {
        self.finalized_snapshots.get(&sequence).map(|(s, c)| (s, c))
    }

    /// Get pending snapshot certificate
    pub fn get_pending_certificate(&self, sequence: u64) -> Option<&SnapshotCertificate> {
        self.pending_snapshots
            .get(&sequence)
            .map(|p| &p.certificate)
    }

    /// Get finalized certificate
    pub fn get_certificate(&self, sequence: u64) -> Option<&SnapshotCertificate> {
        self.finalized_snapshots.get(&sequence).map(|(_, c)| c)
    }

    /// Get pending snapshot count
    pub fn pending_count(&self) -> usize {
        self.pending_snapshots.len()
    }

    /// Get finalized snapshot count
    pub fn finalized_count(&self) -> usize {
        self.finalized_snapshots.len()
    }

    /// Get all pending snapshot sequences
    pub fn get_pending_sequences(&self) -> Vec<u64> {
        self.pending_snapshots.keys().copied().collect()
    }

    /// Verify snapshot certificate
    pub fn verify_certificate(&self, sequence: u64) -> Result<()> {
        self.finalized_snapshots
            .get(&sequence)
            .ok_or_else(|| Error::InvalidData(format!("Snapshot {} not found", sequence)))?
            .1
            .verify()
    }

    /// Get snapshot finalization time
    pub fn get_finalization_time(&self, sequence: u64) -> Option<u64> {
        self.finalized_snapshots
            .get(&sequence)
            .and_then(|(_, c)| c.finalization_time())
    }

    /// Get time to finalization
    pub fn get_time_to_finalization(&self, sequence: u64) -> Option<u64> {
        self.finalized_snapshots
            .get(&sequence)
            .and_then(|(_, c)| c.time_to_finalization())
    }

    /// Clear old snapshots (keep last N)
    pub fn prune_old_snapshots(&mut self, keep_count: usize) {
        if self.finalized_snapshots.len() <= keep_count {
            return;
        }

        let to_remove = self.finalized_snapshots.len() - keep_count;
        let mut sequences: Vec<u64> = self.finalized_snapshots.keys().copied().collect();
        sequences.sort();

        for sequence in sequences.iter().take(to_remove) {
            self.finalized_snapshots.remove(sequence);
            debug!("Pruned snapshot {}", sequence);
        }

        info!(
            "Pruned {} old snapshots, keeping {} finalized snapshots",
            to_remove,
            self.finalized_snapshots.len()
        );
    }
}

impl Default for SnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot certificate with full validation metadata
///
/// Contains validator signatures for a snapshot with complete
/// finalization information and verification data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotCertificate {
    /// Snapshot sequence number
    pub sequence: u64,

    /// Snapshot digest (hash)
    pub snapshot_digest: SnapshotDigest,

    /// Validator signatures indexed by validator ID
    pub signatures: HashMap<ValidatorID, ValidatorSignatureInfo>,

    /// Total stake weight of signers
    pub stake_weight: u64,

    /// Total stake in network at finalization
    pub total_stake: u64,

    /// Timestamp when certificate was created
    pub created_at: u64,

    /// Timestamp when certificate was finalized
    pub finalized_at: Option<u64>,

    /// Whether certificate has quorum
    pub has_quorum: bool,
}

/// Validator signature information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSignatureInfo {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Signature bytes
    pub signature: Signature,

    /// Validator stake weight
    pub stake_weight: u64,

    /// Timestamp when signature was added
    pub signed_at: u64,
}

impl SnapshotCertificate {
    /// Create a new snapshot certificate
    pub fn new(sequence: u64, snapshot_digest: SnapshotDigest, total_stake: u64) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            sequence,
            snapshot_digest,
            signatures: HashMap::new(),
            stake_weight: 0,
            total_stake,
            created_at,
            finalized_at: None,
            has_quorum: false,
        }
    }

    /// Add a validator signature
    ///
    /// Returns true if this is a new signature, false if validator already signed
    pub fn add_signature(
        &mut self,
        validator_id: ValidatorID,
        signature: Signature,
        stake_weight: u64,
    ) -> Result<bool> {
        if stake_weight == 0 {
            return Err(Error::InvalidData(
                "Validator stake weight must be > 0".to_string(),
            ));
        }

        let signed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sig_info = ValidatorSignatureInfo {
            validator_id: validator_id.clone(),
            signature,
            stake_weight,
            signed_at,
        };

        // Check if validator already signed
        if self.signatures.contains_key(&validator_id) {
            debug!(
                "Validator {} already signed snapshot {}",
                validator_id, self.sequence
            );
            return Ok(false);
        }

        self.signatures.insert(validator_id.clone(), sig_info);
        self.stake_weight += stake_weight;

        // Check if we now have quorum
        self.has_quorum = self.check_quorum();

        debug!(
            "Added signature from validator {} for snapshot {} (stake: {}, total: {}/{})",
            validator_id, self.sequence, stake_weight, self.stake_weight, self.total_stake
        );

        Ok(true)
    }

    /// Check if certificate has quorum (2/3+ stake)
    fn check_quorum(&self) -> bool {
        if self.total_stake == 0 {
            return false;
        }
        self.stake_weight * 3 > self.total_stake * 2
    }

    /// Finalize the certificate
    ///
    /// Marks the certificate as finalized and verifies quorum
    pub fn finalize(&mut self) -> Result<()> {
        if !self.has_quorum {
            return Err(Error::InvalidData(format!(
                "Cannot finalize snapshot {} without quorum (have {}/{}, need 2/3)",
                self.sequence, self.stake_weight, self.total_stake
            )));
        }

        let finalized_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.finalized_at = Some(finalized_at);

        info!(
            "Finalized snapshot {} with {} signatures ({}/{} stake)",
            self.sequence,
            self.signatures.len(),
            self.stake_weight,
            self.total_stake
        );

        Ok(())
    }

    /// Check if certificate is finalized
    pub fn is_finalized(&self) -> bool {
        self.finalized_at.is_some()
    }

    /// Get finalization time if available
    pub fn finalization_time(&self) -> Option<u64> {
        self.finalized_at
    }

    /// Get time to finalization in seconds
    pub fn time_to_finalization(&self) -> Option<u64> {
        self.finalized_at
            .map(|finalized| finalized - self.created_at)
    }

    /// Get number of signatures
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }

    /// Get stake weight
    pub fn stake_weight(&self) -> u64 {
        self.stake_weight
    }

    /// Get quorum percentage
    pub fn quorum_percentage(&self) -> f64 {
        if self.total_stake == 0 {
            return 0.0;
        }
        (self.stake_weight as f64 / self.total_stake as f64) * 100.0
    }

    /// Verify certificate integrity
    ///
    /// Checks:
    /// - Has quorum
    /// - Is finalized
    /// - All signatures present
    pub fn verify(&self) -> Result<()> {
        if !self.has_quorum {
            return Err(Error::InvalidData(format!(
                "Certificate for snapshot {} does not have quorum",
                self.sequence
            )));
        }

        if !self.is_finalized() {
            return Err(Error::InvalidData(format!(
                "Certificate for snapshot {} is not finalized",
                self.sequence
            )));
        }

        if self.signatures.is_empty() {
            return Err(Error::InvalidData(format!(
                "Certificate for snapshot {} has no signatures",
                self.sequence
            )));
        }

        Ok(())
    }
}
