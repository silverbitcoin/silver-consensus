//! Cascade mempool implementation
//!
//! The Cascade mempool implements a graph-flow based transaction ordering system
//! where validator workers create batches of transactions that form a directed
//! acyclic graph (DAG) through cryptographic links.
//!
//! Key features:
//! - Worker-based parallel batch creation
//! - Size limits: 500 transactions OR 512KB per batch
//! - Blake3-512 cryptographic links between batches
//! - Automatic batch broadcasting with retry logic
//! - Real-time metrics tracking

use crate::flow_graph::FlowGraph;
use silver_core::{
    BatchID, Certificate, Error, Result, Transaction, TransactionBatch,
    TransactionDigest, ValidatorID, ValidatorSignature,
};
use silver_crypto::KeyPair;
use silver_network::NetworkHandle;
use silver_storage::ObjectStore;

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Maximum transactions per batch
const MAX_TRANSACTIONS_PER_BATCH: usize = 500;

/// Maximum batch size in bytes
const MAX_BATCH_SIZE_BYTES: usize = 512 * 1024; // 512KB

/// Target batch creation interval (milliseconds)
const BATCH_CREATION_INTERVAL_MS: u64 = 50;

/// Maximum retry attempts for batch broadcast
const MAX_BROADCAST_RETRIES: usize = 3;

/// Retry backoff duration (milliseconds)
const RETRY_BACKOFF_MS: u64 = 100;

/// Cascade mempool configuration
#[derive(Debug, Clone)]
pub struct CascadeConfig {
    /// Number of worker threads for batch creation
    pub worker_count: usize,

    /// Maximum transactions per batch
    pub max_transactions_per_batch: usize,

    /// Maximum batch size in bytes
    pub max_batch_size_bytes: usize,

    /// Batch creation interval (milliseconds)
    pub batch_creation_interval_ms: u64,

    /// Enable automatic batch creation
    pub auto_batch_creation: bool,

    /// Maximum pending transactions before backpressure
    pub max_pending_transactions: usize,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            worker_count: 4,
            max_transactions_per_batch: MAX_TRANSACTIONS_PER_BATCH,
            max_batch_size_bytes: MAX_BATCH_SIZE_BYTES,
            batch_creation_interval_ms: BATCH_CREATION_INTERVAL_MS,
            auto_batch_creation: true,
            max_pending_transactions: 100_000,
        }
    }
}

/// Cascade mempool metrics
#[derive(Debug, Clone, Default)]
pub struct CascadeMetrics {
    /// Total batches created
    pub batches_created: u64,

    /// Total transactions batched
    pub transactions_batched: u64,

    /// Average batch size (transactions)
    pub avg_batch_size: f64,

    /// Average batch size (bytes)
    pub avg_batch_size_bytes: f64,

    /// Batch creation rate (batches/second)
    pub batch_creation_rate: f64,

    /// Transaction throughput (tx/second)
    pub transaction_throughput: f64,

    /// Pending transactions count
    pub pending_transactions: usize,

    /// Failed broadcast attempts
    pub failed_broadcasts: u64,

    /// Successful broadcast attempts
    pub successful_broadcasts: u64,
}

/// Cascade mempool state
struct CascadeState {
    /// Pending transactions waiting to be batched
    pending_transactions: VecDeque<Transaction>,

    /// Recently created batches (for deduplication)
    recent_batches: HashMap<BatchID, TransactionBatch>,

    /// Metrics
    metrics: CascadeMetrics,

    /// Last batch creation time
    last_batch_time: Instant,

    /// Batch size history (for averaging)
    batch_size_history: VecDeque<usize>,

    /// Batch byte size history (for averaging)
    batch_byte_size_history: VecDeque<usize>,
}

impl CascadeState {
    fn new() -> Self {
        Self {
            pending_transactions: VecDeque::new(),
            recent_batches: HashMap::new(),
            metrics: CascadeMetrics::default(),
            last_batch_time: Instant::now(),
            batch_size_history: VecDeque::with_capacity(100),
            batch_byte_size_history: VecDeque::with_capacity(100),
        }
    }

