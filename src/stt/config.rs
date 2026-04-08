//! STT configuration

use serde::{Deserialize, Serialize};

/// Configuration for the Speech-to-Text (Murmure) integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// gRPC URL of the Murmure server (e.g. "http://localhost:50051")
    pub grpc_url: Option<String>,

    /// Whether STT is enabled (auto-detected from grpc_url availability)
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Maximum audio duration in seconds (safety limit)
    #[serde(default = "default_max_duration")]
    pub max_audio_duration_secs: u32,

    /// Whether to use Murmure's custom dictionary for corrections
    #[serde(default = "default_use_dictionary")]
    pub use_dictionary: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_max_duration() -> u32 {
    120 // 2 minutes max
}

fn default_use_dictionary() -> bool {
    true
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            grpc_url: None,
            enabled: true,
            max_audio_duration_secs: default_max_duration(),
            use_dictionary: true,
        }
    }
}

impl SttConfig {
    /// Returns the effective gRPC URL, or None if STT is disabled
    pub fn effective_url(&self) -> Option<&str> {
        if self.enabled {
            self.grpc_url.as_deref()
        } else {
            None
        }
    }
}
