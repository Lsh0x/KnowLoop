//! Tool trace collector — extracts tool call data from persisted ChatEventRecords.

use crate::neo4j::traits::GraphStore;
use crate::retrospective::models::{ToolCallEntry, ToolTrace};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// Collect a full tool trace from ChatEventRecords for a session.
///
/// Reads all persisted events, filters for ToolUse and ToolResult,
/// and builds a structured trace with per-tool breakdown and error counts.
pub async fn collect_tool_trace(graph: &dyn GraphStore, session_id: Uuid) -> Result<ToolTrace> {
    let events = graph.get_chat_events(session_id, 0, 50_000).await?;

    let mut trace = ToolTrace::default();
    let mut pending_tool_uses: HashMap<String, usize> = HashMap::new();

    for event in &events {
        let data: serde_json::Value = match serde_json::from_str(&event.data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match event.event_type.as_str() {
            "ToolUse" => {
                let tool_name = data
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let action = data
                    .get("input")
                    .and_then(|v| v.get("action"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let input_json = data.get("input").map(|v| v.to_string()).unwrap_or_default();
                let tool_use_id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let entry = ToolCallEntry {
                    seq: event.seq,
                    tool_name: tool_name.clone(),
                    action,
                    input_json,
                    output_json: None,
                    is_error: false,
                    timestamp: Utc::now(),
                };

                let idx = trace.tool_calls.len();
                trace.tool_calls.push(entry);
                trace.tool_call_count += 1;
                *trace.tool_call_breakdown.entry(tool_name).or_insert(0) += 1;

                if !tool_use_id.is_empty() {
                    pending_tool_uses.insert(tool_use_id, idx);
                }
            }
            "ToolResult" => {
                let tool_use_id = data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = data
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let result_str = data
                    .get("result")
                    .map(|v| {
                        // Truncate large outputs to 10KB
                        let s = v.to_string();
                        if s.len() > 10_240 {
                            format!("{}...[truncated]", &s[..10_240])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();

                if let Some(&idx) = pending_tool_uses.get(&tool_use_id) {
                    if let Some(entry) = trace.tool_calls.get_mut(idx) {
                        entry.output_json = Some(result_str);
                        entry.is_error = is_error;
                    }
                }

                if is_error {
                    trace.error_count += 1;
                    if let Some(err_msg) = data.get("result").and_then(|v| v.as_str()) {
                        trace.last_error = Some(err_msg.to_string());
                    } else {
                        trace.last_error = data.get("result").map(|v| v.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_trace() {
        let trace = ToolTrace::default();
        assert_eq!(trace.tool_call_count, 0);
        assert_eq!(trace.error_count, 0);
        assert!(trace.tool_calls.is_empty());
    }
}
