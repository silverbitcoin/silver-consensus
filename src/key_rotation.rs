//! Validator key rotation and management
//!
//! This module implements production-ready key rotation:
//! - Key rotation scheduling
//! - Old key revocation
//! - Key rotation voting
//! - Transition period management
//! - Backward compatibility
//! - Comprehensive audit trail

use serde::{Deserialize, Serialize};
use silver_core::{Error, PublicKey, Result, ValidatorID};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Key rotation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationConfig {
    /// Transition period (seconds) to accept both old and new keys
    pub transition_period_secs: u64,

    /// Votes required for key rotation (percentage of total stake)
    pub votes_required: f64,

    /// Maximum keys per validator
    pub max_keys_per_validator: usize,

    /// Key expiration period (seconds)
    pub key_expiration_secs: u64,
}

impl Default for KeyRotationConfig {
    fn default() -> Self {
        Self {
            transition_period_secs: 604800, // 7 days
            votes_required: 0.66,           // 66% (2/3)
            max_keys_per_validator: 3,      // Current + 2 old
            key_expiration_secs: 2592000,   // 30 days
        }
    }
}

/// Key status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyStatus {
    /// Active key (currently used)
    Active,

    /// Pending key (waiting for transition)
    Pending,

    /// Transitioning (accepting both old and new)
    Transitioning,

    /// Revoked (no longer accepted)
    Revoked,

    /// Expired (past expiration date)
    Expired,
}

impl std::fmt::Display for KeyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyStatus::Active => write!(f, "Active"),
            KeyStatus::Pending => write!(f, "Pending"),
            KeyStatus::Transitioning => write!(f, "Transitioning"),
            KeyStatus::Revoked => write!(f, "Revoked"),
            KeyStatus::Expired => write!(f, "Expired"),
        }
    }
}

/// Key record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRecord {
    /// Key ID (hash of public key)
    pub key_id: Vec<u8>,

    /// Public key
    pub public_key: PublicKey,

    /// Current status
    pub status: KeyStatus,

    /// Created at
    pub created_at: u64,

    /// Activated at
    pub activated_at: Option<u64>,

    /// Transition starts at
    pub transition_starts_at: Option<u64>,

    /// Transition ends at
    pub transition_ends_at: Option<u64>,

    /// Revoked at
    pub revoked_at: Option<u64>,

    /// Expires at
    pub expires_at: Option<u64>,
}

impl KeyRecord {
    /// Create new key record
    pub fn new(key_id: Vec<u8>, public_key: PublicKey) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            key_id,
            public_key,
            status: KeyStatus::Pending,
            created_at,
            activated_at: None,
            transition_starts_at: None,
            transition_ends_at: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    /// Activate key
    pub fn activate(&mut self, transition_period_secs: u64) -> Result<()> {
        if self.status != KeyStatus::Pending {
            return Err(Error::InvalidData(format!(
                "Cannot activate key with status: {}",
                self.status
            )));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.status = KeyStatus::Transitioning;
        self.activated_at = Some(now);
        self.transition_starts_at = Some(now);
        self.transition_ends_at = Some(now + transition_period_secs);

        debug!(
            "Activated key (transition period: {} seconds)",
            transition_period_secs
        );

        Ok(())
    }

    /// Complete transition
    pub fn complete_transition(&mut self) -> Result<()> {
        if self.status != KeyStatus::Transitioning {
            return Err(Error::InvalidData(format!(
                "Cannot complete transition for key with status: {}",
                self.status
            )));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if let Some(transition_ends_at) = self.transition_ends_at {
            if now < transition_ends_at {
                return Err(Error::InvalidData(format!(
                    "Transition period not complete (ends at: {})",
                    transition_ends_at
                )));
            }
        }

        self.status = KeyStatus::Active;

        info!("Completed key transition");

        Ok(())
    }

    /// Revoke key
    pub fn revoke(&mut self) -> Result<()> {
        if self.status == KeyStatus::Revoked {
            return Err(Error::InvalidData("Key already revoked".to_string()));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.status = KeyStatus::Revoked;
        self.revoked_at = Some(now);

        warn!("Revoked key");

        Ok(())
    }

    /// Check if key is valid (can be used for verification)
    pub fn is_valid(&self) -> bool {
        match self.status {
            KeyStatus::Active | KeyStatus::Transitioning => true,
            _ => false,
        }
    }

    /// Check if transition is complete
    pub fn is_transition_complete(&self) -> bool {
        if let Some(transition_ends_at) = self.transition_ends_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now >= transition_ends_at
        } else {
            false
        }
    }

    /// Get remaining transition time
    pub fn remaining_transition_time(&self) -> u64 {
        if let Some(transition_ends_at) = self.transition_ends_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now >= transition_ends_at {
                0
            } else {
                transition_ends_at - now
            }
        } else {
            0
        }
    }
}

/// Key rotation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationRequest {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// New key ID
    pub new_key_id: Vec<u8>,

