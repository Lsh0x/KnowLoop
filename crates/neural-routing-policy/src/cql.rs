//! CQL (Conservative Q-Learning) — offline RL policy.
//!
//! Fallback when the Decision Transformer is OOD (out-of-distribution).
//! Conservative: penalizes Q-values for unseen state-action pairs.

use serde::{Deserialize, Serialize};

/// Configuration for CQL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CQLConfig {
    /// State dimension.
    pub state_dim: usize,
    /// Action dimension.
    pub action_dim: usize,
    /// Hidden dimension.
    pub hidden_dim: usize,
    /// CQL alpha (conservatism coefficient).
    pub alpha: f64,
    /// Discount factor.
    pub gamma: f64,
}

impl Default for CQLConfig {
    fn default() -> Self {
        Self {
            state_dim: 256,
            action_dim: 50,
            hidden_dim: 128,
            alpha: 1.0,
            gamma: 0.99,
        }
    }
}

/// CQL Policy — conservative offline RL.
///
/// Placeholder for Phase 3 implementation.
pub struct CQLPolicy {
    pub config: CQLConfig,
}

impl CQLPolicy {
    pub fn new(config: CQLConfig) -> Self {
        Self { config }
    }
}
