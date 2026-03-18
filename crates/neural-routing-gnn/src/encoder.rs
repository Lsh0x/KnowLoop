//! Graph Encoder — orchestrates GNN layers to produce node embeddings.

use serde::{Deserialize, Serialize};

/// Configuration for the Graph Encoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEncoderConfig {
    /// Input feature dimension.
    pub input_dim: usize,
    /// Hidden layer dimension.
    pub hidden_dim: usize,
    /// Output embedding dimension.
    pub output_dim: usize,
    /// Number of GNN layers.
    pub num_layers: usize,
    /// Number of relation types in the knowledge graph.
    pub num_relations: usize,
    /// Dropout rate.
    pub dropout: f64,
}

impl Default for GraphEncoderConfig {
    fn default() -> Self {
        Self {
            input_dim: 256,
            hidden_dim: 128,
            output_dim: 256,
            num_layers: 2,
            num_relations: 12, // IMPORTS, CALLS, EXTENDS, etc.
            dropout: 0.1,
        }
    }
}

/// Graph Encoder — produces node embeddings from the knowledge graph.
///
/// Placeholder for Phase 2 implementation.
pub struct GraphEncoder {
    pub config: GraphEncoderConfig,
}

impl GraphEncoder {
    pub fn new(config: GraphEncoderConfig) -> Self {
        Self { config }
    }
}
