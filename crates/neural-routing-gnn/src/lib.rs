//! GNN models for neural route learning.
//!
//! Implements R-GCN (Relational Graph Convolutional Network) and GraphSAGE
//! for encoding the knowledge graph into node embeddings used by the policy network.
//!
//! Built on candle (HuggingFace) — pure Rust, no Python dependency.
//! Always compiled (build full), activation controlled at runtime via settings.

pub mod message_passing;
pub mod rgcn;
pub mod graph_sage;
pub mod encoder;

/// Re-export key types
pub use encoder::GraphEncoder;
