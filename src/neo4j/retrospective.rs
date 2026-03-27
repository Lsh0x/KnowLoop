//! Neo4j TaskRetrospective operations — per-task learning records.
//!
//! Each completed task gets a TaskRetrospective node linked to its
//! AgentExecution and Task, enabling cohort comparison and learning.

use super::client::Neo4jClient;
use crate::retrospective::models::{RetrospectiveOutcome, TaskRetrospective};
use anyhow::Result;
use neo4rs::query;
use std::collections::HashMap;
use uuid::Uuid;

impl Neo4jClient {
    // ========================================================================
    // TaskRetrospective CRUD
    // ========================================================================

    /// Create a TaskRetrospective node and link it to Task (and optionally AgentExecution).
    pub async fn create_task_retrospective_impl(&self, retro: &TaskRetrospective) -> Result<()> {
        let tool_breakdown_json = serde_json::to_string(&retro.tool_call_breakdown)?;
        let notes_generated: Vec<String> = retro.notes_generated.iter().map(|u| u.to_string()).collect();
        let outcome_json = serde_json::to_string(&retro.outcome)?;

        let q = query(
            r#"
            MATCH (t:Task {id: $task_id})
            CREATE (r:TaskRetrospective {
                id: $id,
                task_id: $task_id,
                agent_execution_id: $ae_id,
                project_id: $project_id,
                outcome: $outcome,
                outcome_json: $outcome_json,
                confidence_score: $confidence_score,
                duration_secs: $duration_secs,
                cost_usd: $cost_usd,
                tool_call_count: $tool_call_count,
                tool_call_breakdown: $tool_breakdown,
                error_count: $error_count,
                last_error: $last_error,
                files_modified: $files_modified,
                commits: $commits,
                notes_generated: $notes_generated,
                cohort_json: $cohort_json,
                created_at: datetime($created_at)
            })
            CREATE (r)-[:FOR_TASK]->(t)
            WITH r
            OPTIONAL MATCH (ae:AgentExecution {id: $ae_id})
            WHERE $ae_id <> ''
            FOREACH (_ IN CASE WHEN ae IS NOT NULL THEN [1] ELSE [] END |
                CREATE (r)-[:FOR_EXECUTION]->(ae)
            )
            "#,
        )
        .param("id", retro.id.to_string())
        .param("task_id", retro.task_id.to_string())
        .param(
            "ae_id",
            retro
                .agent_execution_id
                .map(|u| u.to_string())
                .unwrap_or_default(),
        )
        .param(
            "project_id",
            retro
                .project_id
                .map(|u| u.to_string())
                .unwrap_or_default(),
        )
        .param("outcome", retro.outcome.as_str())
        .param("outcome_json", outcome_json)
        .param("confidence_score", retro.confidence_score)
        .param("duration_secs", retro.duration_secs)
        .param("cost_usd", retro.cost_usd)
        .param("tool_call_count", retro.tool_call_count as i64)
        .param("tool_breakdown", tool_breakdown_json)
        .param("error_count", retro.error_count as i64)
        .param(
            "last_error",
            retro.last_error.clone().unwrap_or_default(),
        )
        .param("files_modified", retro.files_modified.clone())
        .param("commits", retro.commits.clone())
        .param("notes_generated", notes_generated)
        .param(
            "cohort_json",
            retro.cohort_json.clone().unwrap_or_default(),
        )
        .param("created_at", retro.created_at.to_rfc3339());

        self.graph.run(q).await?;
        Ok(())
    }

