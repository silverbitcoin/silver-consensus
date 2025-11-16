//! Snapshot management
//!
//! This module handles snapshot creation, certification, and verification
//! for the Mercury Protocol consensus.

use silver_core::{Error, Result, Snapshot, ValidatorID, Signature};
use std::collections::HashMap;
use tracing::info;

/// Snapshot manager
///
/// Manages snapshot creation, certification, and storage
pub struct SnapshotManager {
    /// Latest snapshot sequence number
    latest_sequence: u64,
    
    /// Pending snapshot certificates
    pending_certificates: HashMap<u64, SnapshotCertificate>,
    
    /// Finalized snapshots
    finalized_snapshots: HashMap<u64, Snapshot>,
}

impl SnapshotManager {
    /// Create a new snapshot manager
    pub fn new() -> Self {
        Self {
            latest_sequence: 0,
            pending_certificates: HashMap::new(),
            finalized_snapshots: HashMap::new(),
        }
    }

    /// Create a new snapshot
    pub fn create_snapshot(
        &mut self,
        state_root: [u8; 64],
        transaction_digests: Vec<[u8; 64]>,
        cycle: u64,
    ) -> Result<Snapshot> {
        use silver_core::{SnapshotDigest, StateDigest, TransactionDigest};
        
        let sequence = self.latest_sequence + 1;
        
        let previous_digest = if sequence > 1 {
            self.finalized_snapshots
                .get(&(sequence - 1))
                .map(|s| s.digest)
                .unwrap_or_else(|| SnapshotDigest::new([0u8; 64]))
        } else {
            SnapshotDigest::new([0u8; 64])
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
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
            Vec::new(), // Signatures added later
            0, // Stake weight added later
        );

        info!(
            "Created snapshot {} with {} transactions",
            sequence,
            tx_digests.len()
        );

        Ok(snapshot)
    }

    /// Add validator signature to snapshot certificate
    pub fn add_signature(
        &mut self,
        sequence: u64,
        validator_id: ValidatorID,
        signature: Signature,
        stake_weight: u64,
    ) -> Result<()> {
        let cert = self.pending_certificates
            .entry(sequence)
            .or_insert_with(|| SnapshotCertificate::new(sequence));

        cert.add_signature(validator_id, signature, stake_weight);

        Ok(())
    }

    /// Check if snapshot has quorum (2/3+ stake)
    pub fn has_quorum(&self, sequence: u64, total_stake: u64) -> bool {
        if let Some(cert) = self.pending_certificates.get(&sequence) {
            cert.has_quorum(total_stake)
        } else {
            false
        }
    }

    /// Finalize snapshot with certificate
    pub fn finalize_snapshot(
        &mut self,
        snapshot: Snapshot,
        total_stake: u64,
    ) -> Result<()> {
        let sequence = snapshot.sequence_number;

        if !self.has_quorum(sequence, total_stake) {
            return Err(Error::InvalidData(format!(
                "Snapshot {} does not have quorum",
                sequence
            )));
        }

        self.finalized_snapshots.insert(sequence, snapshot);
        self.latest_sequence = sequence;

        info!("Finalized snapshot {}", sequence);

        Ok(())
    }

    /// Get latest snapshot sequence
    pub fn latest_sequence(&self) -> u64 {
        self.latest_sequence
    }

    /// Get snapshot by sequence number
    pub fn get_snapshot(&self, sequence: u64) -> Option<&Snapshot> {
        self.finalized_snapshots.get(&sequence)
    }

    /// Get snapshot certificate
    pub fn get_certificate(&self, sequence: u64) -> Option<&SnapshotCertificate> {
        self.pending_certificates.get(&sequence)
    }
}

impl Default for SnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot certificate
///
/// Contains validator signatures for a snapshot
pub struct SnapshotCertificate {
    /// Snapshot sequence number
    pub sequence: u64,
    
    /// Validator signatures
    pub signatures: HashMap<ValidatorID, Signature>,
    
    /// Total stake weight of signers
    pub stake_weight: u64,
}

impl SnapshotCertificate {
    /// Create a new snapshot certificate
    pub fn new(sequence: u64) -> Self {
        Self {
            sequence,
            signatures: HashMap::new(),
            stake_weight: 0,
        }
    }

    /// Add a validator signature
    pub fn add_signature(
        &mut self,
        validator_id: ValidatorID,
        signature: Signature,
        stake: u64,
    ) {
        if self.signatures.insert(validator_id.clone(), signature).is_none() {
            self.stake_weight += stake;
        }
    }

    /// Check if certificate has quorum (2/3+ stake)
    pub fn has_quorum(&self, total_stake: u64) -> bool {
        self.stake_weight * 3 > total_stake * 2
    }

    /// Get number of signatures
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }

    /// Get stake weight
    pub fn stake_weight(&self) -> u64 {
        self.stake_weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{SilverAddress, SignatureScheme};

    fn create_test_validator_id(id: u8) -> ValidatorID {
        ValidatorID::new(SilverAddress::new([id; 64]))
    }

    fn create_test_signature() -> Signature {
        Signature {
            scheme: SignatureScheme::Dilithium3,
            bytes: vec![0u8; 32],
        }
    }

    #[test]
    fn test_snapshot_creation() {
        let mut manager = SnapshotManager::new();
        
        let snapshot = manager.create_snapshot(
            [1u8; 64],
            vec![[2u8; 64], [3u8; 64]],
            1,
        ).unwrap();

        assert_eq!(snapshot.sequence_number, 1);
        assert_eq!(snapshot.transactions.len(), 2);
    }

    #[test]
    fn test_certificate_quorum() {
        let mut cert = SnapshotCertificate::new(1);
        
        // Add signatures for 2/3 stake
        cert.add_signature(create_test_validator_id(1), create_test_signature(), 1000);
        cert.add_signature(create_test_validator_id(2), create_test_signature(), 1000);
        
        // Total stake: 3000, signed: 2000 = 2/3
        assert!(cert.has_quorum(3000));
        
        // Total stake: 3001, signed: 2000 < 2/3
        assert!(!cert.has_quorum(3001));
    }

    #[test]
    fn test_snapshot_finalization() {
        let mut manager = SnapshotManager::new();
        
        let snapshot = manager.create_snapshot(
            [1u8; 64],
            vec![],
            1,
        ).unwrap();

        // Add quorum signatures
        manager.add_signature(1, create_test_validator_id(1), create_test_signature(), 2000).unwrap();
        
        // Should finalize with quorum
        assert!(manager.finalize_snapshot(snapshot, 3000).is_ok());
        assert_eq!(manager.latest_sequence(), 1);
    }
}