    /// New public key
    pub new_public_key: PublicKey,

    /// Requested at
    pub requested_at: u64,

    /// Votes in favor
    pub votes_in_favor: u64,

    /// Total votes
    pub total_votes: u64,

    /// Status
    pub status: KeyRotationRequestStatus,
}

/// Key rotation request status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyRotationRequestStatus {
    /// Pending voting
    Pending,

    /// Approved
    Approved,

    /// Rejected
    Rejected,

    /// Expired
    Expired,
}

impl std::fmt::Display for KeyRotationRequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyRotationRequestStatus::Pending => write!(f, "Pending"),
            KeyRotationRequestStatus::Approved => write!(f, "Approved"),
            KeyRotationRequestStatus::Rejected => write!(f, "Rejected"),
            KeyRotationRequestStatus::Expired => write!(f, "Expired"),
        }
    }
}

/// Key rotation event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationEvent {
    /// Validator ID
    pub validator_id: ValidatorID,

    /// Event type
    pub event_type: KeyRotationEventType,

    /// Key ID
    pub key_id: Vec<u8>,

    /// Timestamp
    pub timestamp: u64,

    /// Cycle
    pub cycle: u64,
}

/// Key rotation event type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyRotationEventType {
    /// Key rotation requested
    RotationRequested,

    /// Key rotation approved
    RotationApproved,

    /// Key rotation rejected
    RotationRejected,

    /// Key transition started
    TransitionStarted,

    /// Key transition completed
    TransitionCompleted,

    /// Key revoked
    KeyRevoked,
}

impl std::fmt::Display for KeyRotationEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyRotationEventType::RotationRequested => write!(f, "RotationRequested"),
            KeyRotationEventType::RotationApproved => write!(f, "RotationApproved"),
            KeyRotationEventType::RotationRejected => write!(f, "RotationRejected"),
            KeyRotationEventType::TransitionStarted => write!(f, "TransitionStarted"),
            KeyRotationEventType::TransitionCompleted => write!(f, "TransitionCompleted"),
            KeyRotationEventType::KeyRevoked => write!(f, "KeyRevoked"),
        }
    }
}

/// Key rotation manager
pub struct KeyRotationManager {
    /// Configuration
    config: KeyRotationConfig,

    /// Validator keys (validator_id -> key_id -> KeyRecord)
    validator_keys: HashMap<ValidatorID, HashMap<Vec<u8>, KeyRecord>>,

    /// Current active key per validator
    active_keys: HashMap<ValidatorID, Vec<u8>>,

    /// Pending rotation requests
    pending_requests: HashMap<ValidatorID, KeyRotationRequest>,

    /// Key rotation events
    events: Vec<KeyRotationEvent>,

    /// Current cycle
    current_cycle: u64,
}

