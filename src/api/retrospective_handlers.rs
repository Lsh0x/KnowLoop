//! API handlers for Task Retrospective operations.
//!
//! Provides endpoints:
//! - `GET /api/retrospectives` — List retrospectives with filters
//! - `GET /api/retrospectives/{id}` — Get single retrospective
//! - `GET /api/tasks/{task_id}/retrospective` — Get retrospective for a task
//! - `GET /api/retrospectives/insights` — Aggregated insights

use crate::api::handlers::{AppError, OrchestratorState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Query parameters for `GET /api/retrospectives`.
#[derive(Debug, Deserialize)]
pub struct ListRetrospectivesQuery {
    pub project_id: Option<Uuid>,
    pub outcome: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Response for retrospective list.
#[derive(Debug, Serialize)]
pub struct ListRetrospectivesResponse {
    pub items: Vec<crate::retrospective::models::TaskRetrospective>,
    pub total: usize,
}

/// Query parameters for `GET /api/retrospectives/insights`.
#[derive(Debug, Deserialize)]
pub struct InsightsQuery {
    pub project_id: Uuid,
    pub limit: Option<i64>,
}

/// Aggregated insights from retrospective history.
#[derive(Debug, Serialize)]
pub struct InsightsResponse {
    /// Total retrospectives analyzed
    pub total_retrospectives: usize,
    /// Success rate (0.0–1.0)
    pub success_rate: f64,
    /// Average duration in seconds
    pub avg_duration_secs: f64,
    /// Average cost in USD
    pub avg_cost_usd: f64,
    /// Average confidence score
    pub avg_confidence: f64,
    /// Files with highest failure correlation
    pub risky_files: Vec<RiskyFile>,
    /// Most common error patterns
    pub common_errors: Vec<CommonError>,
}

/// A file with a high failure correlation.
#[derive(Debug, Serialize)]
pub struct RiskyFile {
    pub path: String,
    pub failure_rate: f64,
    pub occurrences: usize,
}

/// A commonly occurring error pattern.
#[derive(Debug, Serialize)]
pub struct CommonError {
    pub error: String,
    pub count: usize,
}

// ============================================================================
// Handlers
// ============================================================================

/// List retrospectives with optional filters.
///
/// GET /api/retrospectives?project_id=...&outcome=...&limit=50&offset=0
pub async fn list_retrospectives(
    State(state): State<OrchestratorState>,
    Query(query): Query<ListRetrospectivesQuery>,
) -> Result<Json<ListRetrospectivesResponse>, AppError> {
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    let items = state
        .orchestrator
        .neo4j()
        .list_retrospectives(query.project_id, query.outcome.as_deref(), limit, offset)
        .await
        .map_err(AppError::Internal)?;

    let total = items.len();
    Ok(Json(ListRetrospectivesResponse { items, total }))
}

/// Get a single retrospective by ID.
///
/// GET /api/retrospectives/{id}
pub async fn get_retrospective(
    State(state): State<OrchestratorState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::retrospective::models::TaskRetrospective>, AppError> {
    state
        .orchestrator
        .neo4j()
        .get_task_retrospective(id)
        .await
        .map_err(AppError::Internal)?
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Retrospective not found".into()))
}

/// Get the retrospective for a specific task.
///
/// GET /api/tasks/{task_id}/retrospective
pub async fn get_task_retrospective(
    State(state): State<OrchestratorState>,
    Path(task_id): Path<Uuid>,
) -> Result<Json<crate::retrospective::models::TaskRetrospective>, AppError> {
    state
        .orchestrator
        .neo4j()
        .get_retrospective_for_task(task_id)
        .await
        .map_err(AppError::Internal)?
        .map(Json)
        .ok_or_else(|| AppError::NotFound("No retrospective for this task".into()))
}

/// Get aggregated insights from retrospective history.
///
/// GET /api/retrospectives/insights?project_id=...
pub async fn get_insights(
    State(state): State<OrchestratorState>,
    Query(query): Query<InsightsQuery>,
) -> Result<Json<InsightsResponse>, AppError> {
    let limit = query.limit.unwrap_or(200);

    let retros = state
        .orchestrator
        .neo4j()
        .list_retrospectives(Some(query.project_id), None, limit, 0)
        .await
        .map_err(AppError::Internal)?;

    if retros.is_empty() {
        return Ok(Json(InsightsResponse {
            total_retrospectives: 0,
            success_rate: 0.0,
            avg_duration_secs: 0.0,
            avg_cost_usd: 0.0,
            avg_confidence: 0.0,
            risky_files: Vec::new(),
            common_errors: Vec::new(),
        }));
    }

    let total = retros.len();
    let successes = retros.iter().filter(|r| r.outcome.is_success()).count();
    let success_rate = successes as f64 / total as f64;
    let avg_duration = retros.iter().map(|r| r.duration_secs).sum::<f64>() / total as f64;
    let avg_cost = retros.iter().map(|r| r.cost_usd).sum::<f64>() / total as f64;
    let avg_confidence = retros.iter().map(|r| r.confidence_score).sum::<f64>() / total as f64;

    // Compute file failure rates from retrospectives
    let mut file_counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for retro in &retros {
        let is_failure = !retro.outcome.is_success();
        for file in &retro.files_modified {
            let entry = file_counts.entry(file.clone()).or_insert((0, 0));
            entry.0 += 1; // total
            if is_failure {
                entry.1 += 1; // failures
            }
        }
    }

    let mut risky_files: Vec<RiskyFile> = file_counts
        .into_iter()
        .filter(|(_, (total, failures))| *total >= 2 && *failures > 0)
        .map(|(path, (total, failures))| RiskyFile {
            path,
            failure_rate: failures as f64 / total as f64,
            occurrences: total,
        })
        .collect();
    risky_files.sort_by(|a, b| {
        b.failure_rate
            .partial_cmp(&a.failure_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    risky_files.truncate(10);

    // Compute common errors
    let mut error_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for retro in &retros {
        if let Some(ref err) = retro.last_error {
            // Truncate to first 100 chars for grouping
            let key = if err.len() > 100 {
                format!("{}...", &err[..100])
            } else {
                err.clone()
            };
            *error_counts.entry(key).or_insert(0) += 1;
        }
    }

    let mut common_errors: Vec<CommonError> = error_counts
        .into_iter()
        .map(|(error, count)| CommonError { error, count })
        .collect();
    common_errors.sort_by(|a, b| b.count.cmp(&a.count));
    common_errors.truncate(10);

    Ok(Json(InsightsResponse {
        total_retrospectives: total,
        success_rate,
        avg_duration_secs: avg_duration,
        avg_cost_usd: avg_cost,
        avg_confidence,
        risky_files,
        common_errors,
    }))
}
