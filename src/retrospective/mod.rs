//! Task Retrospective System — learn from every agent execution.
//!
//! When a task completes (success or failure), the retrospective pipeline:
//! 1. Collects full tool call traces from ChatEventRecords
//! 2. Compares with historical cohort (same project/tags/files)
//! 3. Uses rs-stats for statistical anomaly detection
//! 4. Auto-generates Notes (gotcha/pattern/observation) from signals
//! 5. Persists a TaskRetrospective node linked to AgentExecution + Task
//! 6. Propagates feedback (energy boost / scar) on related notes

pub mod analyzer;
pub mod collector;
pub mod models;
