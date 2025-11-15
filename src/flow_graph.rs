//! Flow graph structure
//!
//! The flow graph is a directed acyclic graph (DAG) of transaction batches
//! where edges represent causal dependencies through cryptographic links.
//!
//! Key features:
//! - Maintains causal ordering of batches
//! - Cryptographic links between batches (Blake3-512)
//! - Fast batch propagation (50ms target)
//! - Efficient traversal for consensus ordering
//! - Concurrent access with fine-grained locking

use silver_core::{BatchID, Certificate, Error, Result, TransactionBatch, ValidatorID};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, warn};

/// Flow graph error types
#[derive(Debug, thiserror::Error)]
pub enum FlowGraphError {
    /// Batch already exists in the graph
    #[error("Batch {0} already exists in flow graph")]
    DuplicateBatch(BatchID),

    /// Batch not found in the graph
    #[error("Batch {0} not found in flow graph")]
    BatchNotFound(BatchID),

    /// Invalid batch structure
    #[error("Invalid batch: {0}")]
    InvalidBatch(String),

    /// Cycle detected in flow graph
    #[error("Cycle detected in flow graph")]
    CycleDetected,

    /// Missing dependency
    #[error("Missing dependency batch {0}")]
    MissingDependency(BatchID),
}

/// Flow graph vertex representing a batch
#[derive(Debug, Clone)]
struct FlowGraphVertex {
    /// The transaction batch
    batch: TransactionBatch,

    /// Certificate for this batch (if certified)
    certificate: Option<Certificate>,

    /// Parent batches (dependencies)
    parents: Vec<BatchID>,

    /// Child batches (dependents)
    children: Vec<BatchID>,

    /// Timestamp when batch was added to graph
    added_at: Instant,

    /// Whether this batch has been finalized
    finalized: bool,
}

impl FlowGraphVertex {
    fn new(batch: TransactionBatch) -> Self {
        let parents = batch.previous_batches.clone();

        Self {
            batch,
            certificate: None,
            parents,
            children: Vec::new(),
            added_at: Instant::now(),
            finalized: false,
        }
    }

    fn is_certified(&self) -> bool {
        self.certificate.is_some()
    }

    fn add_child(&mut self, child_id: BatchID) {
        if !self.children.contains(&child_id) {
            self.children.push(child_id);
        }
    }

    fn set_certificate(&mut self, certificate: Certificate) {
        self.certificate = Some(certificate);
    }

    fn mark_finalized(&mut self) {
        self.finalized = true;
    }
}

/// Flow graph statistics
#[derive(Debug, Clone, Default)]
pub struct FlowGraphStats {
    /// Total batches in graph
    pub total_batches: usize,

    /// Certified batches
    pub certified_batches: usize,

    /// Finalized batches
    pub finalized_batches: usize,

    /// Pending batches (not certified)
    pub pending_batches: usize,

    /// Average batch propagation time (milliseconds)
    pub avg_propagation_time_ms: f64,

    /// Maximum depth of the graph
    pub max_depth: usize,

    /// Number of tips (batches with no children)
    pub tip_count: usize,
}

/// Flow graph implementation
///
/// The flow graph maintains a DAG of transaction batches with cryptographic
/// links representing causal dependencies. It supports efficient traversal
/// for consensus ordering and fast batch propagation.
pub struct FlowGraph {
    /// Vertices indexed by batch ID
    vertices: Arc<DashMap<BatchID, FlowGraphVertex>>,

    /// Root batches (no parents)
    roots: Arc<RwLock<HashSet<BatchID>>>,

    /// Tips (batches with no children yet)
    tips: Arc<RwLock<HashSet<BatchID>>>,

    /// Batch propagation times (for metrics)
    propagation_times: Arc<RwLock<VecDeque<Duration>>>,

    /// Maximum propagation time history size
    max_propagation_history: usize,
}

