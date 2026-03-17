//! Nearest Neighbor Router — the permanent fallback for neural route learning.
//!
//! Given a query embedding, finds the K most similar past trajectories and
//! extracts the best route. Zero ML dependencies, works from ~100 trajectories.

pub mod benchmark;
pub mod metrics;
pub mod router;
pub mod scoring;

pub use metrics::NNMetrics;
pub use router::{NNConfig, NNRouter};
