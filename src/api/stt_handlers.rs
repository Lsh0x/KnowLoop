//! STT API handlers — Speech-to-Text status, transcription, and dictionary management
//!
//! These REST endpoints complement the WebSocket audio streaming.
//! - GET /api/stt/status — check if Murmure is available
//! - POST /api/stt/transcribe — one-shot transcription (for testing / non-streaming use)
//! - GET /api/stt/dictionary/:slug — get project dictionary
//! - PUT /api/stt/dictionary/:slug — save project dictionary
//! - POST /api/stt/dictionary/:slug/rules — add/update a rule
//! - DELETE /api/stt/dictionary/:slug/rules — remove a rule
//! - POST /api/stt/dictionary/:slug/auto-generate — auto-generate from code symbols
//! - GET /api/stt/dictionaries — list all project dictionaries

use crate::api::handlers::{AppError, OrchestratorState};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// STT Status
// ============================================================================

#[derive(Serialize)]
pub struct SttStatusResponse {
    pub available: bool,
    pub grpc_url: Option<String>,
    pub max_audio_duration_secs: u32,
}

/// GET /api/stt/status — Check if Murmure STT sidecar is available
pub async fn stt_status(
    State(state): State<OrchestratorState>,
) -> Result<Json<SttStatusResponse>, AppError> {
    let (available, grpc_url, max_duration) = match &state.murmure_client {
        Some(client) => {
            let available = client.is_available().await;
            (
                available,
                state.stt_config.grpc_url.clone(),
                state.stt_config.max_audio_duration_secs,
            )
        }
        None => (false, None, 0),
    };

    Ok(Json(SttStatusResponse {
        available,
        grpc_url,
        max_audio_duration_secs: max_duration,
    }))
}

// ============================================================================
// One-shot Transcription
// ============================================================================

#[derive(Deserialize)]
pub struct TranscribeRequest {
    /// Base64-encoded audio data (WAV format, 16kHz, mono, 16-bit)
    pub audio_data: String,
    /// Optional language hint (e.g. "fr", "en")
    #[serde(default)]
    pub language: Option<String>,
    /// Whether to apply custom dictionary corrections
    #[serde(default = "default_true")]
    pub use_dictionary: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct TranscribeResponse {
    pub text: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /api/stt/transcribe — One-shot audio transcription
pub async fn stt_transcribe(
    State(state): State<OrchestratorState>,
    Json(request): Json<TranscribeRequest>,
) -> Result<Json<TranscribeResponse>, AppError> {
    let client = state.murmure_client.as_ref().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "STT not configured (Murmure sidecar not available)"
        ))
    })?;

    // Decode base64 audio
    use base64::Engine;
    let audio_bytes = base64::engine::general_purpose::STANDARD
        .decode(&request.audio_data)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64 audio data: {e}")))?;

    // Safety: limit audio size (25MB)
    if audio_bytes.len() > 25 * 1024 * 1024 {
        return Err(AppError::BadRequest(
            "Audio data too large (max 25MB)".to_string(),
        ));
    }

    match client
        .transcribe_file(audio_bytes, request.use_dictionary)
        .await
    {
        Ok(text) => Ok(Json(TranscribeResponse {
            text,
            success: true,
            error: None,
        })),
        Err(e) => Ok(Json(TranscribeResponse {
            text: String::new(),
            success: false,
            error: Some(e.to_string()),
        })),
    }
}

// ============================================================================
// Dictionary Management
// ============================================================================

/// GET /api/stt/dictionaries — List all project dictionaries
pub async fn stt_list_dictionaries(
    State(state): State<OrchestratorState>,
) -> Result<Json<Vec<String>>, AppError> {
    let mgr = dictionary_manager(&state);
    let slugs = mgr
        .list()
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to list dictionaries")))?;
    Ok(Json(slugs))
}

/// GET /api/stt/dictionary/:slug — Get a project's dictionary
pub async fn stt_get_dictionary(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
) -> Result<Json<DictionaryResponse>, AppError> {
    let mgr = dictionary_manager(&state);
    let dict = mgr
        .get(&slug)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to get dictionary")))?;
    Ok(Json(DictionaryResponse::from(dict)))
}

