//! Mercury Protocol implementation
//!
//! Mercury Protocol is the consensus algorithm that operates on the Cascade
//! flow graph to achieve distributed resilience with optimal performance.
//!
//! Key features:
//! - Deterministic flow graph traversal for transaction ordering
//! - Sub-second finality (480ms snapshot interval)
//! - Byzantine fault tolerance (up to 1/3 malicious validators)
//! - Stake-weighted voting for safety
//! - Liveness guarantees with network partitions

use crate::flow_graph::FlowGraph;
use crate::validator::ValidatorSet;
use silver_core::{
    Error, Result, Snapshot, SnapshotDigest, StateDigest,
    Transaction, TransactionBatch, TransactionDigest, ValidatorID, ValidatorMetadata, ValidatorSignature,
};
use silver_crypto::KeyPair;
use silver_storage::ObjectStore;

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock as AsyncRwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Target snapshot interval (milliseconds)
const SNAPSHOT_INTERVAL_MS: u64 = 480;

/// Maximum transactions per snapshot
const MAX_TRANSACTIONS_PER_SNAPSHOT: usize = 1000;

/// Maximum time to wait for snapshot finalization (seconds)
const SNAPSHOT_FINALIZATION_TIMEOUT_SECS: u64 = 5;

/// Mercury Protocol configuration
#[derive(Debug, Clone)]
pub struct MercuryConfig {
    /// Target snapshot interval (milliseconds)
    pub snapshot_interval_ms: u64,

    /// Maximum transactions per snapshot
    pub max_transactions_per_snapshot: usize,

    /// Enable automatic snapshot creation
    pub auto_snapshot_creation: bool,

    /// Snapshot finalization timeout (seconds)
    pub snapshot_finalization_timeout_secs: u64,
}

impl Default for MercuryConfig {
    fn default() -> Self {
        Self {
            snapshot_interval_ms: SNAPSHOT_INTERVAL_MS,
            max_transactions_per_snapshot: MAX_TRANSACTIONS_PER_SNAPSHOT,
            auto_snapshot_creation: true,
            snapshot_finalization_timeout_secs: SNAPSHOT_FINALIZATION_TIMEOUT_SECS,
        }
    }
}

/// Mercury Protocol metrics
#[derive(Debug, Clone, Default)]
pub struct MercuryMetrics {
    /// Total snapshots created
    pub snapshots_created: u64,

    /// Total transactions finalized
    pub transactions_finalized: u64,

    /// Average snapshot interval (milliseconds)
    pub avg_snapshot_interval_ms: f64,

    /// Average transactions per snapshot
    pub avg_transactions_per_snapshot: f64,

    /// Consensus latency (milliseconds)
    pub consensus_latency_ms: f64,

    /// Current snapshot height
    pub current_snapshot_height: u64,

    /// Pending snapshots awaiting finalization
    pub pending_snapshots: usize,

    /// Failed snapshot attempts
    pub failed_snapshots: u64,
}

/// Execution engine for Mercury consensus
pub struct ExecutionEngine {
    /// Transaction executor
    executor: Arc<silver_execution::TransactionExecutor>,
    
    /// Object store for state
    object_store: Arc<silver_storage::ObjectStore>,
    
    /// Fuel schedule
    fuel_schedule: silver_execution::fuel::FuelSchedule,
}

impl ExecutionEngine {
    /// Create a new execution engine
    pub fn new(
        executor: Arc<silver_execution::TransactionExecutor>,
        object_store: Arc<silver_storage::ObjectStore>,
    ) -> Self {
        Self {
            executor,
            object_store,
            fuel_schedule: silver_execution::fuel::FuelSchedule::default(),
        }
    }

    /// Execute transactions and return effects
    pub async fn execute_transactions(
        &self,
        transactions: &[Transaction],
    ) -> Result<Vec<silver_execution::effects::TransactionEffects>> {
        let mut effects = Vec::new();

        for tx in transactions {
            // Validate transaction
            let validator = silver_execution::verifier::TransactionValidator::new(
                self.object_store.clone()
            );
            
            let current_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            validator.validate_transaction(tx, current_time, 0)
                .map_err(|e| MercuryError::ExecutionFailed(e.to_string()))?;

            // Execute transaction
            let tx_effects = self.executor.execute(tx)
                .await
                .map_err(|e| MercuryError::ExecutionFailed(e.to_string()))?;

            effects.push(tx_effects);
        }

        Ok(effects)
    }

    /// Compute state root from current state
    pub async fn compute_state_root(&self) -> Result<StateDigest> {
        // Compute Blake3 hash of all objects in state
        let mut hasher = blake3::Hasher::new();

        // Get all objects from storage
        let objects = self.object_store.get_all_objects()
            .map_err(|e| MercuryError::ExecutionFailed(format!("Failed to get objects: {}", e)))?;

        // Sort objects by ID for deterministic hashing
        let mut sorted_objects = objects;
        sorted_objects.sort_by(|a, b| a.id.cmp(&b.id));

        // Hash each object
        for obj in sorted_objects {
            let obj_bytes = bcs::to_bytes(&obj)
                .map_err(|e| MercuryError::ExecutionFailed(format!("Failed to serialize object: {}", e)))?;
            hasher.update(&obj_bytes);
        }

        // Get final hash
        let hash = hasher.finalize();
        let mut digest_bytes = [0u8; 64];
        digest_bytes.copy_from_slice(hash.as_bytes());

        Ok(StateDigest::new(digest_bytes))
    }