impl KeyRotationManager {
    /// Create new key rotation manager
    pub fn new(config: KeyRotationConfig) -> Self {
        Self {
            config,
            validator_keys: HashMap::new(),
            active_keys: HashMap::new(),
            pending_requests: HashMap::new(),
            events: Vec::new(),
            current_cycle: 0,
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(KeyRotationConfig::default())
    }

    /// Register initial key for validator
    pub fn register_initial_key(
        &mut self,
        validator_id: ValidatorID,
        key_id: Vec<u8>,
        public_key: PublicKey,
    ) -> Result<()> {
        if self.active_keys.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} already has an active key",
                validator_id
            )));
        }

        let mut key_record = KeyRecord::new(key_id.clone(), public_key);
        key_record.status = KeyStatus::Active;
        key_record.activated_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );

        let keys = self
            .validator_keys
            .entry(validator_id.clone())
            .or_insert_with(HashMap::new);

        keys.insert(key_id.clone(), key_record);
        self.active_keys.insert(validator_id.clone(), key_id);

        info!("Registered initial key for validator {}", validator_id);

        Ok(())
    }

    /// Request key rotation
    pub fn request_key_rotation(
        &mut self,
        validator_id: ValidatorID,
        new_key_id: Vec<u8>,
        new_public_key: PublicKey,
    ) -> Result<KeyRotationRequest> {
        // Check if validator has active key
        if !self.active_keys.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} has no active key",
                validator_id
            )));
        }

        // Check if already has pending request
        if self.pending_requests.contains_key(&validator_id) {
            return Err(Error::InvalidData(format!(
                "Validator {} already has a pending key rotation request",
                validator_id
            )));
        }

        let request = KeyRotationRequest {
            validator_id: validator_id.clone(),
            new_key_id: new_key_id.clone(),
            new_public_key,
            requested_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            votes_in_favor: 0,
            total_votes: 0,
            status: KeyRotationRequestStatus::Pending,
        };

        self.pending_requests
            .insert(validator_id.clone(), request.clone());

        let event = KeyRotationEvent {
            validator_id: validator_id.clone(),
            event_type: KeyRotationEventType::RotationRequested,
            key_id: new_key_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            cycle: self.current_cycle,
        };

        self.events.push(event);

        info!("Requested key rotation for validator {}", validator_id);

        Ok(request)
    }

    /// Vote on key rotation request
    pub fn vote_on_rotation(
        &mut self,
        validator_id: &ValidatorID,
        vote_in_favor: bool,
        voting_power: u64,
    ) -> Result<()> {
        let request = self.pending_requests.get_mut(validator_id).ok_or_else(|| {
            Error::InvalidData(format!(
                "No pending key rotation request for validator {}",
                validator_id
            ))
        })?;

        request.total_votes += voting_power;
        if vote_in_favor {
            request.votes_in_favor += voting_power;
        }

        debug!(
            "Recorded vote for key rotation of validator {}: {} votes in favor, {} total",
            validator_id, request.votes_in_favor, request.total_votes
        );

        Ok(())
    }

    /// Finalize key rotation request
    pub fn finalize_rotation(
        &mut self,
        validator_id: &ValidatorID,
        total_stake: u64,
    ) -> Result<KeyRotationRequestStatus> {
        let request = self.pending_requests.get_mut(validator_id).ok_or_else(|| {
            Error::InvalidData(format!(
                "No pending key rotation request for validator {}",
                validator_id
            ))
        })?;

        let required_votes = (total_stake as f64 * self.config.votes_required) as u64;

        let status = if request.votes_in_favor >= required_votes {
            KeyRotationRequestStatus::Approved
        } else {
            KeyRotationRequestStatus::Rejected
        };

        request.status = status;

        if status == KeyRotationRequestStatus::Approved {
            // Add new key
            let mut new_key =
                KeyRecord::new(request.new_key_id.clone(), request.new_public_key.clone());
            new_key.activate(self.config.transition_period_secs)?;

            let keys = self
                .validator_keys
                .entry(validator_id.clone())
                .or_insert_with(HashMap::new);

            // Check max keys limit
            if keys.len() >= self.config.max_keys_per_validator {
                // Remove oldest revoked key
                let oldest_revoked = keys
                    .iter()
                    .filter(|(_, k)| k.status == KeyStatus::Revoked)
                    .min_by_key(|(_, k)| k.revoked_at.unwrap_or(0))
                    .map(|(id, _)| id.clone());

                if let Some(key_id) = oldest_revoked {
                    keys.remove(&key_id);
                }
            }

            keys.insert(request.new_key_id.clone(), new_key);

            let event = KeyRotationEvent {
                validator_id: validator_id.clone(),
                event_type: KeyRotationEventType::RotationApproved,
                key_id: request.new_key_id.clone(),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                cycle: self.current_cycle,
            };

            self.events.push(event);

            info!(
                "Approved key rotation for validator {} ({}/{} votes)",
                validator_id, request.votes_in_favor, required_votes
            );
        } else {
            let event = KeyRotationEvent {
                validator_id: validator_id.clone(),
                event_type: KeyRotationEventType::RotationRejected,
                key_id: request.new_key_id.clone(),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                cycle: self.current_cycle,
            };

            self.events.push(event);

            warn!(
                "Rejected key rotation for validator {} ({}/{} votes)",
                validator_id, request.votes_in_favor, required_votes
            );
        }

        self.pending_requests.remove(validator_id);

        Ok(status)
    }

    /// Process key transitions
    pub fn process_transitions(&mut self) -> Vec<ValidatorID> {
        let mut completed = Vec::new();

        for (validator_id, keys) in self.validator_keys.iter_mut() {
            for (key_id, key) in keys.iter_mut() {
                if key.status == KeyStatus::Transitioning && key.is_transition_complete() {
                    if key.complete_transition().is_ok() {
                        // Update active key
                        self.active_keys
                            .insert(validator_id.clone(), key_id.clone());
                        completed.push(validator_id.clone());

                        let event = KeyRotationEvent {
                            validator_id: validator_id.clone(),
                            event_type: KeyRotationEventType::TransitionCompleted,
                            key_id: key_id.clone(),
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            cycle: self.current_cycle,
                        };

                        self.events.push(event);

                        info!("Completed key transition for validator {}", validator_id);
                    }
                }
            }
        }

        completed
    }

    /// Get active key for validator
    pub fn get_active_key(&self, validator_id: &ValidatorID) -> Option<&KeyRecord> {
        self.active_keys.get(validator_id).and_then(|key_id| {
            self.validator_keys
                .get(validator_id)
                .and_then(|keys| keys.get(key_id))
        })
    }

    /// Get all valid keys for validator (active + transitioning)
    pub fn get_valid_keys(&self, validator_id: &ValidatorID) -> Vec<&KeyRecord> {
        self.validator_keys
            .get(validator_id)
            .map(|keys| keys.values().filter(|k| k.is_valid()).collect())
            .unwrap_or_default()
    }

    /// Get pending rotation request
    pub fn get_pending_request(&self, validator_id: &ValidatorID) -> Option<&KeyRotationRequest> {
        self.pending_requests.get(validator_id)
    }

    /// Get key rotation events
    pub fn get_events(&self) -> &[KeyRotationEvent] {
        &self.events
    }

    /// Get events for validator
    pub fn get_validator_events(&self, validator_id: &ValidatorID) -> Vec<&KeyRotationEvent> {
        self.events
            .iter()
            .filter(|event| event.validator_id == *validator_id)
            .collect()
    }

    /// Advance to next cycle
    pub fn advance_cycle(&mut self) {
        self.current_cycle += 1;
        debug!(
            "Advanced key rotation manager to cycle {}",
            self.current_cycle
        );
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get configuration
    pub fn config(&self) -> &KeyRotationConfig {
        &self.config
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.validator_keys.len()
    }

    /// Get pending request count
    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.len()
    }

    /// Get event count
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}