impl FlowGraph {
    /// Create a new flow graph
    pub fn new() -> Self {
        Self {
            vertices: Arc::new(DashMap::new()),
            roots: Arc::new(RwLock::new(HashSet::new())),
            tips: Arc::new(RwLock::new(HashSet::new())),
            propagation_times: Arc::new(RwLock::new(VecDeque::new())),
            max_propagation_history: 1000,
        }
    }

    /// Add a batch to the flow graph
    pub async fn add_batch(&mut self, batch: TransactionBatch) -> Result<()> {
        let batch_id = batch.batch_id;

        // Check if batch already exists
        if self.vertices.contains_key(&batch_id) {
            return Err(Error::InvalidData(format!(
                "Batch {} already exists in flow graph",
                batch_id
            )));
        }

        // Validate batch
        batch.validate()?;

        // Check for missing dependencies
        for parent_id in &batch.previous_batches {
            if !self.vertices.contains_key(parent_id) {
                warn!(
                    "Batch {} references missing parent {}, will add when available",
                    batch_id, parent_id
                );
            }
        }

        // Create vertex
        let vertex = FlowGraphVertex::new(batch.clone());
        let parents = vertex.parents.clone();
        let added_at = vertex.added_at;

        // Insert vertex
        self.vertices.insert(batch_id, vertex);

        // Update parent-child relationships
        for parent_id in &parents {
            if let Some(mut parent) = self.vertices.get_mut(parent_id) {
                parent.add_child(batch_id);

                // Remove parent from tips since it now has a child
                self.tips.write().remove(parent_id);
            }
        }

        // Update roots and tips
        if parents.is_empty() {
            self.roots.write().insert(batch_id);
        }
        self.tips.write().insert(batch_id);

        // Record propagation time
        let propagation_time = Instant::now().duration_since(added_at);
        let mut times = self.propagation_times.write();
        times.push_back(propagation_time);
        if times.len() > self.max_propagation_history {
            times.pop_front();
        }

        debug!(
            "Added batch {} to flow graph (parents: {}, propagation: {:?})",
            batch_id,
            parents.len(),
            propagation_time
        );

        Ok(())
    }

    /// Get a batch from the flow graph
    pub fn get_batch(&self, batch_id: &BatchID) -> Option<TransactionBatch> {
        self.vertices.get(batch_id).map(|v| v.batch.clone())
    }

    /// Check if a batch exists in the graph
    pub fn contains_batch(&self, batch_id: &BatchID) -> bool {
        self.vertices.contains_key(batch_id)
    }

    /// Get the certificate for a batch
    pub fn get_certificate(&self, batch_id: &BatchID) -> Option<Certificate> {
        self.vertices
            .get(batch_id)
            .and_then(|v| v.certificate.clone())
    }

