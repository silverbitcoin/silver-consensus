//! Backward compatibility layer
//!
//! This module implements backward compatibility during protocol upgrades,
//! ensuring that transactions using old protocol features continue to work
//! during transition periods while rejecting transactions using inactive features.

use silver_core::{Error, FeatureFlags, ProtocolVersion, Result, Transaction};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, warn};

/// Compatibility checker for protocol transitions
///
/// Validates transactions against supported protocol versions and
/// feature flags, ensuring backward compatibility during upgrades.
///
/// # Requirements (27.4, 27.5)
/// - Maintain compatibility during upgrade transition
/// - Reject transactions using inactive features
pub struct CompatibilityChecker {
    /// Active protocol version
    active_version: Arc<RwLock<ProtocolVersion>>,

    /// Supported protocol versions during transition
    /// Maps version -> feature flags
    supported_versions: Arc<RwLock<HashMap<ProtocolVersion, FeatureFlags>>>,

    /// Feature compatibility rules
    /// Maps feature name -> minimum required version
    feature_requirements: Arc<RwLock<HashMap<String, ProtocolVersion>>>,
}

impl CompatibilityChecker {
    /// Create a new compatibility checker
    pub fn new(initial_version: ProtocolVersion) -> Self {
        let mut supported = HashMap::new();
        supported.insert(initial_version, FeatureFlags::new());

        Self {
            active_version: Arc::new(RwLock::new(initial_version)),
            supported_versions: Arc::new(RwLock::new(supported)),
            feature_requirements: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update the active protocol version
    pub fn update_active_version(&self, version: ProtocolVersion) {
        let mut active = self.active_version.write().unwrap();
        *active = version;
        debug!(version = %version, "Updated active protocol version");
    }

    /// Add a supported protocol version
    pub fn add_supported_version(&self, version: ProtocolVersion, flags: FeatureFlags) {
        let mut supported = self.supported_versions.write().unwrap();
        supported.insert(version, flags);
        debug!(version = %version, "Added supported protocol version");
    }

    /// Remove a supported protocol version
    pub fn remove_supported_version(&self, version: ProtocolVersion) -> Result<()> {
        let active = *self.active_version.read().unwrap();

        if version == active {
            return Err(Error::InvalidData(format!(
                "Cannot remove active version {}",
                version
            )));
        }

        let mut supported = self.supported_versions.write().unwrap();
        if supported.remove(&version).is_none() {
            return Err(Error::InvalidData(format!(
                "Version {} not in supported versions",
                version
            )));
        }

        debug!(version = %version, "Removed supported protocol version");
        Ok(())
    }

    /// Register a feature requirement
    ///
    /// Associates a feature with the minimum protocol version required to use it.
    ///
    /// # Arguments
    /// * `feature` - Feature name
    /// * `min_version` - Minimum protocol version required
    pub fn register_feature_requirement(&self, feature: String, min_version: ProtocolVersion) {
        let mut requirements = self.feature_requirements.write().unwrap();
        requirements.insert(feature.clone(), min_version);
        debug!(
            feature = %feature,
            min_version = %min_version,
            "Registered feature requirement"
        );
    }

    /// Check if a protocol version is supported
    pub fn is_version_supported(&self, version: &ProtocolVersion) -> bool {
        let supported = self.supported_versions.read().unwrap();
        supported.contains_key(version)
    }

    /// Check if a feature is enabled in the active version
    pub fn is_feature_enabled(&self, feature: &str) -> bool {
        let active = *self.active_version.read().unwrap();
        let supported = self.supported_versions.read().unwrap();

        if let Some(flags) = supported.get(&active) {
            flags.is_enabled(feature)
        } else {
            false
        }
    }

    /// Check if a feature is enabled in a specific version
    pub fn is_feature_enabled_in_version(&self, feature: &str, version: &ProtocolVersion) -> bool {
        let supported = self.supported_versions.read().unwrap();

        if let Some(flags) = supported.get(version) {
            flags.is_enabled(feature)
        } else {
            false
        }
    }

    /// Validate that a transaction is compatible with current protocol
    ///
    /// # Requirements (27.4, 27.5)
    /// - Maintain compatibility during upgrade transition
    /// - Reject transactions using inactive features
    ///
    /// # Arguments
    /// * `transaction` - The transaction to validate
    /// * `required_features` - Features required by this transaction
    ///
    /// # Returns
    /// * `Ok(())` if transaction is compatible
    /// * `Err` if transaction uses unsupported version or inactive features
    pub fn validate_transaction_compatibility(
        &self,
        _transaction: &Transaction,
        required_features: &[String],
    ) -> Result<()> {
        // Get transaction protocol version from the active version
        // Transactions don't specify protocol version, they use the current active version
        let tx_version = self.active_version.read().unwrap().clone();

        // Check if transaction version is supported
        if !self.is_version_supported(&tx_version) {
            return Err(Error::InvalidData(format!(
                "Transaction uses unsupported protocol version {}",
                tx_version
            )));
        }

        // Check if all required features are enabled
        for feature in required_features {
            if !self.is_feature_enabled(feature) {
                warn!(
                    feature = %feature,
                    version = %tx_version,
                    "Transaction rejected: feature not enabled"
                );

                return Err(Error::InvalidData(format!(
                    "Transaction requires inactive feature: {}",
                    feature
                )));
            }

            // Check if feature meets minimum version requirement
            let requirements = self.feature_requirements.read().unwrap();
            if let Some(min_version) = requirements.get(feature) {
                if tx_version < *min_version {
                    return Err(Error::InvalidData(format!(
                        "Feature {} requires protocol version {} or higher, transaction uses {}",
                        feature, min_version, tx_version
                    )));
                }
            }
        }

        Ok(())
    }

    /// Validate a batch of transactions for compatibility
    ///
    /// # Arguments
    /// * `transactions` - Transactions to validate
    /// * `feature_extractor` - Function to extract required features from a transaction
    ///
    /// # Returns
    /// * `Ok(())` if all transactions are compatible
    /// * `Err` with details of first incompatible transaction
    pub fn validate_batch_compatibility<F>(
        &self,
        transactions: &[Transaction],
        feature_extractor: F,
    ) -> Result<()>
    where
        F: Fn(&Transaction) -> Vec<String>,
    {
        for (idx, tx) in transactions.iter().enumerate() {
            let required_features = feature_extractor(tx);

            if let Err(e) = self.validate_transaction_compatibility(tx, &required_features) {
                return Err(Error::InvalidData(format!(
                    "Transaction {} in batch is incompatible: {}",
                    idx, e
                )));
            }
        }

        Ok(())
    }

    /// Get compatibility statistics
    pub fn get_stats(&self) -> CompatibilityStats {
        let active = *self.active_version.read().unwrap();
        let supported = self.supported_versions.read().unwrap();
        let requirements = self.feature_requirements.read().unwrap();

        CompatibilityStats {
            active_version: active,
            supported_versions: supported.keys().copied().collect(),
            registered_features: requirements.len(),
        }
    }
}

/// Statistics about compatibility checker state
#[derive(Debug, Clone)]
pub struct CompatibilityStats {
    /// Current active protocol version
    pub active_version: ProtocolVersion,

    /// All supported protocol versions
    pub supported_versions: Vec<ProtocolVersion>,

    /// Number of registered feature requirements
    pub registered_features: usize,
}

/// Feature extractor for transactions
///
/// Analyzes a transaction to determine which protocol features it uses.
pub struct FeatureExtractor;

impl FeatureExtractor {
    /// Extract required features from a transaction
    ///
    /// This analyzes the transaction structure and commands to determine
    /// which protocol features are required.
    ///
    /// # Arguments
    /// * `transaction` - The transaction to analyze
    ///
    /// # Returns
    /// * List of feature names required by the transaction
    pub fn extract_features(transaction: &Transaction) -> Vec<String> {
        let mut features = Vec::new();

        // Analyze transaction kind
        match &transaction.data.kind {
            silver_core::TransactionKind::CompositeChain(commands) => {
                // Check for specific command types that require features
                for command in commands {
                    match command {
                        silver_core::Command::TransferObjects { .. } => {
                            // Basic feature, always available
                        }
                        silver_core::Command::SplitCoins { .. } => {
                            // Basic feature, always available
                        }
                        silver_core::Command::MergeCoins { .. } => {
                            // Basic feature, always available
                        }
                        silver_core::Command::Publish { .. } => {
                            features.push("module_publishing".to_string());
                        }
                        silver_core::Command::Call { .. } => {
                            features.push("smart_contracts".to_string());
                        }
                        silver_core::Command::MakeMoveVec { .. } => {
                            features.push("move_vectors".to_string());
                        }
                        silver_core::Command::DeleteObject { .. } => {
                            // Basic feature, always available
                        }
                        silver_core::Command::ShareObject { .. } => {
                            features.push("shared_objects".to_string());
                        }
                        silver_core::Command::FreezeObject { .. } => {
                            features.push("immutable_objects".to_string());
                        }
                    }
                }
            }
            silver_core::TransactionKind::Genesis(_) => {
                features.push("genesis".to_string());
            }
            silver_core::TransactionKind::ConsensusCommit(_) => {
                features.push("consensus_commit".to_string());
            }
        }

        // Check for sponsored transactions
        // (In a real implementation, we'd check if fuel payer != sender)
        // features.push("transaction_sponsorship".to_string());

        features
    }
}