    fn update_metrics(&mut self, batch: &TransactionBatch) {
        self.metrics.batches_created += 1;
        self.metrics.transactions_batched += batch.transaction_count() as u64;
        self.metrics.pending_transactions = self.pending_transactions.len();

        // Update batch size history
        self.batch_size_history.push_back(batch.transaction_count());
        if self.batch_size_history.len() > 100 {
            self.batch_size_history.pop_front();
        }

        self.batch_byte_size_history.push_back(batch.size_bytes());
        if self.batch_byte_size_history.len() > 100 {
            self.batch_byte_size_history.pop_front();
        }

        // Calculate averages
        if !self.batch_size_history.is_empty() {
            let sum: usize = self.batch_size_history.iter().sum();
            self.metrics.avg_batch_size = sum as f64 / self.batch_size_history.len() as f64;
        }

        if !self.batch_byte_size_history.is_empty() {
            let sum: usize = self.batch_byte_size_history.iter().sum();
            self.metrics.avg_batch_size_bytes =
                sum as f64 / self.batch_byte_size_history.len() as f64;
        }

        // Calculate rates
        let elapsed = self.last_batch_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.metrics.batch_creation_rate = 1.0 / elapsed;
            self.metrics.transaction_throughput =
                batch.transaction_count() as f64 / elapsed;
        }

        self.last_batch_time = Instant::now();
    }
}

/// Worker handle for batch creation
struct WorkerHandle {
    /// Worker ID
    id: usize,

    /// Shutdown signal sender
    shutdown_tx: mpsc::Sender<()>,
}

/// Cascade mempool implementation
///
/// The Cascade mempool manages transaction batching and flow graph construction
/// for the Mercury Protocol consensus engine.
pub struct CascadeMempool {
    /// Configuration
    config: CascadeConfig,

    /// Validator ID
    validator_id: ValidatorID,

    /// Validator keypair for signing batches
    keypair: Arc<KeyPair>,

    /// Internal state
    state: Arc<RwLock<CascadeState>>,

    /// Flow graph
    flow_graph: Arc<AsyncRwLock<FlowGraph>>,

    /// Network handle for broadcasting
    network: Arc<NetworkHandle>,

    /// Object store
    store: Arc<ObjectStore>,

    /// Worker handles
    workers: Arc<AsyncMutex<Vec<WorkerHandle>>>,

    /// Transaction submission channel
    tx_sender: mpsc::UnboundedSender<Transaction>,

    /// Transaction receiver (for workers)
    tx_receiver: Arc<AsyncMutex<mpsc::UnboundedReceiver<Transaction>>>,

    /// Batch broadcast channel
    batch_sender: mpsc::UnboundedSender<TransactionBatch>,

    /// Batch receiver (for broadcasting)
    batch_receiver: Arc<AsyncMutex<mpsc::UnboundedReceiver<TransactionBatch>>>,
}

impl CascadeMempool {
    /// Create a new Cascade mempool
    pub fn new(
        config: CascadeConfig,
        validator_id: ValidatorID,
        keypair: KeyPair,
        flow_graph: FlowGraph,
        network: NetworkHandle,
        store: ObjectStore,
    ) -> Self {
        let (tx_sender, tx_receiver) = mpsc::unbounded_channel();
        let (batch_sender, batch_receiver) = mpsc::unbounded_channel();

        Self {
            config,
            validator_id,
            keypair: Arc::new(keypair),
            state: Arc::new(RwLock::new(CascadeState::new())),
            flow_graph: Arc::new(AsyncRwLock::new(flow_graph)),
            network: Arc::new(network),
            store: Arc::new(store),
            workers: Arc::new(AsyncMutex::new(Vec::new())),
            tx_sender,
            tx_receiver: Arc::new(AsyncMutex::new(tx_receiver)),
            batch_sender,
            batch_receiver: Arc::new(AsyncMutex::new(batch_receiver)),
        }
    }