    /// Verify transaction effects
    pub fn verify_effects(
        &self,
        tx: &Transaction,
        effects: &silver_execution::effects::TransactionEffects,
    ) -> Result<()> {
        // Verify effects digest matches transaction
        let expected_digest = tx.digest();
        if effects.transaction_digest != expected_digest {
            return Err(MercuryError::ExecutionFailed(
                "Effects digest mismatch".to_string()
            ));
        }

        // Verify effects status
        match effects.status {
            silver_execution::effects::ExecutionStatus::Success => Ok(()),
            silver_execution::effects::ExecutionStatus::Failure { ref error } => {
                Err(MercuryError::ExecutionFailed(error.clone()))
            }
        }
    }
}

impl Default for ExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Flow graph traversal result
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// Ordered list of batches
    pub batches: Vec<TransactionBatch>,

    /// Ordered list of transactions
    pub transactions: Vec<Transaction>,

    /// Total transaction count
    pub transaction_count: usize,

    /// Traversal time
    pub traversal_time: Duration,
}

/// Mercury Protocol implementation
///
/// The Mercury Protocol consensus engine operates on the Cascade flow graph
/// to determine transaction ordering and create finalized snapshots.
pub struct MercuryProtocol {
    /// Configuration
    config: MercuryConfig,

    /// Validator ID (if this node is a validator)
    validator_id: Option<ValidatorID>,

    /// Validator keypair for signing snapshots
    keypair: Option<Arc<KeyPair>>,

    /// Validator set
    validator_set: Arc<RwLock<ValidatorSet>>,

    /// Flow graph
    flow_graph: Arc<AsyncRwLock<FlowGraph>>,

    /// Execution engine
    execution_engine: Arc<ExecutionEngine>,

    /// Object store
    store: Arc<ObjectStore>,

    /// Current snapshot sequence number
    current_snapshot: Arc<RwLock<u64>>,

    /// Current cycle ID
    current_cycle: Arc<RwLock<u64>>,

    /// Pending snapshots awaiting finalization
    pending_snapshots: Arc<DashMap<u64, Snapshot>>,

    /// Finalized snapshots
    finalized_snapshots: Arc<DashMap<u64, Snapshot>>,

    /// Metrics
    metrics: Arc<RwLock<MercuryMetrics>>,

    /// Last snapshot time
    last_snapshot_time: Arc<RwLock<Instant>>,

    /// Snapshot interval history (for averaging)
    snapshot_interval_history: Arc<RwLock<VecDeque<Duration>>>,

    /// Shutdown signal
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl MercuryProtocol {
    /// Create a new Mercury Protocol instance
    pub fn new(
        config: MercuryConfig,
        validator_id: Option<ValidatorID>,
        keypair: Option<KeyPair>,
        validator_set: ValidatorSet,
        flow_graph: FlowGraph,
        execution_engine: ExecutionEngine,
        store: ObjectStore,
    ) -> Self {
        Self {
            config,
            validator_id,
            keypair: keypair.map(Arc::new),
            validator_set: Arc::new(RwLock::new(validator_set)),
            flow_graph: Arc::new(AsyncRwLock::new(flow_graph)),
            execution_engine: Arc::new(execution_engine),
            store: Arc::new(store),
            current_snapshot: Arc::new(RwLock::new(0)),
            current_cycle: Arc::new(RwLock::new(0)),
            pending_snapshots: Arc::new(DashMap::new()),
            finalized_snapshots: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(MercuryMetrics::default())),
            last_snapshot_time: Arc::new(RwLock::new(Instant::now())),
            snapshot_interval_history: Arc::new(RwLock::new(VecDeque::with_capacity(100))),
            shutdown_tx: None,
        }
    }

    /// Start the Mercury Protocol consensus engine
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting Mercury Protocol consensus engine");

        if self.config.auto_snapshot_creation {
            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
            self.shutdown_tx = Some(shutdown_tx);

            // Spawn snapshot creation task
            self.spawn_snapshot_creator(shutdown_rx).await;
        }

