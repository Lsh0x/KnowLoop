//! Policy Network for neural route learning.
//!
//! Implements:
//! - Decision Transformer (~3M params, GPT-2 style) — primary policy
//! - CQL (Conservative Q-Learning) — offline RL fallback
//! - EWC (Elastic Weight Consolidation) — continual learning without catastrophic forgetting
//!
//! Built on candle (HuggingFace) — pure Rust, no Python dependency.
//! Always compiled (build full), activation controlled at runtime via settings.

pub mod transformer;
pub mod cql;
pub mod ewc;
pub mod training;

/// Re-export key types
pub use transformer::DecisionTransformer;
pub use cql::CQLPolicy;
pub use training::TrainingConfig;
