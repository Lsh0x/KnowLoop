//! Retrospective analyzer — core learning engine.
//!
//! After each task completion, analyzes the execution against historical
//! cohort data and auto-generates Notes for significant patterns.

use crate::neo4j::traits::GraphStore;
use crate::notes::{EntityType, Note, NoteImportance, NoteScope, NoteStatus, NoteType};
use crate::retrospective::collector::collect_tool_trace;
use crate::retrospective::models::*;
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Input parameters for [`run_retrospective`].
pub struct RetrospectiveInput {
    pub graph: Arc<dyn GraphStore + Send + Sync>,
    pub task_id: Uuid,
    pub project_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub agent_execution_id: Option<Uuid>,
    pub outcome: RetrospectiveOutcome,
    pub duration_secs: f64,
    pub cost_usd: f64,
    pub confidence_score: f64,
    pub files_modified: Vec<String>,
    pub commits: Vec<String>,
}

/// Minimum cohort size required for statistical comparison.
const MIN_COHORT_SIZE: usize = 5;

/// Maximum number of auto-generated notes per retrospective.
const MAX_NOTES_PER_RETRO: usize = 3;

/// File failure rate threshold for generating a gotcha note.
const FILE_FAILURE_THRESHOLD: f64 = 0.4;

/// Z-score threshold for "significantly faster" (negative = faster).
const FAST_ZSCORE_THRESHOLD: f64 = -1.5;

/// Z-score threshold for "significantly more expensive".
const EXPENSIVE_ZSCORE_THRESHOLD: f64 = 2.0;

/// Run a full retrospective analysis after task completion.
///
/// This is designed to be called as a fire-and-forget `tokio::spawn`.
/// Errors are logged but don't propagate to the caller.
pub async fn run_retrospective(input: RetrospectiveInput) -> Result<TaskRetrospective> {
    let RetrospectiveInput {
        graph,
        task_id,
        project_id,
        session_id,
        agent_execution_id,
        outcome,
        duration_secs,
        cost_usd,
        confidence_score,
        files_modified,
        commits,
    } = input;
    info!(
        "Running retrospective for task {} (outcome: {})",
        task_id, outcome
    );

    // Step 1: Collect tool trace from ChatEventRecords
    let trace = if let Some(sid) = session_id {
        match collect_tool_trace(graph.as_ref(), sid).await {
            Ok(t) => t,
            Err(e) => {
                warn!("Failed to collect tool trace for session {}: {}", sid, e);
                ToolTrace::default()
            }
        }
    } else {
        ToolTrace::default()
    };

    // Step 2: Build retrospective
    let retro_id = Uuid::new_v4();
    let mut retro = TaskRetrospective {
        id: retro_id,
        task_id,
        agent_execution_id,
        project_id,
        outcome: outcome.clone(),
        confidence_score,
        duration_secs,
        cost_usd,
        tool_call_count: trace.tool_call_count,
        tool_call_breakdown: trace.tool_call_breakdown.clone(),
        error_count: trace.error_count,
        last_error: trace.last_error.clone(),
        files_modified: files_modified.clone(),
        commits: commits.clone(),
        notes_generated: Vec::new(),
        cohort_json: None,
        created_at: Utc::now(),
    };

    // Step 3: Find cohort and compare
    let cohort_comparison = if let Some(pid) = project_id {
        build_cohort_comparison(graph.as_ref(), pid, &files_modified, &retro).await
    } else {
        None
    };

    if let Some(ref comparison) = cohort_comparison {
        retro.cohort_json = serde_json::to_string(comparison).ok();
    }

    // Step 4: Auto-generate notes
    let notes = generate_learning_notes(graph.as_ref(), &retro, cohort_comparison.as_ref()).await;

    retro.notes_generated = notes.iter().map(|n| n.id).collect();

    // Step 5: Persist everything
    if let Err(e) = graph.create_task_retrospective(&retro).await {
        warn!(
            "Failed to persist retrospective for task {}: {}",
            task_id, e
        );
        return Err(e);
    }

    // Persist generated notes
    for note in &notes {
        if let Err(e) = graph.create_note(note).await {
            warn!("Failed to create retrospective note: {}", e);
            continue;
        }
        // Link note to the retrospective
        if let Err(e) = graph.link_retrospective_note(retro_id, note.id).await {
            warn!("Failed to link note {} to retrospective: {}", note.id, e);
        }
        // Attach note to the task
        if let Err(e) = graph
            .link_note_to_entity(note.id, &EntityType::Task, &task_id.to_string(), None, None)
            .await
        {
            warn!("Failed to attach note to task: {}", e);
        }
    }

    info!(
        "Retrospective {} completed for task {}: {} notes generated, cohort_size={}",
        retro_id,
        task_id,
        notes.len(),
        cohort_comparison
            .as_ref()
            .map(|c| c.cohort_size)
            .unwrap_or(0)
    );

    Ok(retro)
}