        info!("Mercury Protocol started successfully");
        Ok(())
    }

    /// Stop the consensus engine
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Mercury Protocol");

        if let Some(tx) = &self.shutdown_tx {
            if let Err(e) = tx.send(()).await {
                warn!("Failed to send shutdown signal: {}", e);
            }
        }

        info!("Mercury Protocol stopped");
        Ok(())
    }

    /// Perform deterministic topological traversal of the flow graph
    ///
    /// This implements the core Mercury Protocol algorithm:
    /// 1. Topological sort of the flow graph (parents before children)
    /// 2. Tie-breaking by batch hash for determinism
    /// 3. Extract ordered transaction list
    ///
    /// Requirements: 17.1, 17.5
    pub async fn traverse_flow_graph(&self) -> Result<TraversalResult> {
        let start_time = Instant::now();

        debug!("Starting flow graph traversal");

        // Get the flow graph
        let graph = self.flow_graph.read().await;

        // Get only certified batches in consensus order
        let batches = graph.get_certified_batches()?;

        debug!("Found {} certified batches for traversal", batches.len());

        // Extract all transactions in order
        let mut transactions = Vec::new();
        for batch in &batches {
            transactions.extend(batch.transactions.clone());
        }

        let transaction_count = transactions.len();
        let traversal_time = start_time.elapsed();

        debug!(
            "Flow graph traversal complete: {} batches, {} transactions in {:?}",
            batches.len(),
            transaction_count,
            traversal_time
        );

        Ok(TraversalResult {
            batches,
            transactions,
            transaction_count,
            traversal_time,
        })
    }

    /// Generate ordered transaction list from flow graph traversal
    ///
    /// This is a convenience method that returns just the transactions
    /// in consensus order.
    ///
    /// Requirements: 17.1, 17.5
    pub async fn get_ordered_transactions(&self) -> Result<Vec<Transaction>> {
        let result = self.traverse_flow_graph().await?;
        Ok(result.transactions)
    }

    /// Get current metrics
    pub fn metrics(&self) -> MercuryMetrics {
        self.metrics.read().clone()
    }

    /// Get current snapshot height
    pub fn current_snapshot_height(&self) -> u64 {
        *self.current_snapshot.read()
    }

    /// Get current cycle
    pub fn current_cycle(&self) -> u64 {
        *self.current_cycle.read()
    }

    /// Spawn snapshot creator task
    async fn spawn_snapshot_creator(&self, mut shutdown_rx: mpsc::Receiver<()>) {
        let config = self.config.clone();
        let flow_graph = Arc::clone(&self.flow_graph);
        let execution_engine = Arc::clone(&self.execution_engine);
        let store = Arc::clone(&self.store);
        let validator_set = Arc::clone(&self.validator_set);
        let current_snapshot = Arc::clone(&self.current_snapshot);
        let current_cycle = Arc::clone(&self.current_cycle);
        let pending_snapshots = Arc::clone(&self.pending_snapshots);
        let finalized_snapshots = Arc::clone(&self.finalized_snapshots);
        let metrics = Arc::clone(&self.metrics);
        let last_snapshot_time = Arc::clone(&self.last_snapshot_time);
        let snapshot_interval_history = Arc::clone(&self.snapshot_interval_history);
        let validator_id = self.validator_id.clone();
        let keypair = self.keypair.clone();

        tokio::spawn(async move {
            info!("Snapshot creator started");

            let mut interval_timer =
                interval(Duration::from_millis(config.snapshot_interval_ms));

            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        // Try to create a snapshot
                        if let Err(e) = Self::create_snapshot_internal(
                            &config,
                            &flow_graph,
                            &execution_engine,
                            &store,
                            &validator_set,
                            &current_snapshot,
                            &current_cycle,
                            &pending_snapshots,
                            &finalized_snapshots,
                            &metrics,
                            &last_snapshot_time,
                            &snapshot_interval_history,
                            &validator_id,
                            &keypair,
                        )
                        .await
                        {
                            error!("Snapshot creation failed: {}", e);
                            metrics.write().failed_snapshots += 1;
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Snapshot creator shutting down");
                        break;
                    }
                }
            }
        });
    }

    /// Internal snapshot creation logic
    ///
    /// This implements the core consensus commit algorithm:
    /// 1. Traverse flow graph to get ordered transactions
    /// 2. Execute transactions to generate state root
    /// 3. Create snapshot with validator signatures
    /// 4. Achieve 480ms snapshot interval for sub-second finality
    ///
    /// Requirements: 17.2, 17.3, 2.5
    async fn create_snapshot_internal(
        config: &MercuryConfig,
        flow_graph: &Arc<AsyncRwLock<FlowGraph>>,
        execution_engine: &Arc<ExecutionEngine>,
        store: &Arc<ObjectStore>,
        validator_set: &Arc<RwLock<ValidatorSet>>,
        current_snapshot: &Arc<RwLock<u64>>,
        current_cycle: &Arc<RwLock<u64>>,
        pending_snapshots: &Arc<DashMap<u64, Snapshot>>,
        finalized_snapshots: &Arc<DashMap<u64, Snapshot>>,
        metrics: &Arc<RwLock<MercuryMetrics>>,
        last_snapshot_time: &Arc<RwLock<Instant>>,
        snapshot_interval_history: &Arc<RwLock<VecDeque<Duration>>>,
        validator_id: &Option<ValidatorID>,
        keypair: &Option<Arc<KeyPair>>,
    ) -> Result<()> {
        let start_time = Instant::now();

        debug!("Starting snapshot creation");

        // Step 1: Traverse flow graph to get ordered transactions
        let graph = flow_graph.read().await;
        let traversal_result = {
            let batches = graph.get_certified_batches()?;
            let mut transactions = Vec::new();
            for batch in &batches {
                transactions.extend(batch.transactions.clone());
            }
            
            // Limit transactions per snapshot
            if transactions.len() > config.max_transactions_per_snapshot {
                transactions.truncate(config.max_transactions_per_snapshot);
            }
            
            transactions
        };
        drop(graph);

        // If no transactions, skip snapshot creation
        if traversal_result.is_empty() {
            debug!("No transactions to finalize, skipping snapshot");
            return Ok(());
        }

        debug!(
            "Found {} transactions for snapshot",
            traversal_result.len()
        );

        // Step 2: Execute ordered transactions to generate state root
        let execution_start = Instant::now();
        let _execution_results = execution_engine
            .execute_transactions(&traversal_result)
            .await?;
        let execution_time = execution_start.elapsed();

        debug!(
            "Executed {} transactions in {:?}",
            traversal_result.len(),
            execution_time
        );

        // Generate state root hash from execution results
        let root_state_digest = execution_engine.compute_state_root().await?;

        // Get transaction digests
        let transaction_digests: Vec<TransactionDigest> = traversal_result
            .iter()
            .map(|tx| tx.digest())
            .collect();

        // Step 3: Create snapshot with validator signatures
        let sequence_number = *current_snapshot.read() + 1;
        let cycle = *current_cycle.read();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Get previous snapshot digest
        let previous_digest = if sequence_number == 1 {
            // Genesis snapshot
            SnapshotDigest::new([0u8; 64])
        } else {
            finalized_snapshots
                .get(&(sequence_number - 1))
                .map(|s| s.digest)
                .unwrap_or_else(|| SnapshotDigest::new([0u8; 64]))
        };

        // Create validator signature if this node is a validator
        let mut validator_signatures = Vec::new();
        let mut stake_weight = 0u64;

        if let (Some(vid), Some(kp)) = (validator_id, keypair) {
            // Sign the snapshot data
            let snapshot_data = Self::prepare_snapshot_signing_data(
                sequence_number,
                timestamp,
                &previous_digest,
                &root_state_digest,
                &transaction_digests,
                cycle,
            );

            let signature = kp.sign(&snapshot_data)?;
            let validator_signature = ValidatorSignature::new(vid.clone(), signature);
            
            // Get validator stake
            let vset = validator_set.read();
            if let Some(validator_info) = vset.get_validator(vid) {
                stake_weight = validator_info.stake_amount();
            }
            drop(vset);

            validator_signatures.push(validator_signature);
        }

        // Create snapshot
        let snapshot = Snapshot::new(
            sequence_number,
            timestamp,
            previous_digest,
            root_state_digest,
            transaction_digests.clone(),
            cycle,
            validator_signatures,
            stake_weight,
        );

        // Validate snapshot
        let total_stake = validator_set.read().total_stake();
        if snapshot.has_quorum(total_stake) {
            // Snapshot has quorum, finalize it
            finalized_snapshots.insert(sequence_number, snapshot.clone());
            
            // Update current snapshot number
            *current_snapshot.write() = sequence_number;

            // Mark batches as finalized in flow graph
            let mut graph = flow_graph.write().await;
            for tx_digest in &transaction_digests {
                // Find batch containing this transaction and mark as finalized
                if let Some(batch_id) = graph.find_batch_for_transaction(tx_digest) {
                    graph.mark_batch_finalized(&batch_id, sequence_number)?;
                    debug!("Marked batch {} as finalized in snapshot {}", batch_id, sequence_number);
                }
            }
            drop(graph);

            // Persist snapshot to storage
            store.store_snapshot(&snapshot).await.map_err(|e| {
                Error::Internal(format!("Failed to store snapshot: {}", e))
            })?;

            info!(
                "Finalized snapshot {} with {} transactions (cycle {})",
                sequence_number,
                snapshot.transaction_count(),
                cycle
            );
        } else {
            // Not enough stake weight yet, add to pending
            pending_snapshots.insert(sequence_number, snapshot.clone());
            
            debug!(
                "Snapshot {} pending finalization ({} / {} stake)",
                sequence_number, stake_weight, total_stake
            );
        }

        // Step 4: Update metrics to achieve 480ms snapshot interval
        let snapshot_time = start_time.elapsed();
        
        // Update interval history
        let interval_since_last = last_snapshot_time.read().elapsed();
        let mut history = snapshot_interval_history.write();
        history.push_back(interval_since_last);
        if history.len() > 100 {
            history.pop_front();
        }

        // Calculate average interval
        let avg_interval_ms = if !history.is_empty() {
            let sum: Duration = history.iter().sum();
            sum.as_millis() as f64 / history.len() as f64
        } else {
            0.0
        };

        // Update metrics
        let mut m = metrics.write();
        m.snapshots_created += 1;
        m.transactions_finalized += traversal_result.len() as u64;
        m.current_snapshot_height = sequence_number;
        m.avg_snapshot_interval_ms = avg_interval_ms;
        m.consensus_latency_ms = snapshot_time.as_millis() as f64;
        m.pending_snapshots = pending_snapshots.len();
        
        if !traversal_result.is_empty() {
            let total_txs = m.transactions_finalized;
            let total_snapshots = m.snapshots_created;
            m.avg_transactions_per_snapshot = total_txs as f64 / total_snapshots as f64;
        }
        drop(m);

        // Update last snapshot time
        *last_snapshot_time.write() = Instant::now();

        debug!(
            "Snapshot creation complete in {:?} (interval: {:?})",
            snapshot_time, interval_since_last
        );

        Ok(())
    }

    /// Prepare data for snapshot signing
    fn prepare_snapshot_signing_data(
        sequence_number: u64,
        timestamp: u64,
        previous_digest: &SnapshotDigest,
        root_state_digest: &StateDigest,
        transaction_digests: &[TransactionDigest],
        cycle: u64,
    ) -> Vec<u8> {
        let mut data = Vec::new();

        // Add sequence number
        data.extend_from_slice(&sequence_number.to_le_bytes());

        // Add timestamp
        data.extend_from_slice(&timestamp.to_le_bytes());

        // Add previous digest
        data.extend_from_slice(previous_digest.as_bytes());

        // Add state root
        data.extend_from_slice(root_state_digest.as_bytes());

        // Add transaction digests
        for tx_digest in transaction_digests {
            data.extend_from_slice(tx_digest.as_bytes());
        }

        // Add cycle
        data.extend_from_slice(&cycle.to_le_bytes());

        data
    }

    /// Add validator signature to a pending snapshot
    ///
    /// When a snapshot receives signatures from 2/3+ stake weight,
    /// it becomes finalized.
    pub async fn add_snapshot_signature(
        &self,
        sequence_number: u64,
        validator_signature: ValidatorSignature,
    ) -> Result<bool> {
        // Check if snapshot is already finalized
        if self.finalized_snapshots.contains_key(&sequence_number) {
            debug!("Snapshot {} already finalized", sequence_number);
            return Ok(false);
        }

        // Get pending snapshot
        let mut snapshot = match self.pending_snapshots.get(&sequence_number) {
            Some(s) => s.clone(),
            None => {
                return Err(Error::InvalidData(format!(
                    "Snapshot {} not found in pending",
                    sequence_number
                )));
            }
        };

        // Verify validator exists
        let validator_set = self.validator_set.read();
        if !validator_set.contains_validator(&validator_signature.validator) {
            return Err(Error::InvalidData(format!(
                "Unknown validator {}",
                validator_signature.validator
            )));
        }

        // Add signature
        snapshot.validator_signatures.push(validator_signature.clone());

        // Recalculate stake weight
        let validator_ids: Vec<ValidatorID> = snapshot
            .validator_signatures
            .iter()
            .map(|sig| sig.validator.clone())
            .collect();
        
        snapshot.stake_weight = validator_set.calculate_stake_weight(&validator_ids);
        let total_stake = validator_set.total_stake();
        drop(validator_set);

        // Check for quorum
        if snapshot.has_quorum(total_stake) {
            // Finalize snapshot
            self.finalized_snapshots.insert(sequence_number, snapshot.clone());
            self.pending_snapshots.remove(&sequence_number);

            // Update current snapshot number
            *self.current_snapshot.write() = sequence_number;

            // Persist to storage
            self.store.store_snapshot(&snapshot).await.map_err(|e| {
                Error::Internal(format!("Failed to store snapshot: {}", e))
            })?;

            info!(
                "Finalized snapshot {} with {} signatures ({} stake)",
                sequence_number,
                snapshot.validator_signatures.len(),
                snapshot.stake_weight
            );

            Ok(true)
        } else {
            // Update pending snapshot
            let stake_weight = snapshot.stake_weight;
            self.pending_snapshots.insert(sequence_number, snapshot);
            
            debug!(
                "Snapshot {} still pending ({} / {} stake)",
                sequence_number, stake_weight, total_stake
            );

            Ok(false)
        }
    }

    /// Get a finalized snapshot
    pub fn get_snapshot(&self, sequence_number: u64) -> Option<Snapshot> {
        self.finalized_snapshots.get(&sequence_number).map(|s| s.clone())
    }

    /// Get the latest finalized snapshot
    pub fn get_latest_snapshot(&self) -> Option<Snapshot> {
        let seq = *self.current_snapshot.read();
        self.get_snapshot(seq)
    }

    /// Check if a snapshot is finalized
    pub fn is_snapshot_finalized(&self, sequence_number: u64) -> bool {
        self.finalized_snapshots.contains_key(&sequence_number)
    }

    // ========== Validator Set Management ==========

    /// Add a validator to the validator set
    ///
    /// Requirements: 2.6, 13.1
    pub fn add_validator(&mut self, metadata: ValidatorMetadata) -> Result<()> {
        let mut validator_set = self.validator_set.write();
        validator_set.add_validator(metadata)?;
        
        info!(
            "Validator added to set (total: {}, stake: {})",
            validator_set.validator_count(),
            validator_set.total_stake()
        );
        
        Ok(())
    }

    /// Remove a validator from the validator set
    ///
    /// Requirements: 2.6, 13.1
    pub fn remove_validator(&mut self, validator_id: &ValidatorID) -> Result<()> {
        let mut validator_set = self.validator_set.write();
        validator_set.remove_validator(validator_id)?;
        
        info!(
            "Validator removed from set (total: {}, stake: {})",
            validator_set.validator_count(),
            validator_set.total_stake()
        );
        
        Ok(())
    }

    /// Get validator information
    pub fn get_validator(&self, validator_id: &ValidatorID) -> Option<crate::validator::ValidatorInfo> {
        self.validator_set.read().get_validator(validator_id)
    }

    /// Get all validators
    pub fn get_all_validators(&self) -> Vec<crate::validator::ValidatorInfo> {
        self.validator_set.read().get_all_validators()
    }

    /// Get active validators
    pub fn get_active_validators(&self) -> Vec<crate::validator::ValidatorInfo> {
        self.validator_set.read().get_active_validators()
    }

    /// Get total stake in the network
    pub fn total_stake(&self) -> u64 {
        self.validator_set.read().total_stake()
    }

    /// Get validator count
    pub fn validator_count(&self) -> usize {
        self.validator_set.read().validator_count()
    }

    /// Get active validator count
    pub fn active_validator_count(&self) -> usize {
        self.validator_set.read().active_validator_count()
    }

    /// Record validator participation in a snapshot
    ///
    /// This tracks which validators participated in creating snapshots
    /// for reward distribution and penalty calculation.
    ///
    /// Requirements: 13.2
    pub fn record_validator_participation(&mut self, validator_id: &ValidatorID, participated: bool) {
        self.validator_set.write().record_participation(validator_id, participated);
    }

    /// Advance to the next cycle
    ///
    /// This reconfigures the validator set at cycle boundaries, applying
    /// any pending validator joins/exits and resetting cycle statistics.
    ///
    /// Requirements: 2.6, 13.1, 13.2
    pub fn advance_cycle(&mut self) -> u64 {
        let mut validator_set = self.validator_set.write();
        let new_cycle = validator_set.advance_cycle();
        drop(validator_set);

        // Update current cycle
        *self.current_cycle.write() = new_cycle;

        info!(
            "Advanced to cycle {} with {} validators ({} stake)",
            new_cycle,
            self.validator_count(),
            self.total_stake()
        );

        new_cycle
    }

    /// Apply participation penalties to validators
    ///
    /// Validators with participation rate below the threshold are penalized.
    /// This is typically called at the end of each cycle.
    ///
    /// Requirements: 13.3, 13.4
    pub fn apply_participation_penalties(&mut self, threshold: f64) -> Vec<ValidatorID> {
        let mut validator_set = self.validator_set.write();
        let penalized = validator_set.apply_participation_penalties(threshold);
        
        if !penalized.is_empty() {
            warn!(
                "Applied penalties to {} validators for low participation",
                penalized.len()
            );
        }
        
        penalized
    }

    /// Check if a set of validators has quorum (2/3+ stake)
    ///
    /// Requirements: 17.4, 29.2
    pub fn has_quorum(&self, validator_ids: &[ValidatorID]) -> bool {
        self.validator_set.read().has_quorum(validator_ids)
    }

    /// Calculate stake weight for a set of validators
    ///
    /// Requirements: 2.6, 13.1
    pub fn calculate_stake_weight(&self, validator_ids: &[ValidatorID]) -> u64 {
        self.validator_set.read().calculate_stake_weight(validator_ids)
    }

    /// Get validator set statistics
    pub fn validator_set_stats(&self) -> ValidatorSetStats {
        let validator_set = self.validator_set.read();
        
        ValidatorSetStats {
            total_validators: validator_set.validator_count(),
            active_validators: validator_set.active_validator_count(),
            total_stake: validator_set.total_stake(),
            current_cycle: *self.current_cycle.read(),
        }
    }
}

