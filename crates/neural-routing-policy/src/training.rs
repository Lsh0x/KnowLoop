//! Training configuration and pipeline for the policy network.

use serde::{Deserialize, Serialize};

/// Training configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainingConfig {
    /// Training mode: "automatic" or "manual".
    pub mode: String,
    /// Number of trajectories before auto-training triggers (automatic mode).
    pub auto_trigger_threshold: usize,
    /// Maximum threads for training.
    pub max_threads: usize,
    /// Cron schedule for automatic training (e.g., "0 2 * * *").
    pub schedule: String,
    /// Learning rate.
    pub learning_rate: f64,
    /// Batch size.
    pub batch_size: usize,
    /// Number of epochs.
    pub epochs: usize,
    /// Gradient clipping norm.
    pub max_grad_norm: f64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            mode: "manual".to_string(),
            auto_trigger_threshold: 1000,
            max_threads: 2,
            schedule: "0 2 * * *".to_string(),
            learning_rate: 1e-4,
            batch_size: 32,
            epochs: 10,
            max_grad_norm: 1.0,
        }
    }
}
