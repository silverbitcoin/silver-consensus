//! # SilverBitcoin Consensus
//!
//! Mercury Protocol consensus engine with Cascade mempool.
//!
//! This crate implements:
//! - Cascade mempool (graph-flow transaction ordering)
//! - Mercury Protocol (DRP consensus algorithm)
//! - Validator set management
//! - Snapshot creation and certification
//! - Byzantine fault tolerance (up to 1/3 malicious validators)

#![warn(missing_docs, rust_2018_idioms)]
#![forbid(unsafe_code)]

pub mod cascade;
pub mod mercury;
pub mod validator;
pub mod snapshot;
pub mod flow_graph;

pub use cascade::CascadeMempool;
pub use mercury::MercuryProtocol;
pub use validator::{ValidatorSet, ValidatorInfo};
pub use snapshot::{SnapshotManager, SnapshotCertificate};
pub use flow_graph::FlowGraph;