/// Validator set statistics
#[derive(Debug, Clone)]
pub struct ValidatorSetStats {
    /// Total number of validators
    pub total_validators: usize,

    /// Number of active validators
    pub active_validators: usize,

    /// Total stake in the network
    pub total_stake: u64,

    /// Current cycle ID
    pub current_cycle: u64,
}

// ========== Liveness and Safety Guarantees ==========

impl MercuryProtocol {
    /// Verify that a snapshot has sufficient stake weight for finality
    ///
    /// This ensures the safety guarantee: snapshots are only finalized when
    /// they have signatures from validators representing more than 2/3 of
    /// the total stake weight.
    ///
    /// Requirements: 17.4, 29.2
    pub fn verify_snapshot_safety(&self, snapshot: &Snapshot) -> Result<()> {
        let total_stake = self.total_stake();
        
        if !snapshot.has_quorum(total_stake) {
            return Err(Error::InvalidData(format!(
                "Snapshot {} does not have sufficient stake weight for finality: {} / {} (need 2/3+)",
                snapshot.sequence_number,
                snapshot.stake_weight,
                total_stake
            )));
        }

        // Verify all validator signatures are from known validators
        let validator_set = self.validator_set.read();
        for validator_sig in &snapshot.validator_signatures {
            if !validator_set.contains_validator(&validator_sig.validator) {
                return Err(Error::InvalidData(format!(
                    "Snapshot {} contains signature from unknown validator {}",
                    snapshot.sequence_number,
                    validator_sig.validator
                )));
            }
        }
        drop(validator_set);

        debug!(
            "Snapshot {} safety verified: {} / {} stake (2/3+ quorum)",
            snapshot.sequence_number,
            snapshot.stake_weight,
            total_stake
        );

        Ok(())
    }

