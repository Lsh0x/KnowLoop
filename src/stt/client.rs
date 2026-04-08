//! Murmure gRPC client wrapper
//!
//! Provides a high-level interface to the Murmure STT service.
//! Handles connection management, health checks, and graceful degradation.

use super::config::SttConfig;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::Channel;
use tracing::{debug, info};

// Generated from proto/murmure.proto
pub mod proto {
    tonic::include_proto!("murmure");
}

use proto::transcription_service_client::TranscriptionServiceClient;
use proto::{TranscribeFileRequest, TranscribeStreamRequest, TranscribeStreamResponse};

/// Murmure STT client with automatic reconnection and health checking
#[derive(Clone)]
pub struct MurmureClient {
    config: SttConfig,
    channel: Arc<RwLock<Option<Channel>>>,
}

impl MurmureClient {
    /// Create a new MurmureClient. Does NOT connect immediately — lazy connection.
    pub fn new(config: SttConfig) -> Self {
        Self {
            config,
            channel: Arc::new(RwLock::new(None)),
        }
    }

    /// Get or create a gRPC channel
    async fn get_channel(&self) -> Result<Channel> {
        // Check cached channel
        {
            let guard = self.channel.read().await;
            if let Some(ch) = guard.as_ref() {
                return Ok(ch.clone());
            }
        }

        // Create new channel
        let url = self
            .config
            .effective_url()
            .context("STT not configured (no grpc_url)")?;

        debug!(url = %url, "Connecting to Murmure gRPC server");

        let channel = Channel::from_shared(url.to_string())
            .context("Invalid Murmure gRPC URL")?
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .connect()
            .await
            .context("Failed to connect to Murmure gRPC server")?;

        // Cache it
        let mut guard = self.channel.write().await;
        *guard = Some(channel.clone());

        info!("Connected to Murmure STT server at {url}");
        Ok(channel)
    }

    /// Reset the cached channel (force reconnect on next call)
    async fn reset_channel(&self) {
        let mut guard = self.channel.write().await;
        *guard = None;
    }

    /// Check if Murmure is available and responding
    pub async fn is_available(&self) -> bool {
        if self.config.effective_url().is_none() {
            return false;
        }
        match self.get_channel().await {
            Ok(ch) => {
                // Try a lightweight operation to verify the service is responding
                let mut client = TranscriptionServiceClient::new(ch);
                let req = TranscribeFileRequest {
                    audio_data: vec![],
                    use_dictionary: false,
                };
                // An empty audio request will fail but the gRPC channel is alive
                match client.transcribe_file(req).await {
                    Ok(_) => true,
                    Err(status) => {
                        // Any gRPC response (even error) means the server is up
                        debug!(
                            code = ?status.code(),
                            "Murmure health check got error (server is up)"
                        );
                        true
                    }
                }
            }
            Err(e) => {
                debug!(error = %e, "Murmure not available");
                false
            }
        }
    }

    /// Transcribe a complete audio file (one-shot, non-streaming)
    pub async fn transcribe_file(
        &self,
        audio_data: Vec<u8>,
        use_dictionary: bool,
    ) -> Result<String> {
        let channel = self.get_channel().await?;
        let mut client = TranscriptionServiceClient::new(channel);

        let request = TranscribeFileRequest {
            audio_data,
            use_dictionary,
        };

        let response = client
            .transcribe_file(request)
            .await
            .context("Murmure transcribe_file RPC failed")?
            .into_inner();

        if response.success {
            Ok(response.text)
        } else {
            self.reset_channel().await;
            anyhow::bail!("Murmure transcription failed: {}", response.error)
        }
    }

    /// Open a bidirectional streaming transcription session.
    ///
    /// Returns a sender for audio chunks and a receiver for transcription results.
    pub async fn transcribe_stream(
        &self,
    ) -> Result<(
        tokio::sync::mpsc::Sender<TranscribeStreamRequest>,
        tonic::Streaming<TranscribeStreamResponse>,
    )> {
        let channel = self.get_channel().await?;
        let mut client = TranscriptionServiceClient::new(channel);

        let (tx, rx) = tokio::sync::mpsc::channel::<TranscribeStreamRequest>(32);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let response = client
            .transcribe_stream(stream)
            .await
            .context("Murmure transcribe_stream RPC failed")?;

        Ok((tx, response.into_inner()))
    }

    /// Helper: send an audio chunk to an active stream
    pub fn make_audio_chunk(data: Vec<u8>) -> TranscribeStreamRequest {
        TranscribeStreamRequest {
            request_type: Some(proto::transcribe_stream_request::RequestType::AudioChunk(
                data,
            )),
        }
    }

    /// Helper: send end-of-stream signal
    pub fn make_end_of_stream() -> TranscribeStreamRequest {
        TranscribeStreamRequest {
            request_type: Some(proto::transcribe_stream_request::RequestType::EndOfStream(
                true,
            )),
        }
    }
}
