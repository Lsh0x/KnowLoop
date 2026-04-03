//! GitDriftCheck — detects when local branches are behind their remote.
//!
//! Runs `git fetch` + `git log HEAD..origin/main` per watched project.
//! Creates an alert if there are new upstream commits.
//!
//! Projects are fetched concurrently with a per-project timeout of 10s.
//! The overall check timeout is 30s (via timeout_override).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, warn};

use crate::heartbeat::{HeartbeatCheck, HeartbeatContext};

/// Per-project timeout for `git fetch` + `git log` (10 seconds).
const PER_PROJECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Check for git drift on watched projects (every 10 minutes).
pub struct GitDriftCheck;

#[async_trait]
impl HeartbeatCheck for GitDriftCheck {
    fn name(&self) -> &str {
        "git_drift"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(10 * 60) // 10 minutes
    }

    fn timeout_override(&self) -> Option<Duration> {
        // 30s total — enough for concurrent fetch of multiple projects
        Some(Duration::from_secs(30))
    }

    async fn run(&self, ctx: &HeartbeatContext) -> Result<()> {
        let projects = ctx.graph.list_projects().await?;

        // Run all projects concurrently with per-project timeouts
        let graph = ctx.graph.clone();
        let emitter = ctx.emitter.clone();

        let handles: Vec<_> = projects
            .into_iter()
            .map(|project| {
                let graph = graph.clone();
                let emitter = emitter.clone();
                tokio::spawn(async move {
                    match tokio::time::timeout(
                        PER_PROJECT_TIMEOUT,
                        check_project_drift(&project, &graph, &emitter),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            warn!("GitDriftCheck: error checking '{}': {}", project.name, e);
                        }
                        Err(_) => {
                            warn!(
                                "GitDriftCheck: project '{}' timed out (>{:?}), skipping",
                                project.name, PER_PROJECT_TIMEOUT
                            );
                        }
                    }
                })
            })
            .collect();

        // Wait for all projects to complete (or timeout individually)
        for handle in handles {
            let _ = handle.await;
        }

        Ok(())
    }
}

/// Check a single project for git drift.
async fn check_project_drift(
    project: &crate::neo4j::models::ProjectNode,
    graph: &Arc<dyn crate::neo4j::traits::GraphStore>,
    emitter: &Option<Arc<dyn crate::events::EventEmitter>>,
) -> Result<()> {
    let root_path = &project.root_path;

    // git fetch (quiet)
    let fetch_output = tokio::process::Command::new("git")
        .args(["fetch", "--quiet"])
        .current_dir(root_path)
        .output()
        .await;

    if let Err(e) = &fetch_output {
        warn!(
            "GitDriftCheck: git fetch failed for '{}': {}",
            project.name, e
        );
        return Ok(());
    }

    // Check for new commits on origin/main
    let log_output = tokio::process::Command::new("git")
        .args(["log", "--oneline", "HEAD..origin/main"])
        .current_dir(root_path)
        .output()
        .await;

    match log_output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let commit_count = stdout.lines().count();

            if commit_count > 0 {
                debug!(
                    "GitDriftCheck: project '{}' is {} commit(s) behind origin/main",
                    project.name, commit_count
                );

                // Create alert
                let alert = crate::neo4j::models::AlertNode::new(
                    "git_drift".to_string(),
                    crate::neo4j::models::AlertSeverity::Warning,
                    format!(
                        "Project '{}' is {} commit(s) behind origin/main",
                        project.name, commit_count
                    ),
                    Some(project.id),
                );

                if let Err(e) = graph.create_alert(&alert).await {
                    warn!(
                        "GitDriftCheck: failed to create alert for '{}': {}",
                        project.name, e
                    );
                }

                // Emit event
                if let Some(ref emitter) = emitter {
                    emitter.emit_created(
                        crate::events::EntityType::Alert,
                        &alert.id.to_string(),
                        serde_json::json!({
                            "alert_type": "git_drift",
                            "project": project.name,
                            "commits_behind": commit_count,
                        }),
                        Some(project.id.to_string()),
                    );
                }
            } else {
                debug!(
                    "GitDriftCheck: project '{}' is up to date with origin/main",
                    project.name
                );
            }
        }
        Err(e) => {
            warn!(
                "GitDriftCheck: git log failed for '{}': {}",
                project.name, e
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_drift_check_name() {
        let check = GitDriftCheck;
        assert_eq!(check.name(), "git_drift");
    }

    #[test]
    fn test_git_drift_check_interval() {
        let check = GitDriftCheck;
        assert_eq!(check.interval(), Duration::from_secs(600));
    }

    #[test]
    fn test_git_drift_check_timeout_override() {
        let check = GitDriftCheck;
        assert_eq!(check.timeout_override(), Some(Duration::from_secs(30)));
    }
}