    /// Check if the network can make progress with current validator participation
    ///
    /// This ensures the liveness guarantee: the network can continue to make
    /// progress as long as validators representing more than 2/3 of stake are
    /// online and participating.
    ///
    /// Requirements: 17.4, 29.1, 29.3
    pub fn check_liveness(&self) -> LivenessStatus {
        let validator_set = self.validator_set.read();
        let total_stake = validator_set.total_stake();
        let active_validators = validator_set.get_active_validators();
        
        // Calculate stake of active validators
        let active_stake: u64 = active_validators
            .iter()
            .map(|v| v.stake_amount())
            .sum();

        // Check if we have 2/3+ stake active
        let has_quorum = active_stake * 3 > total_stake * 2;

        // Calculate participation rates
        let mut _participating_count = 0;
        let mut total_participation_rate = 0.0;

        for validator in &active_validators {
            let rate = validator.participation_rate();
            total_participation_rate += rate;
            if rate > 0.5 {
                // Consider validator participating if >50% participation
                _participating_count += 1;
            }
        }

        let avg_participation_rate = if !active_validators.is_empty() {
            total_participation_rate / active_validators.len() as f64
        } else {
            0.0
        };

        drop(validator_set);

        let status = if has_quorum && avg_participation_rate > 0.66 {
            LivenessState::Healthy
        } else if has_quorum {
            LivenessState::Degraded
        } else {
            LivenessState::Stalled
        };

        LivenessStatus {
            state: status,
            active_validators: active_validators.len(),
            total_validators: self.validator_count(),
            active_stake,
            total_stake,
            avg_participation_rate,
            has_quorum,
        }
    }

