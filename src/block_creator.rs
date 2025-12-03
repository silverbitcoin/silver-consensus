//! Block creation from finalized snapshots
//!
//! This module handles converting finalized snapshots into blocks
//! and storing them in RocksDB.

use silver_core::Snapshot;
use silver_storage::{Block, BlockStore};
use std::sync::Arc;
use tracing::{debug, info};

/// Block creator for converting snapshots to blocks
pub struct BlockCreator {
    /// Block store for persistence
    block_store: Arc<BlockStore>,
}

impl BlockCreator {
    /// Create a new block creator
    pub fn new(block_store: Arc<BlockStore>) -> Self {
        info!("Initializing BlockCreator");
        Self { block_store }
    }

    /// Create a block from a finalized snapshot
    ///
    /// Converts snapshot data into a block structure and stores it in RocksDB.
    ///
    /// # Arguments
    /// * `snapshot` - The finalized snapshot
    /// * `validator_address` - Address of the validator creating the block
    ///
    /// # Returns
    /// The created block
    pub fn create_block_from_snapshot(
        &self,
        snapshot: &Snapshot,
        validator_address: Vec<u8>,
    ) -> Result<Block, String> {
        debug!(
            "Creating block from snapshot sequence: {}",
            snapshot.sequence_number
        );

        // Extract transaction digests from snapshot
        let transaction_digests: Vec<Vec<u8>> = snapshot
            .transactions
            .iter()
            .map(|td| td.as_bytes().to_vec())
            .collect();

        // Create block hash from snapshot digest
        let block_hash = snapshot.digest.as_bytes().to_vec();

        // Create parent hash from previous snapshot digest
        let parent_hash = snapshot.previous_digest.as_bytes().to_vec();

        // Create block
        let block = Block::new(
            snapshot.sequence_number,
            block_hash,
            parent_hash,
            snapshot.timestamp,
            transaction_digests,
            validator_address,
            0, // gas_used - will be calculated from transaction effects
            0, // gas_limit - will be set from consensus config
        );

        debug!(
            "Created block {} with {} transactions",
            block.number,
            block.transactions.len()
        );

        Ok(block)
    }

    /// Store a block in RocksDB
    ///
    /// # Arguments
    /// * `block` - The block to store
    ///
    /// # Returns
    /// Ok if successful, Err with message otherwise
    pub fn store_block(&self, block: &Block) -> Result<(), String> {
        debug!("Storing block {}", block.number);

        self.block_store
            .store_block(block)
            .map_err(|e| format!("Failed to store block: {}", e))?;

        info!("Block {} stored successfully", block.number);
        Ok(())
    }

    /// Get a block by number
    ///
    /// # Arguments
    /// * `number` - Block number
    ///
    /// # Returns
    /// Ok(Some(block)) if found, Ok(None) if not found, Err on database error
    pub fn get_block(&self, number: u64) -> Result<Option<Block>, String> {
        debug!("Retrieving block {}", number);

        self.block_store
            .get_block(number)
            .map_err(|e| format!("Failed to get block: {}", e))
    }

    /// Get the latest block number
    ///
    /// # Returns
    /// Latest block number or 0 if no blocks exist
    pub fn get_latest_block_number(&self) -> Result<u64, String> {
        self.block_store
            .get_latest_block_number()
            .map_err(|e| format!("Failed to get latest block number: {}", e))
    }

    /// Create and store a block from snapshot
    ///
    /// Convenience method that creates and stores in one call.
    ///
    /// # Arguments
    /// * `snapshot` - The finalized snapshot
    /// * `validator_address` - Address of the validator
    ///
    /// # Returns
    /// The created and stored block
    pub fn create_and_store_block(
        &self,
        snapshot: &Snapshot,
        validator_address: Vec<u8>,
    ) -> Result<Block, String> {
        let block = self.create_block_from_snapshot(snapshot, validator_address)?;
        self.store_block(&block)?;
        Ok(block)
    }

    /// Verify block integrity
    ///
    /// Checks that a block is valid and can be stored.
    ///
    /// # Arguments
    /// * `block` - The block to verify
    ///
    /// # Returns
    /// Ok if valid, Err with message if invalid
    pub fn verify_block(&self, block: &Block) -> Result<(), String> {
        // Check block number is valid
        if block.number == 0 {
            // Genesis block
            return Ok(());
        }

        // Get previous block
        let prev_block = self
            .get_block(block.number - 1)
            .map_err(|e| format!("Failed to get previous block: {}", e))?
            .ok_or_else(|| "Previous block not found".to_string())?;

        // Verify parent hash matches
        if block.parent_hash != prev_block.hash {
            return Err(format!(
                "Block {} parent hash mismatch",
                block.number
            ));
        }

        // Verify timestamp is not before previous block
        if block.timestamp < prev_block.timestamp {
            return Err(format!(
                "Block {} timestamp is before previous block",
                block.number
            ));
        }

        // Verify block hash is not empty
        if block.hash.is_empty() {
            return Err("Block hash cannot be empty".to_string());
        }

        // Verify block hash is 32 bytes
        if block.hash.len() != 32 {
            return Err(format!(
                "Block hash must be 32 bytes, got {}",
                block.hash.len()
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_creation() {
        // This would require setting up a test database
        // Actual tests would be in integration tests
    }
}