    /// Start the mempool workers
    pub async fn start(&self) -> Result<()> {
        info!(
            "Starting Cascade mempool with {} workers",
            self.config.worker_count
        );

        let mut workers = self.workers.lock().await;

        // Start worker threads
        for worker_id in 0..self.config.worker_count {
            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

            let handle = WorkerHandle {
                id: worker_id,
                shutdown_tx,
            };

            // Spawn worker task
            self.spawn_worker(worker_id, shutdown_rx).await;

            workers.push(handle);
        }

        // Start batch broadcaster
        self.spawn_batch_broadcaster().await;

        info!("Cascade mempool started successfully");
        Ok(())
    }

    /// Stop the mempool workers
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Cascade mempool");

        let workers = self.workers.lock().await;

        // Send shutdown signals to all workers
        for worker in workers.iter() {
            if let Err(e) = worker.shutdown_tx.send(()).await {
                warn!("Failed to send shutdown signal to worker {}: {}", worker.id, e);
            }
        }

        info!("Cascade mempool stopped");
        Ok(())
    }

    /// Submit a transaction to the mempool
    pub fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionDigest> {
        // Validate transaction
        transaction.validate()?;

        let digest = transaction.digest();

        // Check if mempool is full
        let state = self.state.read();
        if state.pending_transactions.len() >= self.config.max_pending_transactions {
            return Err(Error::ResourceExhausted(
                "Mempool is full, try again later".to_string(),
            ));
        }
        drop(state);

        // Send to workers
        self.tx_sender.send(transaction).map_err(|e| {
            Error::Internal(format!("Failed to submit transaction: {}", e))
        })?;

        debug!("Transaction {} submitted to mempool", digest);
        Ok(digest)
    }

    /// Get current metrics
    pub fn metrics(&self) -> CascadeMetrics {
        self.state.read().metrics.clone()
    }

    /// Get pending transaction count
    pub fn pending_count(&self) -> usize {
        self.state.read().pending_transactions.len()
    }

    /// Spawn a worker task
    async fn spawn_worker(&self, worker_id: usize, mut shutdown_rx: mpsc::Receiver<()>) {
        let state = Arc::clone(&self.state);
        let config = self.config.clone();
        let validator_id = self.validator_id.clone();
        let keypair = Arc::clone(&self.keypair);
        let flow_graph = Arc::clone(&self.flow_graph);
        let batch_sender = self.batch_sender.clone();
        let tx_receiver = Arc::clone(&self.tx_receiver);

        tokio::spawn(async move {
            info!("Worker {} started", worker_id);

            let mut interval =
                tokio::time::interval(Duration::from_millis(config.batch_creation_interval_ms));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Try to create a batch
                        if let Err(e) = Self::worker_create_batch(
                            worker_id,
                            &state,
                            &config,
                            &validator_id,
                            &keypair,
                            &flow_graph,
                            &batch_sender,
                            &tx_receiver,
                        )
                        .await
                        {
                            error!("Worker {} batch creation failed: {}", worker_id, e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Worker {} shutting down", worker_id);
                        break;
                    }
                }
            }
        });
    }

    /// Worker batch creation logic
    async fn worker_create_batch(
        worker_id: usize,
        state: &Arc<RwLock<CascadeState>>,
        config: &CascadeConfig,
        validator_id: &ValidatorID,
        keypair: &Arc<KeyPair>,
        flow_graph: &Arc<AsyncRwLock<FlowGraph>>,
        batch_sender: &mpsc::UnboundedSender<TransactionBatch>,
        tx_receiver: &Arc<AsyncMutex<mpsc::UnboundedReceiver<Transaction>>>,
    ) -> Result<()> {
        // Collect transactions from the receiver
        let mut transactions = Vec::new();
        let mut total_size = 0usize;

        // Try to receive transactions without blocking
        let mut rx = tx_receiver.lock().await;
        while transactions.len() < config.max_transactions_per_batch
            && total_size < config.max_batch_size_bytes
        {
            match rx.try_recv() {
                Ok(tx) => {
                    let tx_size = tx.size_bytes();
                    if total_size + tx_size > config.max_batch_size_bytes {
                        // Would exceed size limit, put it back
                        state.write().pending_transactions.push_front(tx);
                        break;
                    }
                    total_size += tx_size;
                    transactions.push(tx);
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(Error::Internal("Transaction channel disconnected".to_string()));
                }
            }
        }
        drop(rx);

        // If no transactions, skip batch creation
        if transactions.is_empty() {
            return Ok(());
        }

        // Get previous batches from flow graph
        let graph = flow_graph.read().await;
        let previous_batches = graph.get_latest_batch_ids(10); // Get up to 10 recent batches
        drop(graph);

        // Get current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Sign the batch
        let batch_data = Self::prepare_batch_signing_data(
            &transactions,
            &validator_id.address,
            timestamp,
            &previous_batches,
        );
        let signature = keypair.sign(&batch_data)?;

        // Create batch
        let batch = TransactionBatch::new(
            transactions,
            validator_id.clone(),
            timestamp,
            previous_batches,
            signature,
        )?;

        debug!(
            "Worker {} created batch {} with {} transactions ({} bytes)",
            worker_id,
            batch.batch_id,
            batch.transaction_count(),
            batch.size_bytes()
        );

        // Update metrics
        state.write().update_metrics(&batch);

        // Send batch for broadcasting
        batch_sender.send(batch).map_err(|e| {
            Error::Internal(format!("Failed to send batch for broadcasting: {}", e))
        })?;

        Ok(())
    }

    /// Prepare data for batch signing
    fn prepare_batch_signing_data(
        transactions: &[Transaction],
        author: &silver_core::SilverAddress,
        timestamp: u64,
        previous_batches: &[BatchID],
    ) -> Vec<u8> {
        let mut data = Vec::new();

        // Add author
        data.extend_from_slice(author.as_bytes());

        // Add timestamp
        data.extend_from_slice(&timestamp.to_le_bytes());

        // Add previous batches
        for batch_id in previous_batches {
            data.extend_from_slice(batch_id.as_bytes());
        }

        // Add transaction digests
        for tx in transactions {
            data.extend_from_slice(tx.digest().as_bytes());
        }

        data
    }

    /// Spawn batch broadcaster task
    async fn spawn_batch_broadcaster(&self) {
        let network = Arc::clone(&self.network);
        let flow_graph = Arc::clone(&self.flow_graph);
        let state = Arc::clone(&self.state);
        let batch_receiver = Arc::clone(&self.batch_receiver);

        tokio::spawn(async move {
            info!("Batch broadcaster started");

            let mut rx = batch_receiver.lock().await;

            while let Some(batch) = rx.recv().await {
                debug!("Broadcasting batch {}", batch.batch_id);

                // Add batch to flow graph
                let mut graph = flow_graph.write().await;
                if let Err(e) = graph.add_batch(batch.clone()).await {
                    error!("Failed to add batch to flow graph: {}", e);
                    continue;
                }
                drop(graph);

                // Broadcast batch to all validators with retry
                let mut retries = 0;
                let mut success = false;

                while retries < MAX_BROADCAST_RETRIES && !success {
                    match network.broadcast_batch(&batch).await {
                        Ok(_) => {
                            debug!("Batch {} broadcast successful", batch.batch_id);
                            state.write().metrics.successful_broadcasts += 1;
                            success = true;
                        }
                        Err(e) => {
                            warn!(
                                "Batch {} broadcast failed (attempt {}): {}",
                                batch.batch_id,
                                retries + 1,
                                e
                            );
                            state.write().metrics.failed_broadcasts += 1;
                            retries += 1;

                            if retries < MAX_BROADCAST_RETRIES {
                                // Exponential backoff
                                let backoff = Duration::from_millis(
                                    RETRY_BACKOFF_MS * (1 << retries),
                                );
                                sleep(backoff).await;
                            }
                        }
                    }
                }

                if !success {
                    error!(
                        "Batch {} broadcast failed after {} retries",
                        batch.batch_id, MAX_BROADCAST_RETRIES
                    );
                }
            }

            info!("Batch broadcaster stopped");
        });
    }

    /// Handle incoming batch from another validator
    pub async fn handle_incoming_batch(&self, batch: TransactionBatch) -> Result<()> {
        // Validate batch
        batch.validate()?;

        let batch_id = batch.batch_id;

        // Check if we already have this batch
        let graph = self.flow_graph.read().await;
        if graph.contains_batch(&batch_id) {
            debug!("Batch {} already in flow graph, ignoring", batch_id);
            return Ok(());
        }
        drop(graph);

        // Add to flow graph
        let mut graph = self.flow_graph.write().await;
        graph.add_batch(batch).await?;
        drop(graph);

        debug!("Added incoming batch {} to flow graph", batch_id);
        Ok(())
    }
}

