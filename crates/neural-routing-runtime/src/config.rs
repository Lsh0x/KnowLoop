//! Neural routing configuration — runtime settings (no feature flags).

use serde::{Deserialize, Serialize};

use neural_routing_nn::NNConfig;
use neural_routing_policy::TrainingConfig;

use crate::cpu_guard::CpuGuardConfig;

/// Top-level neural routing configuration.
///
/// Maps to `neural_routing:` section in config.yaml.
/// All fields have sensible defaults — the system works out of the box.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralRoutingConfig {
    /// Master switch — enable/disable neural routing entirely.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Routing mode: "nn" (Nearest Neighbor only) or "full" (Policy Net + NN fallback).
    #[serde(default = "default_mode")]
    pub mode: RoutingMode,

    /// Training configuration.
    #[serde(default)]
    pub training: TrainingConfig,

    /// Inference configuration.
    #[serde(default)]
    pub inference: InferenceConfig,

    /// Collection configuration.
    #[serde(default)]
    pub collection: CollectionConfig,

    /// Nearest Neighbor router configuration.
    #[serde(default)]
    pub nn: NNConfig,

    /// CPU guard configuration.
    #[serde(default)]
    pub cpu_guard: CpuGuardSettings,
}

impl Default for NeuralRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: RoutingMode::NN,
            training: TrainingConfig::default(),
            inference: InferenceConfig::default(),
            collection: CollectionConfig::default(),
            nn: NNConfig::default(),
            cpu_guard: CpuGuardSettings::default(),
        }
    }
}

/// Routing mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RoutingMode {
    /// Nearest Neighbor only — zero ML, immediate.
    NN,
    /// Full pipeline: Policy Net + NN Router fallback.
    Full,
}

/// Inference settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Maximum time budget for inference in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Fall back to NN Router if the policy net times out or is OOD.
    #[serde(default = "default_true")]
    pub nn_fallback: bool,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 15,
            nn_fallback: true,
        }
    }
}

/// Trajectory collection settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Enable trajectory collection (always recommended).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Flush batch size — trajectories are buffered and flushed in batches.
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            buffer_size: 50,
        }
    }
}

/// CPU guard settings (serializable subset of CpuGuardConfig).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuGuardSettings {
    /// Pause threshold (default: 80%).
    #[serde(default = "default_pause_threshold")]
    pub pause_threshold: f32,
    /// Resume threshold (default: 50%).
    #[serde(default = "default_resume_threshold")]
    pub resume_threshold: f32,
    /// Poll interval in seconds (default: 2).
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

impl Default for CpuGuardSettings {
    fn default() -> Self {
        Self {
            pause_threshold: 80.0,
            resume_threshold: 50.0,
            poll_interval_secs: 2,
        }
    }
}

impl From<CpuGuardSettings> for CpuGuardConfig {
    fn from(s: CpuGuardSettings) -> Self {
        Self {
            pause_threshold: s.pause_threshold,
            resume_threshold: s.resume_threshold,
            poll_interval: std::time::Duration::from_secs(s.poll_interval_secs),
        }
    }
}

// Default value helpers for serde
fn default_true() -> bool { true }
fn default_mode() -> RoutingMode { RoutingMode::NN }
fn default_timeout_ms() -> u64 { 15 }
fn default_buffer_size() -> usize { 50 }
fn default_pause_threshold() -> f32 { 80.0 }
fn default_resume_threshold() -> f32 { 50.0 }
fn default_poll_interval_secs() -> u64 { 2 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NeuralRoutingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.mode, RoutingMode::NN);
        assert_eq!(config.inference.timeout_ms, 15);
        assert!(config.collection.enabled);
        assert_eq!(config.nn.top_k, 5);
    }

    #[test]
    fn test_deserialize_yaml() {
        let yaml = r#"
enabled: true
mode: nn
training:
  mode: manual
  max_threads: 2
inference:
  timeout_ms: 20
  nn_fallback: true
collection:
  enabled: true
  buffer_size: 100
nn:
  top_k: 10
  min_similarity: 0.8
  max_route_age_days: 60
  cache_capacity: 1000
  cache_ttl_secs: 7200
"#;
        let config: NeuralRoutingConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.mode, RoutingMode::NN);
        assert_eq!(config.inference.timeout_ms, 20);
        assert_eq!(config.nn.top_k, 10);
        assert_eq!(config.collection.buffer_size, 100);
    }

    #[test]
    fn test_deserialize_full_mode() {
        let yaml = "mode: full";
        let config: NeuralRoutingConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.mode, RoutingMode::Full);
    }
}