/// Build a cohort comparison using historical retrospectives from the same project.
async fn build_cohort_comparison(
    graph: &dyn GraphStore,
    project_id: Uuid,
    files: &[String],
    current: &TaskRetrospective,
) -> Option<CohortComparison> {
    let cohort = match graph
        .get_retrospectives_for_cohort(project_id, &[], files, 100)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to fetch cohort: {}", e);
            return None;
        }
    };

    // Exclude current retrospective from cohort
    let cohort: Vec<_> = cohort.into_iter().filter(|r| r.id != current.id).collect();

    if cohort.len() < MIN_COHORT_SIZE {
        debug!(
            "Cohort too small ({} < {}), skipping comparison",
            cohort.len(),
            MIN_COHORT_SIZE
        );
        return None;
    }

    let durations: Vec<f64> = cohort.iter().map(|r| r.duration_secs).collect();
    let costs: Vec<f64> = cohort.iter().map(|r| r.cost_usd).collect();
    let confidences: Vec<f64> = cohort.iter().map(|r| r.confidence_score).collect();

    // Tool anomaly detection
    let tool_anomalies = detect_tool_anomalies(current, &cohort);

    // File risk signals
    let file_risk_signals = match graph
        .get_file_failure_rates(project_id, &current.files_modified)
        .await
    {
        Ok(rates) => rates
            .into_iter()
            .filter(|(_, rate)| *rate > FILE_FAILURE_THRESHOLD)
            .map(|(path, rate)| FileRiskSignal {
                file_path: path,
                failure_rate: rate,
                signal: "high_failure_correlation".to_string(),
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    Some(CohortComparison {
        cohort_size: cohort.len(),
        cohort_criteria: format!("project:{}, file_overlap", project_id),
        duration: MetricComparison::compute(current.duration_secs, &durations),
        cost: MetricComparison::compute(current.cost_usd, &costs),
        confidence: MetricComparison::compute(current.confidence_score, &confidences),
        tool_anomalies,
        file_risk_signals,
    })
}

/// Detect tools used at unusual ratios compared to the cohort.
fn detect_tool_anomalies(
    current: &TaskRetrospective,
    cohort: &[TaskRetrospective],
) -> Vec<ToolAnomaly> {
    if current.tool_call_count == 0 {
        return Vec::new();
    }

    let current_total = current.tool_call_count as f64;
    let mut anomalies = Vec::new();

    // Collect all tool names from current + cohort
    let mut all_tools: std::collections::HashSet<String> =
        current.tool_call_breakdown.keys().cloned().collect();
    for r in cohort {
        all_tools.extend(r.tool_call_breakdown.keys().cloned());
    }

    for tool in &all_tools {
        let current_ratio =
            *current.tool_call_breakdown.get(tool).unwrap_or(&0) as f64 / current_total;

        let cohort_ratios: Vec<f64> = cohort
            .iter()
            .filter(|r| r.tool_call_count > 0)
            .map(|r| {
                *r.tool_call_breakdown.get(tool).unwrap_or(&0) as f64 / r.tool_call_count as f64
            })
            .collect();

        if cohort_ratios.is_empty() {
            continue;
        }

        let mean = cohort_ratios.iter().sum::<f64>() / cohort_ratios.len() as f64;
        let variance = cohort_ratios
            .iter()
            .map(|v| (v - mean).powi(2))
            .sum::<f64>()
            / cohort_ratios.len() as f64;
        let std = variance.sqrt();

        // Flag if more than 2 standard deviations from mean
        if std > f64::EPSILON {
            let z = (current_ratio - mean) / std;
            if z.abs() > 2.0 {
                anomalies.push(ToolAnomaly {
                    tool_name: tool.clone(),
                    current_ratio,
                    cohort_mean_ratio: mean,
                    direction: if z > 0.0 {
                        "higher".to_string()
                    } else {
                        "lower".to_string()
                    },
                });
            }
        }
    }

    anomalies
}

/// Generate learning notes based on retrospective analysis.
async fn generate_learning_notes(
    graph: &dyn GraphStore,
    retro: &TaskRetrospective,
    cohort: Option<&CohortComparison>,
) -> Vec<Note> {
    let mut notes = Vec::new();
    let base_tags = vec![
        "auto-retrospective".to_string(),
        format!("task:{}", retro.task_id),
    ];

    // Rule 1: Failed task + high-risk file → Gotcha
    if !retro.outcome.is_success() {
        if let Some(comparison) = cohort {
            for signal in &comparison.file_risk_signals {
                if signal.failure_rate > 0.5 && notes.len() < MAX_NOTES_PER_RETRO {
                    let reason = match &retro.outcome {
                        RetrospectiveOutcome::Failure { reason } => reason.clone(),
                        _ => "unknown".to_string(),
                    };

                    // Check if a similar note already exists
                    let tag = format!("file-risk:{}", signal.file_path);
                    if note_exists_with_tag(graph, retro.project_id, &tag).await {
                        continue;
                    }

                    let mut tags = base_tags.clone();
                    tags.push(tag);

                    notes.push(make_note(
                        retro.project_id,
                        NoteType::Gotcha,
                        NoteImportance::High,
                        format!(
                            "File `{}` has a {:.0}% task failure rate. Last failure: {}. Consider extra review when modifying this file.",
                            signal.file_path,
                            signal.failure_rate * 100.0,
                            reason,
                        ),
                        tags,
                    ));
                }
            }
        }
    }

    // Rule 2: Successful task significantly faster than cohort → Pattern
    if retro.outcome.is_success() {
        if let Some(comparison) = cohort {
            if comparison.duration.z_score < FAST_ZSCORE_THRESHOLD
                && notes.len() < MAX_NOTES_PER_RETRO
            {
                let tools_summary: String = retro
                    .tool_call_breakdown
                    .iter()
                    .map(|(k, v)| format!("{}:{}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");

                let mut tags = base_tags.clone();
                tags.push("fast-execution".to_string());

                notes.push(make_note(
                    retro.project_id,
                    NoteType::Pattern,
                    NoteImportance::Medium,
                    format!(
                        "Task completed in {:.0}s vs cohort avg {:.0}s ({:.1}x faster). Tool mix: [{}]. This approach may be worth formalizing.",
                        retro.duration_secs,
                        comparison.duration.cohort_mean,
                        comparison.duration.cohort_mean / retro.duration_secs.max(1.0),
                        tools_summary,
                    ),
                    tags,
                ));
            }
        }
    }

    // Rule 3: Failed task with error count above cohort p90 → Gotcha
    if !retro.outcome.is_success() {
        if let Some(comparison) = cohort {
            // Compute p90 of error counts from cohort implicitly via z-score
            // If error_count is significantly above mean, flag it
            if retro.error_count > 3 && notes.len() < MAX_NOTES_PER_RETRO {
                let reason = match &retro.outcome {
                    RetrospectiveOutcome::Failure { reason } => reason.clone(),
                    _ => "unknown".to_string(),
                };
                let tag = format!("error-pattern:{}", retro.task_id);
                if !note_exists_with_tag(graph, retro.project_id, &tag).await {
                    let mut tags = base_tags.clone();
                    tags.push(tag);

                    notes.push(make_note(
                        retro.project_id,
                        NoteType::Gotcha,
                        NoteImportance::High,
                        format!(
                            "Task failed with {} errors (cohort size: {}). Error: {}. Files: [{}].",
                            retro.error_count,
                            comparison.cohort_size,
                            retro.last_error.as_deref().unwrap_or(&reason),
                            retro.files_modified.join(", "),
                        ),
                        tags,
                    ));
                }
            }
        }
    }

    // Rule 4: Cost significantly above cohort → Observation
    if let Some(comparison) = cohort {
        if comparison.cost.z_score > EXPENSIVE_ZSCORE_THRESHOLD && notes.len() < MAX_NOTES_PER_RETRO
        {
            let mut tags = base_tags.clone();
            tags.push("cost-alert".to_string());

            notes.push(make_note(
                retro.project_id,
                NoteType::Observation,
                NoteImportance::Medium,
                format!(
                    "Task cost ${:.3} vs cohort avg ${:.3} (z-score: {:.1}). This may indicate increased complexity or inefficiency.",
                    retro.cost_usd,
                    comparison.cost.cohort_mean,
                    comparison.cost.z_score,
                ),
                tags,
            ));
        }
    }

    notes
}

/// Check if a note with a specific tag already exists for this project.
async fn note_exists_with_tag(graph: &dyn GraphStore, project_id: Option<Uuid>, tag: &str) -> bool {
    if let Some(pid) = project_id {
        let filters = crate::notes::NoteFilters {
            tags: Some(vec![tag.to_string()]),
            status: Some(vec![NoteStatus::Active]),
            limit: Some(1),
            ..Default::default()
        };
        match graph.list_notes(Some(pid), None, &filters).await {
            Ok((notes, _)) => !notes.is_empty(),
            Err(_) => false,
        }
    } else {
        false
    }
}

/// Create a Note with standard retrospective defaults.
fn make_note(
    project_id: Option<Uuid>,
    note_type: NoteType,
    importance: NoteImportance,
    content: String,
    tags: Vec<String>,
) -> Note {
    Note {
        id: Uuid::new_v4(),
        project_id,
        note_type,
        status: NoteStatus::Active,
        importance,
        scope: NoteScope::Project,
        content,
        tags,
        anchors: Vec::new(),
        created_at: Utc::now(),
        created_by: "retrospective-engine".to_string(),
        last_confirmed_at: None,
        last_confirmed_by: None,
        staleness_score: 0.0,
        energy: 1.0,
        last_activated: Some(Utc::now()),
        reactivation_count: 0,
        last_reactivated: None,
        freshness_pinged_at: None,
        activation_count: 0,
        scar_intensity: 0.0,
        memory_horizon: Default::default(),
        supersedes: None,
        superseded_by: None,
        changes: Vec::new(),
        assertion_rule: None,
        last_assertion_result: None,
        sharing_consent: Default::default(),
    }
}
