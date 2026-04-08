//! Speech-to-Text integration via Murmure gRPC sidecar
//!
//! Murmure is a privacy-first, on-device STT engine using NVIDIA's Parakeet model.
//! Communication happens over gRPC with bidirectional streaming support.
//!
//! Architecture:
//! - Murmure runs as a sidecar service (Docker or standalone)
//! - KnowLoop connects via gRPC on configurable URL (default: localhost:50051)
//! - Audio is streamed from the browser → WebSocket → KnowLoop → gRPC → Murmure
//! - Transcription results stream back: Murmure → gRPC → KnowLoop → WebSocket → browser
//!
//! If Murmure is unavailable, STT features are disabled gracefully (no crash).

pub mod client;
pub mod config;

pub use client::MurmureClient;
pub use config::SttConfig;
