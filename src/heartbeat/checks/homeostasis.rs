//! HomeostasisCheck — auto-corrective thermostat for the knowledge graph.
//!
//! Runs every 2 hours. For each project:
//! 1. Computes homeostasis report via `compute_homeostasis`
//! 2. Fetches neural metrics via `get_neural_metrics`
//! 3. **Dormancy guard**: if `pain_score < 0.1`, the system is stable — skip
//!    corrections to avoid eroding a dormant graph with no new input.
//! 4. Feeds metrics to `HomeostasisController::evaluate`
//! 5. Executes corrective actions via `execute_actions`
//!
//! Creates alerts when corrective actions are taken.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::heartbeat::{HeartbeatCheck, HeartbeatContext};
use crate::homeostasis::{execute_actions, HomeostasisController, HomeostasisMetrics};

/// Pain score below which we consider the system dormant/stable.
/// No corrective actions are taken — only observation.
const DORMANCY_THRESHOLD: f64 = 0.1;

/// Run homeostasis evaluation and correction on all projects (every 2 hours).
///
/// Includes a dormancy guard: if `pain_score < 0.1`, the project is considered
/// stable and no corrective actions are executed. This prevents erosion of a
/// quiet graph (e.g., synapse decay with no new input to compensate).
pub struct HomeostasisCheck;

#[async_trait]
impl HeartbeatCheck for HomeostasisCheck {
    fn name(&self) -> &str {
        "homeostasis"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(2 * 60 * 60) // 2 hours
    }

    async fn run(&self, ctx: &HeartbeatContext) -> Result<()> {
        let projects = ctx.graph.list_projects().await?;

        for project in &projects {
            debug!("HomeostasisCheck: evaluating project '{}'", project.name);

            // 1. Compute homeostasis report from the graph
            let report = match ctx.graph.compute_homeostasis(project.id, None).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        "HomeostasisCheck: compute_homeostasis failed for '{}': {}",
                        project.name, e
                    );
                    continue;
                }
            };

            // 2. Dormancy guard — stable systems are left alone.
            //    A dormant graph with pain < 0.1 should not receive corrective
            //    actions (decay, archive) that would erode it without new input.
            if report.pain_score < DORMANCY_THRESHOLD {
                debug!(
                    pain_score = report.pain_score,
                    threshold = DORMANCY_THRESHOLD,
                    "HomeostasisCheck: project '{}' is stable (pain < threshold), skipping corrections",
                    project.name
                );
                continue;
            }

            // 3. Fetch neural metrics for dead_notes info
            let neural = match ctx.graph.get_neural_metrics(project.id).await {
                Ok(n) => n,
                Err(e) => {
                    warn!(
                        "HomeostasisCheck: get_neural_metrics failed for '{}': {}",
                        project.name, e
                    );
                    continue;
                }
            };

            // 4. Map report ratios + neural metrics to HomeostasisMetrics
            //    Ratio names from compute_homeostasis: "synapse_health", "note_density",
            //    "decision_coverage", "churn_balance", "scar_load"
            let metrics = HomeostasisMetrics {
                synapse_health: report
                    .ratios
                    .iter()
                    .find(|r| r.name == "synapse_health")
                    .map(|r| r.value)
                    .unwrap_or(1.0),
                dead_notes_ratio: if neural.dead_notes_count > 0 {
                    // Approximate: dead_notes / (dead_notes + active_synapses as proxy for active notes)
                    // This is a heuristic; consolidate_memory will handle the actual cleanup.
                    let total =
                        (neural.dead_notes_count as f64) + (neural.active_synapses.max(1) as f64);
                    neural.dead_notes_count as f64 / total
                } else {
                    0.0
                },
                note_density: report
                    .ratios
                    .iter()
                    .find(|r| r.name == "note_density")
                    .map(|r| r.value)
                    .unwrap_or(1.0),
                pain_score: report.pain_score,
            };

            // 5. Evaluate corrective actions
            let actions = HomeostasisController::evaluate(&metrics);

            if actions.is_empty() {
                debug!(
                    "HomeostasisCheck: project '{}' has pain but no actionable corrections",
                    project.name
                );
                continue;
            }

            info!(
                "HomeostasisCheck: project '{}' (pain={:.3}) needs {} corrective action(s): {}",
                project.name,
                report.pain_score,
                actions.len(),
                actions
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            // 6. Execute corrective actions
            let graph_arc: Arc<dyn crate::neo4j::traits::GraphStore> = Arc::clone(&ctx.graph);
            match execute_actions(&graph_arc, &actions).await {
                Ok(executed) => {
                    info!(
                        "HomeostasisCheck: executed {}/{} actions for '{}'",
                        executed,
                        actions.len(),
                        project.name
                    );

                    // Create alert if actions were taken
                    if executed > 0 {
                        if let Some(ref emitter) = ctx.emitter {
                            emitter.emit_created(
                                crate::events::EntityType::Alert,
                                &project.id.to_string(),
                                serde_json::json!({
                                    "alert_type": "homeostasis_correction",
                                    "project": project.name,
                                    "pain_score": report.pain_score,
                                    "actions_executed": executed,
                                    "actions": actions.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                                }),
                                Some(project.id.to_string()),
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "HomeostasisCheck: execute_actions failed for '{}': {}",
                        project.name, e
                    );
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_homeostasis_check_name() {
        let check = HomeostasisCheck;
        assert_eq!(check.name(), "homeostasis");
    }

    #[test]
    fn test_homeostasis_check_interval_2h() {
        let check = HomeostasisCheck;
        assert_eq!(check.interval(), Duration::from_secs(2 * 3600));
    }

    #[test]
    fn test_dormancy_threshold_is_reasonable() {
        // Threshold should be low enough that only truly stable systems are skipped.
        // Use const block to satisfy clippy::assertions_on_constants.
        const {
            assert!(DORMANCY_THRESHOLD > 0.0);
            assert!(DORMANCY_THRESHOLD <= 0.15);
        }
    }
}
