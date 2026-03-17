//! API handlers for trajectory storage and retrieval.
//!
//! Endpoints:
//! - GET  /api/trajectories          — list with filters
//! - GET  /api/trajectories/stats    — statistics
//! - GET  /api/trajectories/:id      — get by ID with nodes
//! - POST /api/trajectories/similar  — vector similarity search

use super::handlers::{AppError, OrchestratorState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use neural_routing_runtime::{Trajectory, TrajectoryFilter, TrajectoryStats};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Query parameters for GET /api/trajectories
#[derive(Debug, Deserialize)]
pub struct ListTrajectoriesQuery {
    pub session_id: Option<String>,
    pub min_reward: Option<f64>,
    pub max_reward: Option<f64>,
    pub min_steps: Option<usize>,
    pub max_steps: Option<usize>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Request body for POST /api/trajectories/similar
#[derive(Debug, Deserialize)]
pub struct SimilarSearchRequest {
    /// Query embedding (256d).
    pub embedding: Vec<f32>,
    /// Number of results to return (default: 10).
    pub top_k: Option<usize>,
    /// Minimum cosine similarity threshold (default: 0.7).
    pub min_similarity: Option<f32>,
}

/// Response for similarity search
#[derive(Debug, Serialize)]
pub struct SimilarResult {
    pub trajectory: Trajectory,
    pub similarity: f64,
}

/// Wrapper for list response
#[derive(Debug, Serialize)]
pub struct TrajectoryListResponse {
    pub trajectories: Vec<Trajectory>,
    pub count: usize,
}

/// Wrapper for single trajectory
#[derive(Debug, Serialize)]
pub struct TrajectoryDetailResponse {
    pub trajectory: Trajectory,
}

/// Wrapper for stats
#[derive(Debug, Serialize)]
pub struct TrajectoryStatsResponse {
    pub stats: TrajectoryStats,
}

/// Wrapper for similar results
#[derive(Debug, Serialize)]
pub struct SimilarSearchResponse {
    pub results: Vec<SimilarResult>,
    pub count: usize,
}

// ============================================================================
// Helper
// ============================================================================

fn get_store(
    state: &OrchestratorState,
) -> Result<&dyn neural_routing_runtime::TrajectoryStore, AppError> {
    state
        .trajectory_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            AppError::BadRequest(
                "Trajectory store not available (neural routing may be disabled)".to_string(),
            )
        })
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /api/trajectories
///
/// List trajectories with optional filters.
pub async fn list_trajectories(
    State(state): State<OrchestratorState>,
    Query(query): Query<ListTrajectoriesQuery>,
) -> Result<Json<TrajectoryListResponse>, AppError> {
    let store = get_store(&state)?;

    let filter = TrajectoryFilter {
        session_id: query.session_id,
        min_reward: query.min_reward,
        max_reward: query.max_reward,
        from_date: None,
        to_date: None,
        min_steps: query.min_steps,
        max_steps: query.max_steps,
        limit: Some(query.limit.unwrap_or(50).min(200)),
        offset: query.offset,
    };

    let trajectories = store
        .list_trajectories(&filter)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to list trajectories: {e}")))?;

    let count = trajectories.len();
    Ok(Json(TrajectoryListResponse {
        trajectories,
        count,
    }))
}

/// GET /api/trajectories/stats
///
/// Get trajectory statistics.
pub async fn get_stats(
    State(state): State<OrchestratorState>,
) -> Result<Json<TrajectoryStatsResponse>, AppError> {
    let store = get_store(&state)?;

    let stats = store
        .get_stats()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to get trajectory stats: {e}")))?;

    Ok(Json(TrajectoryStatsResponse { stats }))
}

/// GET /api/trajectories/:id
///
/// Get a single trajectory with all its nodes.
pub async fn get_trajectory(
    State(state): State<OrchestratorState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TrajectoryDetailResponse>, AppError> {
    let store = get_store(&state)?;

    let trajectory = store
        .get_trajectory(&id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to get trajectory: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("Trajectory {id} not found")))?;

    Ok(Json(TrajectoryDetailResponse { trajectory }))
}

/// POST /api/trajectories/similar
///
/// Find trajectories similar to a given embedding.
pub async fn search_similar(
    State(state): State<OrchestratorState>,
    Json(req): Json<SimilarSearchRequest>,
) -> Result<Json<SimilarSearchResponse>, AppError> {
    let store = get_store(&state)?;

    // Validate embedding dimension (must be 256d)
    if req.embedding.len() != neural_routing_runtime::TOTAL_DIM {
        return Err(AppError::BadRequest(format!(
            "Embedding must be {}d, got {}d",
            neural_routing_runtime::TOTAL_DIM,
            req.embedding.len()
        )));
    }

    let top_k = req.top_k.unwrap_or(10).min(100);
    let min_similarity = req.min_similarity.unwrap_or(0.7);

    let results = store
        .search_similar(&req.embedding, top_k, min_similarity)
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "Failed to search similar trajectories: {e}"
            ))
        })?;

    let count = results.len();
    let results: Vec<SimilarResult> = results
        .into_iter()
        .map(|(trajectory, similarity)| SimilarResult {
            trajectory,
            similarity,
        })
        .collect();

    Ok(Json(SimilarSearchResponse { results, count }))
}