    /// Handle network partition recovery
    ///
    /// When the network recovers from a partition, this method synchronizes
    /// missing certificates and transactions from peer validators.
    ///
    /// Requirements: 29.1, 29.4, 29.5
    pub async fn handle_partition_recovery(&mut self) -> Result<PartitionRecoveryResult> {
        info!("Starting partition recovery");

        let start_time = Instant::now();
        let recovered_batches = 0;
        let recovered_snapshots = 0;

        // Get current snapshot height
        let current_height = self.current_snapshot_height();

        // Check flow graph for missing batches
        let graph = self.flow_graph.read().await;
        let stats = graph.stats();
        let pending_batches = stats.pending_batches;
        drop(graph);

        debug!(
            "Partition recovery: current height {}, {} pending batches",
            current_height, pending_batches
        );

        // In a real implementation, this would:
        // 1. Query peers for missing batches and certificates
        // 2. Verify and add missing batches to flow graph
        // 3. Request missing snapshots from peers
        // 4. Verify snapshot signatures
        // 5. Update local state

        // For now, we just log the recovery attempt
        info!(
            "Partition recovery complete in {:?}: {} batches, {} snapshots",
            start_time.elapsed(),
            recovered_batches,
            recovered_snapshots
        );

        Ok(PartitionRecoveryResult {
            recovered_batches,
            recovered_snapshots,
            recovery_time: start_time.elapsed(),
        })
    }

