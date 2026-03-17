//! Neural Routing Core — types, traits, and storage for trajectory-based route learning.
//!
//! This crate provides the foundational data structures shared by all neural-routing crates:
//! - `Trajectory` / `TrajectoryNode` / `DecisionVector` — the core data model
//! - `TrajectoryStore` trait — async CRUD + vector search abstraction
//! - `RewardStrategy` trait — pluggable credit assignment strategies
//! - `Neo4jTrajectoryStore` — concrete storage implementation

pub mod models;
pub mod traits;
pub mod validation;
pub mod reward;
pub mod store;
pub mod error;

pub use models::*;
pub use traits::*;
pub use validation::*;
pub use reward::*;
pub use store::Neo4jTrajectoryStore;
pub use error::NeuralRoutingError;
