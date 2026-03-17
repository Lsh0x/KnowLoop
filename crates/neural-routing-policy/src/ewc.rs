//! EWC (Elastic Weight Consolidation) — continual learning.
//!
//! Prevents catastrophic forgetting when re-training on new trajectories
//! by penalizing changes to parameters important for previous tasks.

use serde::{Deserialize, Serialize};

/// Configuration for EWC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EWCConfig {
    /// Lambda — importance weighting for the EWC penalty.
    pub lambda: f64,
    /// Number of samples used to estimate Fisher information.
    pub fisher_samples: usize,
}

impl Default for EWCConfig {
    fn default() -> Self {
        Self {
            lambda: 5000.0,
            fisher_samples: 200,
        }
    }
}

/// EWC module — prevents catastrophic forgetting.
///
/// Placeholder for Phase 3 implementation.
pub struct EWCRegularizer {
    pub config: EWCConfig,
}

impl EWCRegularizer {
    pub fn new(config: EWCConfig) -> Self {
        Self { config }
    }
}