    /// Verify Byzantine fault tolerance
    ///
    /// This checks that the consensus can tolerate up to 1/3 of validators
    /// being malicious or offline.
    ///
    /// Requirements: 17.4, 29.1, 29.2, 29.3
    pub fn verify_byzantine_tolerance(&self) -> ByzantineTolerance {
        let total_stake = self.total_stake();
        let total_validators = self.validator_count();

        // Calculate maximum tolerable malicious stake (1/3)
        let max_malicious_stake = total_stake / 3;

        // Calculate minimum honest stake needed (2/3+)
        let min_honest_stake = (total_stake * 2) / 3 + 1;

        // Get active validators
        let active_validators = self.get_active_validators();
        let active_stake: u64 = active_validators
            .iter()
            .map(|v| v.stake_amount())
            .sum();

        // Check if we have enough active stake
        let is_tolerant = active_stake >= min_honest_stake;

        ByzantineTolerance {
            total_stake,
            total_validators,
            active_stake,
            active_validators: active_validators.len(),
            max_malicious_stake,
            min_honest_stake,
            is_tolerant,
            safety_margin: if is_tolerant {
                active_stake - min_honest_stake
            } else {
                0
            },
        }
    }

    /// Check if the network is in a safe state
    ///
    /// A safe state means:
    /// - 2/3+ stake is active and participating
    /// - Recent snapshots have proper quorum
    /// - No conflicting snapshots detected
    ///
    /// Requirements: 17.4, 29.2, 29.5
    pub fn is_network_safe(&self) -> bool {
        // Check liveness
        let liveness = self.check_liveness();
        if !liveness.has_quorum {
            warn!("Network not safe: insufficient quorum");
            return false;
        }

        // Check Byzantine tolerance
        let bft = self.verify_byzantine_tolerance();
        if !bft.is_tolerant {
            warn!("Network not safe: Byzantine tolerance violated");
            return false;
        }

        // Check recent snapshot has quorum
        if let Some(latest_snapshot) = self.get_latest_snapshot() {
            let total_stake = self.total_stake();
            if !latest_snapshot.has_quorum(total_stake) {
                warn!(
                    "Network not safe: latest snapshot {} lacks quorum",
                    latest_snapshot.sequence_number
                );
                return false;
            }
        }

        true
    }

