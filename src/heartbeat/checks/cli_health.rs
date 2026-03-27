//! CliHealthCheck — proactive watchdog for CLI subprocess liveness.
//!
//! Periodically checks all active chat sessions to detect dead CLI subprocesses.
//! When the CLI process dies, its stdin receiver is dropped, making `stdin_tx.is_closed()`
//! return true. This check detects that condition and:
//! 1. Emits a ChatEvent::Error to notify connected WebSocket clients
//! 2. Removes the session from active_sessions so the next message triggers a fresh CLI
//!
//! Without this check, dead CLIs are only detected reactively when the user sends
//! a message and gets a "Broken pipe" error — which can be minutes later.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{info, warn};

use std::sync::Arc;

use crate::chat::manager::ActiveSession;
use crate::chat::types::ChatEvent;
use crate::heartbeat::{HeartbeatCheck, HeartbeatContext};

/// Proactive health check for CLI subprocesses.
///
/// Holds a reference to the shared `active_sessions` map from `ChatManager`.
/// On each tick, iterates all sessions and checks if the stdin channel is closed
/// (indicating the CLI process has exited).
pub struct CliHealthCheck {
    active_sessions: Arc<RwLock<HashMap<String, ActiveSession>>>,
}

impl CliHealthCheck {
    pub fn new(active_sessions: Arc<RwLock<HashMap<String, ActiveSession>>>) -> Self {
        Self { active_sessions }
    }
}

#[async_trait]
impl HeartbeatCheck for CliHealthCheck {
    fn name(&self) -> &str {
        "cli_health"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn run(&self, _ctx: &HeartbeatContext) -> Result<()> {
        // Collect dead session IDs under a read lock first
        let dead_sessions: Vec<String> = {
            let sessions = self.active_sessions.read().await;
            sessions
                .iter()
                .filter_map(|(session_id, session)| {
                    // Check if the stdin channel is closed (CLI process exited)
                    let is_dead = match &session.stdin_tx {
                        Some(tx) => tx.is_closed(),
                        // No stdin_tx means session was never fully connected,
                        // or it's a session being set up — skip
                        None => false,
                    };

                    if is_dead {
                        Some(session_id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        if dead_sessions.is_empty() {
            return Ok(());
        }

        // Now take a write lock to remove dead sessions and notify clients
        let mut sessions = self.active_sessions.write().await;
        for session_id in &dead_sessions {
            if let Some(session) = sessions.get(session_id) {
                // Only act if the session is still marked as streaming —
                // if not streaming, the CLI may have exited normally after completing
                let was_streaming = session.is_streaming.load(Ordering::Relaxed);

                // Notify connected WebSocket clients
                let _ = session.events_tx.send(ChatEvent::Error {
                    message: format!(
                        "CLI subprocess exited unexpectedly{}. Send a new message to reconnect.",
                        if was_streaming {
                            " while streaming"
                        } else {
                            ""
                        }
                    ),
                    parent_tool_use_id: None,
                });

                // Also emit StreamingStatus false so the UI stops showing the spinner
                if was_streaming {
                    let _ = session
                        .events_tx
                        .send(ChatEvent::StreamingStatus { is_streaming: false });
                }

                warn!(
                    "CliHealthCheck: dead CLI detected for session {} (was_streaming={}), removing from active_sessions",
                    session_id, was_streaming
                );
            }

            sessions.remove(session_id);
        }

        info!(
            "CliHealthCheck: removed {} dead session(s)",
            dead_sessions.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_health_check_name() {
        let sessions = Arc::new(RwLock::new(HashMap::new()));
        let check = CliHealthCheck::new(sessions);
        assert_eq!(check.name(), "cli_health");
    }

    #[test]
    fn test_cli_health_check_interval() {
        let sessions = Arc::new(RwLock::new(HashMap::new()));
        let check = CliHealthCheck::new(sessions);
        assert_eq!(check.interval(), Duration::from_secs(30));
    }
}
