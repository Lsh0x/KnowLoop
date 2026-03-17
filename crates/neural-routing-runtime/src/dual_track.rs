//! DualTrack Router — Policy Net + NN Router fallback.
//!
//! In "nn" mode: only uses the Nearest Neighbor Router.
//! In "full" mode: tries the Policy Net first, falls back to NN Router
//! if the policy net times out, returns OOD, or the CpuGuard is paused.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use neural_routing_core::{NNRoute, Router, TrajectoryStore, error::Result};
use neural_routing_nn::NNRouter;

use crate::config::{NeuralRoutingConfig, RoutingMode};
use crate::cpu_guard::CpuGuard;

/// DualTrack Router — orchestrates policy net and NN router with timeout + fallback.
pub struct DualTrackRouter {
    nn_router: NNRouter,
    config: NeuralRoutingConfig,
    cpu_guard: CpuGuard,
}

impl DualTrackRouter {
    pub fn new(
        store: Arc<dyn TrajectoryStore>,
        config: NeuralRoutingConfig,
    ) -> Self {
        let cpu_guard = CpuGuard::new(config.cpu_guard.clone().into());
        let nn_router = NNRouter::new(store, config.nn.clone());

        Self {
            nn_router,
            config,
            cpu_guard,
        }
    }

    /// Start the CPU guard background monitoring.
    pub fn start_cpu_monitoring(&self) -> tokio::task::JoinHandle<()> {
        self.cpu_guard.start_monitoring()
    }

    /// Get the NN router for direct access (metrics, cache invalidation).
    pub fn nn_router(&self) -> &NNRouter {
        &self.nn_router
    }

    /// Get the CPU guard state.
    pub fn cpu_guard(&self) -> &CpuGuard {
        &self.cpu_guard
    }

    /// Get the current config.
    pub fn config(&self) -> &NeuralRoutingConfig {
        &self.config
    }

    /// Update config at runtime (hot reload).
    pub fn update_config(&mut self, config: NeuralRoutingConfig) {
        self.config = config;
    }
}

#[async_trait]
impl Router for DualTrackRouter {
    async fn route(&self, query_embedding: &[f32]) -> Result<Option<NNRoute>> {
        if !self.config.enabled {
            return Ok(None);
        }

        match self.config.mode {
            RoutingMode::NN => {
                // NN-only mode — direct to nearest neighbor router
                self.nn_router.route(query_embedding).await
            }
            RoutingMode::Full => {
                // Full mode — try policy net with timeout, fallback to NN
                // Phase 3+ will add policy net here. For now, fallback to NN.
                //
                // Future implementation:
                // 1. Check CpuGuard — if paused, skip to NN
                // 2. Try policy net with timeout (inference.timeout_ms)
                // 3. If timeout/OOD/error → fallback to NN
                //
                // For now: always NN (policy net not yet implemented)
                if self.cpu_guard.is_paused() {
                    tracing::debug!("DualTrack: CPU guard paused, using NN Router");
                }

                let timeout = Duration::from_millis(self.config.inference.timeout_ms);
                match tokio::time::timeout(timeout, self.nn_router.route(query_embedding)).await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::warn!("DualTrack: NN Router timed out after {}ms", self.config.inference.timeout_ms);
                        Ok(None)
                    }
                }
            }
        }
    }

    async fn route_with_context(
        &self,
        query_embedding: &[f32],
        available_tools: &[String],
    ) -> Result<Option<NNRoute>> {
        if !self.config.enabled {
            return Ok(None);
        }

        match self.config.mode {
            RoutingMode::NN => {
                self.nn_router.route_with_context(query_embedding, available_tools).await
            }
            RoutingMode::Full => {
                // Same as above — policy net will be added in Phase 3
                let timeout = Duration::from_millis(self.config.inference.timeout_ms);
                match tokio::time::timeout(
                    timeout,
                    self.nn_router.route_with_context(query_embedding, available_tools),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::warn!("DualTrack: timed out, returning None");
                        Ok(None)
                    }
                }
            }
        }
    }
}