    /// Set the certificate for a batch
    pub fn set_certificate(&mut self, batch_id: BatchID, certificate: Certificate) -> Result<()> {
        if let Some(mut vertex) = self.vertices.get_mut(&batch_id) {
            vertex.set_certificate(certificate);
            debug!("Set certificate for batch {}", batch_id);
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Batch {} not found in flow graph",
                batch_id
            )))
        }
    }

    /// Mark a batch as finalized
    pub fn mark_finalized(&mut self, batch_id: &BatchID) -> Result<()> {
        if let Some(mut vertex) = self.vertices.get_mut(batch_id) {
            vertex.mark_finalized();
            debug!("Marked batch {} as finalized", batch_id);
            Ok(())
        } else {
            Err(Error::InvalidData(format!(
                "Batch {} not found in flow graph",
                batch_id
            )))
        }
    }

    /// Get all root batches (no parents)
    pub fn get_roots(&self) -> Vec<BatchID> {
        self.roots.read().iter().copied().collect()
    }

    /// Get all tip batches (no children)
    pub fn get_tips(&self) -> Vec<BatchID> {
        self.tips.read().iter().copied().collect()
    }

    /// Get the latest batch IDs (tips) for creating new batches
    pub fn get_latest_batch_ids(&self, limit: usize) -> Vec<BatchID> {
        let tips = self.tips.read();
        tips.iter().take(limit).copied().collect()
    }

    /// Get children of a batch
    pub fn get_children(&self, batch_id: &BatchID) -> Vec<BatchID> {
        self.vertices
            .get(batch_id)
            .map(|v| v.children.clone())
            .unwrap_or_default()
    }

    /// Get parents of a batch
    pub fn get_parents(&self, batch_id: &BatchID) -> Vec<BatchID> {
        self.vertices
            .get(batch_id)
            .map(|v| v.parents.clone())
            .unwrap_or_default()
    }

    /// Perform topological sort of the flow graph
    ///
    /// Returns batches in causal order (parents before children).
    /// Uses Kahn's algorithm for topological sorting.
    pub fn topological_sort(&self) -> Result<Vec<BatchID>> {
        let mut result = Vec::new();
        let mut in_degree: HashMap<BatchID, usize> = HashMap::new();
        let mut queue = VecDeque::new();

        // Calculate in-degrees
        for entry in self.vertices.iter() {
            let batch_id = *entry.key();
            let vertex = entry.value();

            in_degree.insert(batch_id, vertex.parents.len());

            if vertex.parents.is_empty() {
                queue.push_back(batch_id);
            }
        }

        // Process queue
        while let Some(batch_id) = queue.pop_front() {
            result.push(batch_id);

            // Get children
            if let Some(vertex) = self.vertices.get(&batch_id) {
                for child_id in &vertex.children {
                    if let Some(degree) = in_degree.get_mut(child_id) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(*child_id);
                        }
                    }
                }
            }
        }

        // Check for cycles
        if result.len() != self.vertices.len() {
            return Err(Error::InvalidData(
                "Cycle detected in flow graph".to_string(),
            ));
        }

        Ok(result)
    }

    /// Get batches in deterministic order for consensus
    ///
    /// Uses topological sort with tie-breaking by batch ID hash.
    pub fn get_consensus_order(&self) -> Result<Vec<TransactionBatch>> {
        let mut sorted_ids = self.topological_sort()?;

        // Sort by batch ID for deterministic tie-breaking
        sorted_ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Collect batches
        let mut batches = Vec::new();
        for batch_id in sorted_ids {
            if let Some(batch) = self.get_batch(&batch_id) {
                batches.push(batch);
            }
        }

        Ok(batches)
    }

    /// Get only certified batches in consensus order
    pub fn get_certified_batches(&self) -> Result<Vec<TransactionBatch>> {
        let all_batches = self.get_consensus_order()?;

        Ok(all_batches
            .into_iter()
            .filter(|batch| {
                self.vertices
                    .get(&batch.batch_id)
                    .map(|v| v.is_certified())
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Get flow graph statistics
    pub fn stats(&self) -> FlowGraphStats {
        let total_batches = self.vertices.len();
        let mut certified_batches = 0;
        let mut finalized_batches = 0;
        let mut max_depth = 0;

        for entry in self.vertices.iter() {
            let vertex = entry.value();
            if vertex.is_certified() {
                certified_batches += 1;
            }
            if vertex.finalized {
                finalized_batches += 1;
            }

            // Calculate depth (simple approximation)
            let depth = vertex.parents.len();
            if depth > max_depth {
                max_depth = depth;
            }
        }

        let pending_batches = total_batches - certified_batches;
        let tip_count = self.tips.read().len();

        // Calculate average propagation time
        let times = self.propagation_times.read();
        let avg_propagation_time_ms = if !times.is_empty() {
            let sum: Duration = times.iter().sum();
            sum.as_millis() as f64 / times.len() as f64
        } else {
            0.0
        };

        FlowGraphStats {
            total_batches,
            certified_batches,
            finalized_batches,
            pending_batches,
            avg_propagation_time_ms,
            max_depth,
            tip_count,
        }
    }

    /// Prune finalized batches older than the given duration
    pub fn prune_old_batches(&mut self, retention_duration: Duration) -> usize {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for entry in self.vertices.iter() {
            let batch_id = *entry.key();
            let vertex = entry.value();

            if vertex.finalized && now.duration_since(vertex.added_at) > retention_duration {
                to_remove.push(batch_id);
            }
        }

        let count = to_remove.len();
        for batch_id in to_remove {
            self.vertices.remove(&batch_id);
            self.roots.write().remove(&batch_id);
            self.tips.write().remove(&batch_id);
        }

        if count > 0 {
            info!("Pruned {} old finalized batches from flow graph", count);
        }

        count
    }

    /// Clear all batches from the graph
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.roots.write().clear();
        self.tips.write().clear();
        self.propagation_times.write().clear();
        info!("Cleared flow graph");
    }

    /// Get the number of batches in the graph
    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    /// Check if the graph is empty
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

impl Default for FlowGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silver_core::{Signature, SignatureScheme, SilverAddress, Transaction, ValidatorID};

    fn create_test_batch(
        id: u8,
        previous: Vec<BatchID>,
    ) -> TransactionBatch {
        let validator_id = ValidatorID::new(SilverAddress::new([id; 64]));
        let signature = Signature {
            scheme: SignatureScheme::Dilithium3,
            bytes: vec![0u8; 100],
        };

        TransactionBatch::new(
            vec![], // Empty transactions for testing
            validator_id,
            1000,
            previous,
            signature,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_flow_graph_add_batch() {
        let mut graph = FlowGraph::new();

        let batch = create_test_batch(1, vec![]);
        let batch_id = batch.batch_id;

        assert!(graph.add_batch(batch).await.is_ok());
        assert!(graph.contains_batch(&batch_id));
        assert_eq!(graph.len(), 1);
    }

    #[tokio::test]
    async fn test_flow_graph_parent_child() {
        let mut graph = FlowGraph::new();

        // Add parent batch
        let parent = create_test_batch(1, vec![]);
        let parent_id = parent.batch_id;
        graph.add_batch(parent).await.unwrap();

        // Add child batch
        let child = create_test_batch(2, vec![parent_id]);
        let child_id = child.batch_id;
        graph.add_batch(child).await.unwrap();

        // Verify relationships
        let children = graph.get_children(&parent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], child_id);

        let parents = graph.get_parents(&child_id);
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], parent_id);
    }

    #[tokio::test]
    async fn test_flow_graph_topological_sort() {
        let mut graph = FlowGraph::new();

        // Create a simple DAG: A -> B -> C
        let batch_a = create_test_batch(1, vec![]);
        let id_a = batch_a.batch_id;
        graph.add_batch(batch_a).await.unwrap();

        let batch_b = create_test_batch(2, vec![id_a]);
        let id_b = batch_b.batch_id;
        graph.add_batch(batch_b).await.unwrap();

        let batch_c = create_test_batch(3, vec![id_b]);
        let id_c = batch_c.batch_id;
        graph.add_batch(batch_c).await.unwrap();

        // Get topological order
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted.len(), 3);

        // Verify A comes before B, and B comes before C
        let pos_a = sorted.iter().position(|&id| id == id_a).unwrap();
        let pos_b = sorted.iter().position(|&id| id == id_b).unwrap();
        let pos_c = sorted.iter().position(|&id| id == id_c).unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[tokio::test]
    async fn test_flow_graph_stats() {
        let mut graph = FlowGraph::new();

        let batch = create_test_batch(1, vec![]);
        graph.add_batch(batch).await.unwrap();

        let stats = graph.stats();
        assert_eq!(stats.total_batches, 1);
        assert_eq!(stats.certified_batches, 0);
        assert_eq!(stats.pending_batches, 1);
    }

    #[test]
    fn test_flow_graph_new() {
        let graph = FlowGraph::new();
        assert_eq!(graph.len(), 0);
        assert!(graph.is_empty());
    }
}