    /// Get a TaskRetrospective by ID.
    pub async fn get_task_retrospective_impl(
        &self,
        id: Uuid,
    ) -> Result<Option<TaskRetrospective>> {
        let q = query(
            r#"
            MATCH (r:TaskRetrospective {id: $id})
            RETURN r
            "#,
        )
        .param("id", id.to_string());

        let mut result = self.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("r")?;
            Ok(Some(self.node_to_retrospective(&node)?))
        } else {
            Ok(None)
        }
    }

    /// Get the retrospective for a specific task.
    pub async fn get_retrospective_for_task_impl(
        &self,
        task_id: Uuid,
    ) -> Result<Option<TaskRetrospective>> {
        let q = query(
            r#"
            MATCH (r:TaskRetrospective {task_id: $task_id})
            RETURN r
            ORDER BY r.created_at DESC
            LIMIT 1
            "#,
        )
        .param("task_id", task_id.to_string());

        let mut result = self.graph.execute(q).await?;
        if let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("r")?;
            Ok(Some(self.node_to_retrospective(&node)?))
        } else {
            Ok(None)
        }
    }

    /// List retrospectives with optional filters.
    pub async fn list_retrospectives_impl(
        &self,
        project_id: Option<Uuid>,
        outcome: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskRetrospective>> {
        let mut cypher = String::from("MATCH (r:TaskRetrospective) WHERE true ");

        if project_id.is_some() {
            cypher.push_str("AND r.project_id = $project_id ");
        }
        if outcome.is_some() {
            cypher.push_str("AND r.outcome = $outcome ");
        }

        cypher.push_str("RETURN r ORDER BY r.created_at DESC SKIP $offset LIMIT $limit");

        let mut q = query(&cypher)
            .param("limit", limit)
            .param("offset", offset);

        if let Some(pid) = project_id {
            q = q.param("project_id", pid.to_string());
        }
        if let Some(out) = outcome {
            q = q.param("outcome", out);
        }

        let mut result = self.graph.execute(q).await?;
        let mut retros = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("r")?;
            retros.push(self.node_to_retrospective(&node)?);
        }
        Ok(retros)
    }

    /// Get retrospectives for a cohort: same project, overlapping tags or files.
    pub async fn get_retrospectives_for_cohort_impl(
        &self,
        project_id: Uuid,
        tags: &[String],
        files: &[String],
        limit: i64,
    ) -> Result<Vec<TaskRetrospective>> {
        // Find retrospectives from the same project.
        // If tags/files provided, prefer those that overlap, but still return
        // project-level cohort as fallback.
        let q = query(
            r#"
            MATCH (r:TaskRetrospective {project_id: $project_id})
            WITH r,
                 CASE WHEN size($tags) > 0 THEN
                    size([f IN r.files_modified WHERE f IN $files])
                 ELSE 0 END AS file_overlap
            RETURN r
            ORDER BY file_overlap DESC, r.created_at DESC
            LIMIT $limit
            "#,
        )
        .param("project_id", project_id.to_string())
        .param("tags", tags.to_vec())
        .param("files", files.to_vec())
        .param("limit", limit);

        let mut result = self.graph.execute(q).await?;
        let mut retros = Vec::new();
        while let Some(row) = result.next().await? {
            let node: neo4rs::Node = row.get("r")?;
            retros.push(self.node_to_retrospective(&node)?);
        }
        Ok(retros)
    }

    /// Get failure rates for specific files based on historical retrospectives.
    /// Returns a map of file_path -> failure_rate (0.0–1.0).
    pub async fn get_file_failure_rates_impl(
        &self,
        project_id: Uuid,
        file_paths: &[String],
    ) -> Result<HashMap<String, f64>> {
        if file_paths.is_empty() {
            return Ok(HashMap::new());
        }

        let q = query(
            r#"
            UNWIND $files AS file_path
            OPTIONAL MATCH (r:TaskRetrospective {project_id: $project_id})
            WHERE file_path IN r.files_modified
            WITH file_path,
                 count(r) AS total,
                 sum(CASE WHEN r.outcome = 'failure' THEN 1 ELSE 0 END) AS failures
            WHERE total > 0
            RETURN file_path, toFloat(failures) / toFloat(total) AS failure_rate
            "#,
        )
        .param("project_id", project_id.to_string())
        .param("files", file_paths.to_vec());

        let mut result = self.graph.execute(q).await?;
        let mut rates = HashMap::new();
        while let Some(row) = result.next().await? {
            let path: String = row.get("file_path")?;
            let rate: f64 = row.get("failure_rate")?;
            rates.insert(path, rate);
        }
        Ok(rates)
    }

    /// Link a generated note to a retrospective.
    pub async fn link_retrospective_note_impl(
        &self,
        retrospective_id: Uuid,
        note_id: Uuid,
    ) -> Result<()> {
        let q = query(
            r#"
            MATCH (r:TaskRetrospective {id: $retro_id})
            MATCH (n:Note {id: $note_id})
            MERGE (r)-[:GENERATED_NOTE]->(n)
            "#,
        )
        .param("retro_id", retrospective_id.to_string())
        .param("note_id", note_id.to_string());

        self.graph.run(q).await?;
        Ok(())
    }

    /// Convert a Neo4j node to TaskRetrospective.
    fn node_to_retrospective(&self, node: &neo4rs::Node) -> Result<TaskRetrospective> {
        let id: String = node.get("id")?;
        let task_id: String = node.get("task_id")?;
        let ae_id: Option<String> = node.get("agent_execution_id").ok();
        let project_id: Option<String> = node.get("project_id").ok();
        let outcome_str: String = node.get("outcome")?;
        let outcome_json: Option<String> = node.get("outcome_json").ok();
        let created_at: String = node.get("created_at")?;
        let notes_strs: Vec<String> = node.get("notes_generated").unwrap_or_default();
        let tool_breakdown_str: String = node.get("tool_call_breakdown").unwrap_or_default();

        let outcome = if let Some(ref json) = outcome_json {
            serde_json::from_str(json).unwrap_or_else(|_| {
                RetrospectiveOutcome::from_neo4j(&outcome_str, None)
            })
        } else {
            RetrospectiveOutcome::from_neo4j(&outcome_str, None)
        };

        let tool_call_breakdown: HashMap<String, u32> =
            serde_json::from_str(&tool_breakdown_str).unwrap_or_default();

        let notes_generated: Vec<Uuid> = notes_strs
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        Ok(TaskRetrospective {
            id: id.parse()?,
            task_id: task_id.parse()?,
            agent_execution_id: ae_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            project_id: project_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            outcome,
            confidence_score: node.get("confidence_score").unwrap_or(0.0),
            duration_secs: node.get("duration_secs").unwrap_or(0.0),
            cost_usd: node.get("cost_usd").unwrap_or(0.0),
            tool_call_count: node.get::<i64>("tool_call_count").unwrap_or(0) as u32,
            tool_call_breakdown,
            error_count: node.get::<i64>("error_count").unwrap_or(0) as u32,
            last_error: node.get("last_error").ok().filter(|s: &String| !s.is_empty()),
            files_modified: node.get("files_modified").unwrap_or_default(),
            commits: node.get("commits").unwrap_or_default(),
            notes_generated,
            cohort_json: node.get("cohort_json").ok().filter(|s: &String| !s.is_empty()),
            created_at: created_at.parse()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrospective::models::RetrospectiveOutcome;
    use chrono::Utc;

    fn make_retrospective() -> TaskRetrospective {
        TaskRetrospective {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            agent_execution_id: Some(Uuid::new_v4()),
            project_id: Some(Uuid::new_v4()),
            outcome: RetrospectiveOutcome::Success,
            confidence_score: 0.85,
            duration_secs: 120.5,
            cost_usd: 0.12,
            tool_call_count: 15,
            tool_call_breakdown: HashMap::from([
                ("Edit".to_string(), 5),
                ("Bash".to_string(), 3),
                ("Read".to_string(), 7),
            ]),
            error_count: 1,
            last_error: Some("compile error".to_string()),
            files_modified: vec!["src/main.rs".to_string()],
            commits: vec!["abc123".to_string()],
            notes_generated: vec![],
            cohort_json: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_retrospective_serialize_roundtrip() {
        let retro = make_retrospective();
        let json = serde_json::to_string(&retro).unwrap();
        let deserialized: TaskRetrospective = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, retro.id);
        assert_eq!(deserialized.confidence_score, retro.confidence_score);
        assert_eq!(deserialized.tool_call_count, retro.tool_call_count);
    }

    #[test]
    fn test_retrospective_failure_outcome() {
        let mut retro = make_retrospective();
        retro.outcome = RetrospectiveOutcome::Failure {
            reason: "tests failed".to_string(),
        };
        let json = serde_json::to_string(&retro.outcome).unwrap();
        let deserialized: RetrospectiveOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(retro.outcome, deserialized);
    }
}