/// Batch certification manager
///
/// Manages the collection of validator signatures for batches and
/// creation of certificates when 2/3+ stake weight is achieved.
pub struct BatchCertifier {
    /// Validator set
    validator_set: Arc<RwLock<crate::validator::ValidatorSet>>,

    /// Flow graph
    flow_graph: Arc<AsyncRwLock<FlowGraph>>,

    /// Pending signatures for batches
    pending_signatures: Arc<DashMap<BatchID, Vec<ValidatorSignature>>>,

    /// Certified batches
    certified_batches: Arc<DashMap<BatchID, Certificate>>,
}

impl BatchCertifier {
    /// Create a new batch certifier
    pub fn new(
        validator_set: Arc<RwLock<crate::validator::ValidatorSet>>,
        flow_graph: Arc<AsyncRwLock<FlowGraph>>,
    ) -> Self {
        Self {
            validator_set,
            flow_graph,
            pending_signatures: Arc::new(DashMap::new()),
            certified_batches: Arc::new(DashMap::new()),
        }
    }

    /// Add a validator signature for a batch
    pub async fn add_signature(
        &self,
        batch_id: BatchID,
        validator_signature: ValidatorSignature,
    ) -> Result<Option<Certificate>> {
        // Verify the batch exists
        let graph = self.flow_graph.read().await;
        if !graph.contains_batch(&batch_id) {
            return Err(Error::InvalidData(format!(
                "Batch {} not found in flow graph",
                batch_id
            )));
        }
        drop(graph);

        // Check if already certified
        if self.certified_batches.contains_key(&batch_id) {
            debug!("Batch {} already certified", batch_id);
            return Ok(None);
        }

        // Verify validator exists
        let validator_set = self.validator_set.read();
        if !validator_set.contains_validator(&validator_signature.validator) {
            return Err(Error::InvalidData(format!(
                "Unknown validator {}",
                validator_signature.validator
            )));
        }

        // Add signature to pending
        self.pending_signatures
            .entry(batch_id)
            .or_insert_with(Vec::new)
            .push(validator_signature.clone());

        // Check if we have enough signatures for a certificate
        let signatures = self.pending_signatures.get(&batch_id).unwrap();
        let validator_ids: Vec<ValidatorID> = signatures
            .iter()
            .map(|sig| sig.validator.clone())
            .collect();

        let stake_weight = validator_set.calculate_stake_weight(&validator_ids);
        let total_stake = validator_set.total_stake();

        drop(validator_set);

        // Check for quorum (2/3+ stake)
        if stake_weight * 3 > total_stake * 2 {
            // Create certificate
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            let certificate = Certificate::new(
                batch_id,
                signatures.clone(),
                stake_weight,
                timestamp,
            );

            // Validate certificate
            certificate.validate(total_stake)?;

            // Store certificate
            self.certified_batches.insert(batch_id, certificate.clone());

            // Update flow graph
            let mut graph = self.flow_graph.write().await;
            graph.set_certificate(batch_id, certificate.clone())?;
            drop(graph);

            // Remove from pending
            self.pending_signatures.remove(&batch_id);

            info!(
                "Created certificate for batch {} with {} signatures ({} stake)",
                batch_id,
                certificate.signature_count(),
                stake_weight
            );

            Ok(Some(certificate))
        } else {
            debug!(
                "Batch {} has {} / {} stake (need 2/3+)",
                batch_id, stake_weight, total_stake
            );
            Ok(None)
        }
    }

