//! STT API handlers — Speech-to-Text status and one-shot transcription
//!
//! These REST endpoints complement the WebSocket audio streaming.
//! - GET /api/stt/status — check if Murmure is available
//! - POST /api/stt/transcribe — one-shot transcription (for testing / non-streaming use)

use crate::api::handlers::{AppError, OrchestratorState};
use axum::{extract::State, Json};
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