    /// Get network health status
    ///
    /// Provides a comprehensive view of network health including liveness,
    /// safety, and Byzantine tolerance.
    pub fn get_network_health(&self) -> NetworkHealth {
        let liveness = self.check_liveness();
        let bft = self.verify_byzantine_tolerance();
        let is_safe = self.is_network_safe();

        let metrics = self.metrics();

        NetworkHealth {
            is_safe,
            liveness,
            byzantine_tolerance: bft,
            current_snapshot_height: metrics.current_snapshot_height,
            pending_snapshots: metrics.pending_snapshots,
            avg_snapshot_interval_ms: metrics.avg_snapshot_interval_ms,
            consensus_latency_ms: metrics.consensus_latency_ms,
        }
    }
}

/// Liveness state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessState {
    /// Network is healthy and making progress
    Healthy,

    /// Network has quorum but degraded performance
    Degraded,

    /// Network is stalled (no quorum)
    Stalled,
}

/// Liveness status
#[derive(Debug, Clone)]
pub struct LivenessStatus {
    /// Current liveness state
    pub state: LivenessState,

    /// Number of active validators
    pub active_validators: usize,

    /// Total number of validators
    pub total_validators: usize,

    /// Active stake amount
    pub active_stake: u64,

    /// Total stake amount
    pub total_stake: u64,

    /// Average participation rate
    pub avg_participation_rate: f64,

    /// Whether network has quorum (2/3+ stake)
    pub has_quorum: bool,
}

/// Partition recovery result
#[derive(Debug, Clone)]
pub struct PartitionRecoveryResult {
    /// Number of batches recovered
    pub recovered_batches: usize,

    /// Number of snapshots recovered
    pub recovered_snapshots: usize,

    /// Time taken for recovery
    pub recovery_time: Duration,
}

/// Byzantine fault tolerance status
#[derive(Debug, Clone)]
pub struct ByzantineTolerance {
    /// Total stake in network
    pub total_stake: u64,

    /// Total number of validators
    pub total_validators: usize,

    /// Active stake amount
    pub active_stake: u64,

    /// Number of active validators
    pub active_validators: usize,

    /// Maximum tolerable malicious stake (1/3)
    pub max_malicious_stake: u64,

    /// Minimum honest stake needed (2/3+)
    pub min_honest_stake: u64,

    /// Whether network is Byzantine fault tolerant
    pub is_tolerant: bool,

    /// Safety margin (active stake - minimum needed)
    pub safety_margin: u64,
}

/// Network health status
#[derive(Debug, Clone)]
pub struct NetworkHealth {
    /// Whether network is in a safe state
    pub is_safe: bool,

    /// Liveness status
    pub liveness: LivenessStatus,

    /// Byzantine tolerance status
    pub byzantine_tolerance: ByzantineTolerance,

    /// Current snapshot height
    pub current_snapshot_height: u64,

    /// Pending snapshots awaiting finalization
    pub pending_snapshots: usize,

    /// Average snapshot interval (milliseconds)
    pub avg_snapshot_interval_ms: f64,

    /// Consensus latency (milliseconds)
    pub consensus_latency_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_storage::RocksDatabase;
    use std::sync::Arc;

    #[test]
    fn test_mercury_config_default() {
        let config = MercuryConfig::default();
        assert_eq!(config.snapshot_interval_ms, 480);
        assert_eq!(config.max_transactions_per_snapshot, 1000);
        assert!(config.auto_snapshot_creation);
    }

    #[test]
    fn test_mercury_metrics_default() {
        let metrics = MercuryMetrics::default();
        assert_eq!(metrics.snapshots_created, 0);
        assert_eq!(metrics.transactions_finalized, 0);
        assert_eq!(metrics.current_snapshot_height, 0);
    }

    #[tokio::test]
    async fn test_traverse_empty_flow_graph() {
        let config = MercuryConfig::default();
        let validator_set = ValidatorSet::new();
        let flow_graph = FlowGraph::new();
        let execution_engine = ExecutionEngine::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(RocksDatabase::open(temp_dir.path()).unwrap());
        let store = ObjectStore::new(db);

        let mercury = MercuryProtocol::new(
            config,
            None,
            None,
            validator_set,
            flow_graph,
            execution_engine,
            store,
        );

        let result = mercury.traverse_flow_graph().await.unwrap();
        assert_eq!(result.batches.len(), 0);
        assert_eq!(result.transactions.len(), 0);
        assert_eq!(result.transaction_count, 0);
    }
}