/// PUT /api/stt/dictionary/:slug — Save a project's dictionary (full replace)
pub async fn stt_save_dictionary(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
    Json(request): Json<SaveDictionaryRequest>,
) -> Result<Json<DictionaryResponse>, AppError> {
    let mgr = dictionary_manager(&state);
    let dict = crate::stt::dictionary::ProjectDictionary {
        project_slug: slug,
        corrections: request.corrections.into_iter().collect(),
    };
    mgr.save(&dict)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to save dictionary")))?;
    Ok(Json(DictionaryResponse::from(dict)))
}

/// POST /api/stt/dictionary/:slug/rules — Add or update a correction rule
pub async fn stt_upsert_rule(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
    Json(request): Json<RuleRequest>,
) -> Result<Json<DictionaryResponse>, AppError> {
    let mgr = dictionary_manager(&state);
    let dict = mgr
        .upsert_rule(&slug, &request.pattern, &request.replacement)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to upsert rule")))?;
    Ok(Json(DictionaryResponse::from(dict)))
}

/// DELETE /api/stt/dictionary/:slug/rules — Remove a correction rule
pub async fn stt_remove_rule(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
    Json(request): Json<RemoveRuleRequest>,
) -> Result<Json<DictionaryResponse>, AppError> {
    let mgr = dictionary_manager(&state);
    let dict = mgr
        .remove_rule(&slug, &request.pattern)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to remove rule")))?;
    Ok(Json(DictionaryResponse::from(dict)))
}

/// POST /api/stt/dictionary/:slug/auto-generate — Auto-generate rules from code symbols
pub async fn stt_auto_generate_dictionary(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
) -> Result<Json<DictionaryResponse>, AppError> {
    // Resolve project UUID from slug
    let project = state
        .orchestrator
        .neo4j()
        .get_project_by_slug(&slug)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to look up project")))?
        .ok_or_else(|| AppError::NotFound(format!("Project not found: {slug}")))?;

    // Fetch code symbols — tuples: (id, name, type, file_path, visibility, line_start)
    let symbol_tuples = state
        .orchestrator
        .neo4j()
        .list_project_symbols(project.id, 5000)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to fetch project symbols")))?;

    // Extract just the names
    let symbol_names: Vec<String> = symbol_tuples
        .into_iter()
        .map(|(_, name, ..)| name)
        .collect();

    let mgr = dictionary_manager(&state);
    let dict = mgr
        .auto_generate(&slug, &symbol_names)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to auto-generate dictionary")))?;
    Ok(Json(DictionaryResponse::from(dict)))
}

/// DELETE /api/stt/dictionary/:slug — Delete an entire project dictionary
pub async fn stt_delete_dictionary(
    State(state): State<OrchestratorState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mgr = dictionary_manager(&state);
    mgr.delete(&slug)
        .await
        .map_err(|e| AppError::Internal(e.context("Failed to delete dictionary")))?;
    Ok(Json(
        serde_json::json!({"deleted": true, "project_slug": slug}),
    ))
}

// ============================================================================
// Dictionary DTOs
// ============================================================================

#[derive(Serialize)]
pub struct DictionaryResponse {
    pub project_slug: String,
    pub corrections: Vec<CorrectionEntry>,
    pub total_rules: usize,
}

#[derive(Serialize)]
pub struct CorrectionEntry {
    pub pattern: String,
    pub replacement: String,
}

impl From<crate::stt::dictionary::ProjectDictionary> for DictionaryResponse {
    fn from(dict: crate::stt::dictionary::ProjectDictionary) -> Self {
        let total = dict.corrections.len();
        Self {
            project_slug: dict.project_slug,
            corrections: dict
                .corrections
                .into_iter()
                .map(|(pattern, replacement)| CorrectionEntry {
                    pattern,
                    replacement,
                })
                .collect(),
            total_rules: total,
        }
    }
}

#[derive(Deserialize)]
pub struct SaveDictionaryRequest {
    /// Map of pattern → replacement
    pub corrections: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct RuleRequest {
    pub pattern: String,
    pub replacement: String,
}

#[derive(Deserialize)]
pub struct RemoveRuleRequest {
    pub pattern: String,
}

/// Helper: create a DictionaryManager from server state
fn dictionary_manager(_state: &OrchestratorState) -> crate::stt::DictionaryManager {
    // Use cc-rules dir relative to CWD, or configurable path
    let rules_dir =
        std::env::var("MURMURE_CC_RULES_PATH").unwrap_or_else(|_| "resources/cc-rules".to_string());
    crate::stt::DictionaryManager::new(rules_dir)
}