    /// Get certificate for a batch
    pub fn get_certificate(&self, batch_id: &BatchID) -> Option<Certificate> {
        self.certified_batches.get(batch_id).map(|c| c.clone())
    }

    /// Check if a batch is certified
    pub fn is_certified(&self, batch_id: &BatchID) -> bool {
        self.certified_batches.contains_key(batch_id)
    }

    /// Get all certified batch IDs
    pub fn get_certified_batch_ids(&self) -> Vec<BatchID> {
        self.certified_batches
            .iter()
            .map(|entry| *entry.key())
            .collect()
    }

    /// Get pending signature count for a batch
    pub fn pending_signature_count(&self, batch_id: &BatchID) -> usize {
        self.pending_signatures
            .get(batch_id)
            .map(|sigs| sigs.len())
            .unwrap_or(0)
    }

    /// Clear old pending signatures
    pub fn clear_old_pending(&self, max_age: Duration) {
        // This would require tracking timestamp of when signatures were added
        // For now, just clear all pending for certified batches
        let certified: Vec<BatchID> = self.certified_batches
            .iter()
            .map(|entry| *entry.key())
            .collect();

        for batch_id in certified {
            self.pending_signatures.remove(&batch_id);
        }
    }

    /// Get certification statistics
    pub fn stats(&self) -> CertificationStats {
        let certified_count = self.certified_batches.len();
        let pending_count = self.pending_signatures.len();

        let total_pending_signatures: usize = self
            .pending_signatures
            .iter()
            .map(|entry| entry.value().len())
            .sum();

        CertificationStats {
            certified_batches: certified_count,
            pending_batches: pending_count,
            total_pending_signatures,
        }
    }
}

