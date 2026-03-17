//! API handlers for Neural Routing.
//!
//! These handlers back the MCP `neural_routing` mega-tool and provide
//! the REST surface for neural route learning status and configuration.

use super::handlers::{AppError, OrchestratorState};
use axum::{extract::State, Json};
use neural_routing_runtime::config::{NeuralRoutingConfig, RoutingMode};
use neural_routing_runtime::NNMetricsSnapshot;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Response for GET /api/neural-routing/status
#[derive(Debug, Serialize)]
pub struct NeuralRoutingStatusResponse {
    pub enabled: bool,
    pub mode: String,
    pub cpu_guard_paused: bool,
    pub metrics: NNMetricsSnapshot,
}

/// Response for GET /api/neural-routing/config
#[derive(Debug, Serialize)]
pub struct NeuralRoutingConfigResponse {
    pub config: NeuralRoutingConfig,
}

/// Request body for PUT /api/neural-routing/mode
#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    /// "nn" or "full"
    pub mode: String,
}

/// Request body for PUT /api/neural-routing/config
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub inference_timeout_ms: Option<u64>,
    pub nn_fallback: Option<bool>,
    pub collection_enabled: Option<bool>,
    pub collection_buffer_size: Option<usize>,
    pub nn_top_k: Option<usize>,
    pub nn_min_similarity: Option<f32>,
    pub nn_max_route_age_days: Option<u32>,
}

/// Generic success response
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub ok: bool,
    pub message: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /api/neural-routing/status
///
/// Returns current neural routing status including mode, CPU guard state, and metrics.
pub async fn get_status(
    State(state): State<OrchestratorState>,
) -> Result<Json<NeuralRoutingStatusResponse>, AppError> {
    let router = state.neural_router.read().await;
    let config = router.config();
    let metrics = router.nn_router().metrics().snapshot();

    let mode_str = match config.mode {
        RoutingMode::NN => "nn",
        RoutingMode::Full => "full",
    };

    Ok(Json(NeuralRoutingStatusResponse {
        enabled: config.enabled,
        mode: mode_str.to_string(),
        cpu_guard_paused: router.cpu_guard().is_paused(),
        metrics,
    }))
}

/// GET /api/neural-routing/config
///
/// Returns the full neural routing configuration.
pub async fn get_config(
    State(state): State<OrchestratorState>,
) -> Result<Json<NeuralRoutingConfigResponse>, AppError> {
    let router = state.neural_router.read().await;
    let config = router.config().clone();

    Ok(Json(NeuralRoutingConfigResponse { config }))
}

/// POST /api/neural-routing/enable
///
/// Enable neural routing at runtime.
pub async fn enable(
    State(state): State<OrchestratorState>,
) -> Result<Json<SuccessResponse>, AppError> {
    let mut router = state.neural_router.write().await;
    let mut config = router.config().clone();
    config.enabled = true;
    router.update_config(config);

    tracing::info!("Neural routing enabled via API");

    Ok(Json(SuccessResponse {
        ok: true,
        message: "Neural routing enabled".to_string(),
    }))
}

/// POST /api/neural-routing/disable
///
/// Disable neural routing at runtime.
pub async fn disable(
    State(state): State<OrchestratorState>,
) -> Result<Json<SuccessResponse>, AppError> {
    let mut router = state.neural_router.write().await;
    let mut config = router.config().clone();
    config.enabled = false;
    router.update_config(config);

    tracing::info!("Neural routing disabled via API");

    Ok(Json(SuccessResponse {
        ok: true,
        message: "Neural routing disabled".to_string(),
    }))
}

/// PUT /api/neural-routing/mode
///
/// Set routing mode: "nn" (NN only) or "full" (Policy Net + NN fallback).
pub async fn set_mode(
    State(state): State<OrchestratorState>,
    Json(req): Json<SetModeRequest>,
) -> Result<Json<SuccessResponse>, AppError> {
    let new_mode = match req.mode.as_str() {
        "nn" => RoutingMode::NN,
        "full" => RoutingMode::Full,
        other => {
            return Err(AppError::BadRequest(format!(
                "Invalid mode '{}': expected 'nn' or 'full'",
                other
            )));
        }
    };

    let mut router = state.neural_router.write().await;
    let mut config = router.config().clone();
    config.mode = new_mode.clone();
    router.update_config(config);

    let mode_str = match new_mode {
        RoutingMode::NN => "nn",
        RoutingMode::Full => "full",
    };
    tracing::info!(mode = mode_str, "Neural routing mode changed via API");

    Ok(Json(SuccessResponse {
        ok: true,
        message: format!("Neural routing mode set to '{}'", mode_str),
    }))
}

/// PUT /api/neural-routing/config
///
/// Update neural routing configuration (partial update).
pub async fn update_config(
    State(state): State<OrchestratorState>,
    Json(req): Json<UpdateConfigRequest>,
) -> Result<Json<SuccessResponse>, AppError> {
    let mut router = state.neural_router.write().await;
    let mut config = router.config().clone();

    if let Some(enabled) = req.enabled {
        config.enabled = enabled;
    }
    if let Some(ref mode) = req.mode {
        config.mode = match mode.as_str() {
            "nn" => RoutingMode::NN,
            "full" => RoutingMode::Full,
            other => {
                return Err(AppError::BadRequest(format!(
                    "Invalid mode '{}': expected 'nn' or 'full'",
                    other
                )));
            }
        };
    }
    if let Some(timeout_ms) = req.inference_timeout_ms {
        config.inference.timeout_ms = timeout_ms;
    }
    if let Some(nn_fallback) = req.nn_fallback {
        config.inference.nn_fallback = nn_fallback;
    }
    if let Some(collection_enabled) = req.collection_enabled {
        config.collection.enabled = collection_enabled;
    }
    if let Some(buffer_size) = req.collection_buffer_size {
        config.collection.buffer_size = buffer_size;
    }
    if let Some(top_k) = req.nn_top_k {
        config.nn.top_k = top_k;
    }
    if let Some(min_sim) = req.nn_min_similarity {
        config.nn.min_similarity = min_sim;
    }
    if let Some(max_age) = req.nn_max_route_age_days {
        config.nn.max_route_age_days = max_age;
    }

    router.update_config(config);

    tracing::info!("Neural routing config updated via API");

    Ok(Json(SuccessResponse {
        ok: true,
        message: "Neural routing configuration updated".to_string(),
    }))
}
