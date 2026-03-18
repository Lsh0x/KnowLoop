//! Error types for the neural routing system.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NeuralRoutingError {
    #[error("Invalid decision vector: {0}")]
    InvalidVector(String),

    #[error("Trajectory not found: {0}")]
    TrajectoryNotFound(String),

    #[error("Storage error: {0}")]
    Storage(#[from] anyhow::Error),

    #[error("Neo4j error: {0}")]
    Neo4j(String),

    #[error("Reward computation error: {0}")]
    RewardError(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, NeuralRoutingError>;