/// Certification statistics
#[derive(Debug, Clone, Default)]
pub struct CertificationStats {
    /// Number of certified batches
    pub certified_batches: usize,

    /// Number of batches pending certification
    pub pending_batches: usize,

    /// Total pending signatures across all batches
    pub total_pending_signatures: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cascade_config_default() {
        let config = CascadeConfig::default();
        assert_eq!(config.worker_count, 4);
        assert_eq!(config.max_transactions_per_batch, 500);
        assert_eq!(config.max_batch_size_bytes, 512 * 1024);
    }

    #[test]
    fn test_cascade_metrics_default() {
        let metrics = CascadeMetrics::default();
        assert_eq!(metrics.batches_created, 0);
        assert_eq!(metrics.transactions_batched, 0);
    }

    #[test]
    fn test_cascade_state_new() {
        let state = CascadeState::new();
        assert_eq!(state.pending_transactions.len(), 0);
        assert_eq!(state.recent_batches.len(), 0);
    }

    #[test]
    fn test_certification_stats_default() {
        let stats = CertificationStats::default();
        assert_eq!(stats.certified_batches, 0);
        assert_eq!(stats.pending_batches, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cascade_config_default() {
        let config = CascadeConfig::default();
        assert_eq!(config.worker_count, 4);
        assert_eq!(config.max_transactions_per_batch, 500);
        assert_eq!(config.max_batch_size_bytes, 512 * 1024);
    }

    #[test]
    fn test_cascade_metrics_default() {
        let metrics = CascadeMetrics::default();
        assert_eq!(metrics.batches_created, 0);
        assert_eq!(metrics.transactions_batched, 0);
    }

    #[test]
    fn test_cascade_state_new() {
        let state = CascadeState::new();
        assert_eq!(state.pending_transactions.len(), 0);
        assert_eq!(state.recent_batches.len(), 0);
    }
}
