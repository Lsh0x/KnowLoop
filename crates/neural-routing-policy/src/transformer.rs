//! Decision Transformer — GPT-2 style sequence model for trajectory prediction.
//!
//! Input: (return-to-go, state, action) tuples
//! Output: next action prediction
//!
//! ~3M parameters, optimized for CPU inference (<15ms).

use serde::{Deserialize, Serialize};

/// Configuration for the Decision Transformer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTransformerConfig {
    /// State/embedding dimension (256d for decision vectors).
    pub state_dim: usize,
    /// Action vocabulary size.
    pub action_dim: usize,
    /// Hidden dimension for transformer layers.
    pub hidden_dim: usize,
    /// Number of transformer layers.
    pub num_layers: usize,
    /// Number of attention heads.
    pub num_heads: usize,
    /// Maximum context length (trajectory steps).
    pub max_context_len: usize,
    /// Dropout rate.
    pub dropout: f64,
}

impl Default for DecisionTransformerConfig {
    fn default() -> Self {
        Self {
            state_dim: 256,
            action_dim: 50, // ~50 distinct MCP actions
            hidden_dim: 128,
            num_layers: 4,
            num_heads: 4,
            max_context_len: 20,
            dropout: 0.1,
        }
    }
}

/// Decision Transformer — sequence model for action prediction.
///
/// Placeholder for Phase 3 implementation.
pub struct DecisionTransformer {
    pub config: DecisionTransformerConfig,
}

impl DecisionTransformer {
    pub fn new(config: DecisionTransformerConfig) -> Self {
        Self { config }
    }
}
