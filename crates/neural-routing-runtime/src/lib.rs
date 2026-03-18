//! Neural Routing Runtime — orchestration layer.
//!
//! Brings together all neural-routing crates into a unified runtime:
//! - `CpuGuard` — circuit breaker for CPU protection
//! - `DualTrackRouter` — Policy Net + NN Router fallback
//! - `InferenceEngine` — timeout-bounded inference orchestration
//! - Settings-based activation (runtime control, no feature flags)

pub mod collector;
pub mod config;
pub mod cpu_guard;
pub mod dual_track;

pub use collector::{CollectorEvent, DecisionRecord, TrajectoryCollector};
pub use config::NeuralRoutingConfig;
pub use cpu_guard::CpuGuard;
pub use dual_track::DualTrackRouter;

// Re-export core types so consumers only need neural-routing-runtime
pub use neural_routing_core::{
    create_reward_strategy, error as routing_error, DecisionContext, DecisionVectorBuilder,
    NNRoute, Neo4jTrajectoryStore, NodeFeatures, RewardConfig, RewardStrategy, Router, SessionMeta,
    ToolUsage, TouchedEntity, Trajectory, TrajectoryFilter, TrajectoryNode, TrajectoryStats,
    TrajectoryStore, SOURCE_EMBED_DIM, TOTAL_DIM,
};

// Re-export NN types needed by handlers
pub use neural_routing_nn::metrics::NNMetricsSnapshot;
