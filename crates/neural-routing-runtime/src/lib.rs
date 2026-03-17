//! Neural Routing Runtime — orchestration layer.
//!
//! Brings together all neural-routing crates into a unified runtime:
//! - `CpuGuard` — circuit breaker for CPU protection
//! - `DualTrackRouter` — Policy Net + NN Router fallback
//! - `InferenceEngine` — timeout-bounded inference orchestration
//! - Settings-based activation (runtime control, no feature flags)

pub mod cpu_guard;
pub mod config;
pub mod dual_track;

pub use cpu_guard::CpuGuard;
pub use config::NeuralRoutingConfig;
pub use dual_track::DualTrackRouter;
