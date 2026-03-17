//! TrajectoryCollector — fire-and-forget decision capture with mpsc channel.
//!
//! Architecture:
//! - Hot path: `record_decision()` sends a `DecisionEvent` via bounded mpsc channel (~0 latency)
//! - Background task: receives events, buffers them, flushes in batches to Neo4j
//! - Session-scoped: each session gets its own trajectory, finalized on `end_session()`

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use neural_routing_core::{
    Neo4jTrajectoryStore, ToolUsage, TouchedEntity, Trajectory, TrajectoryNode, TrajectoryStore,
};

use crate::config::CollectionConfig;

// ---------------------------------------------------------------------------
// Events sent through the mpsc channel
// ---------------------------------------------------------------------------

/// A decision event sent from the hot path to the background collector.
#[derive(Debug, Clone)]
pub enum CollectorEvent {
    /// Record a single decision point.
    Decision(DecisionRecord),
    /// End a session — triggers trajectory finalization and flush.
    EndSession {
        session_id: String,
        total_reward: f64,
    },
    /// Graceful shutdown — flush all pending data.
    Shutdown,
}

/// A single decision point captured from the hot path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    /// Session this decision belongs to.
    pub session_id: String,
    /// Context embedding (256d, L2-normalized). Empty if not available yet.
    pub context_embedding: Vec<f32>,
    /// MCP tool + action invoked (e.g., "code.search", "note.get_context").
    pub action_type: String,
    /// Serialized key parameters (stripped of PII).
    pub action_params: serde_json::Value,
    /// Number of alternative actions considered.
    pub alternatives_count: usize,
    /// Index of the chosen action among alternatives (0 if no alternatives).
    pub chosen_index: usize,
    /// Model confidence in this decision (0.0 - 1.0).
    pub confidence: f64,
    /// Tools used at this decision point.
    pub tool_usages: Vec<ToolUsage>,
    /// Entities touched at this decision point.
    pub touched_entities: Vec<TouchedEntity>,
    /// Timestamp in ms since session start.
    pub timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// TrajectoryCollector — the public API
// ---------------------------------------------------------------------------

/// Fire-and-forget trajectory collector.
///
/// The `record_decision()` method sends events via a bounded mpsc channel.
/// A background task receives and batches them for Neo4j persistence.
///
/// Latency budget: <1ms on the hot path (just a channel send).
pub struct TrajectoryCollector {
    /// Sender half of the bounded mpsc channel.
    tx: mpsc::Sender<CollectorEvent>,
    /// Whether collection is enabled (runtime toggle).
    enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl TrajectoryCollector {
    /// Create a new collector and spawn the background flush task.
    ///
    /// Returns the collector handle and a JoinHandle for the background task.
    pub fn new(
        store: Arc<Neo4jTrajectoryStore>,
        config: &CollectionConfig,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let buffer_size = config.buffer_size;
        // Channel capacity = 2x buffer to absorb bursts without blocking
        let (tx, rx) = mpsc::channel(buffer_size * 2);

        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(config.enabled));
        let enabled_clone = enabled.clone();

        let handle = tokio::spawn(async move {
            run_collector_loop(rx, store, buffer_size, enabled_clone).await;
        });

        let collector = Self { tx, enabled };
        (collector, handle)
    }

    /// Record a decision point (fire-and-forget).
    ///
    /// Returns immediately. If the channel is full, the event is dropped
    /// (we never block the hot path).
    pub fn record_decision(&self, record: DecisionRecord) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        // try_send: non-blocking, drops if channel full
        let _ = self.tx.try_send(CollectorEvent::Decision(record));
    }

    /// End a session — triggers finalization and flush of the trajectory.
    pub fn end_session(&self, session_id: String, total_reward: f64) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        if let Err(e) = self.tx.try_send(CollectorEvent::EndSession {
            session_id: session_id.clone(),
            total_reward,
        }) {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to send EndSession event — trajectory will be lost"
            );
        }
    }

    /// Request graceful shutdown — flushes all pending data.
    pub async fn shutdown(&self) {
        let _ = self.tx.send(CollectorEvent::Shutdown).await;
    }

    /// Toggle collection on/off at runtime.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if collection is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Background collector loop
// ---------------------------------------------------------------------------

/// In-flight session state (buffered decisions before finalization).
struct SessionBuffer {
    decisions: Vec<DecisionRecord>,
    started_at: chrono::DateTime<Utc>,
}

async fn run_collector_loop(
    mut rx: mpsc::Receiver<CollectorEvent>,
    store: Arc<Neo4jTrajectoryStore>,
    buffer_size: usize,
    _enabled: Arc<std::sync::atomic::AtomicBool>,
) {
    let sessions: Arc<Mutex<HashMap<String, SessionBuffer>>> = Arc::new(Mutex::new(HashMap::new()));

    // Batch of finalized trajectories waiting to be flushed
    let pending_flush: Arc<Mutex<Vec<PendingTrajectory>>> = Arc::new(Mutex::new(Vec::new()));

    while let Some(event) = rx.recv().await {
        match event {
            CollectorEvent::Decision(record) => {
                let mut sessions = sessions.lock().await;
                let session = sessions
                    .entry(record.session_id.clone())
                    .or_insert_with(|| SessionBuffer {
                        decisions: Vec::new(),
                        started_at: Utc::now(),
                    });
                session.decisions.push(record);
            }

            CollectorEvent::EndSession {
                session_id,
                total_reward,
            } => {
                let mut sessions = sessions.lock().await;
                if let Some(buffer) = sessions.remove(&session_id) {
                    let trajectory = build_trajectory(&session_id, &buffer, total_reward);
                    let mut pending = pending_flush.lock().await;
                    pending.push(trajectory);

                    // Flush if we've accumulated enough
                    if pending.len() >= buffer_size {
                        let to_flush: Vec<_> = pending.drain(..).collect();
                        drop(pending);
                        flush_batch(&store, to_flush).await;
                    }
                }
            }

            CollectorEvent::Shutdown => {
                // Finalize any open sessions with reward 0.0 (incomplete)
                let mut sessions = sessions.lock().await;
                let open_sessions: Vec<_> = sessions.drain().collect();
                drop(sessions);

                let mut pending = pending_flush.lock().await;
                for (session_id, buffer) in open_sessions {
                    let trajectory = build_trajectory(&session_id, &buffer, 0.0);
                    pending.push(trajectory);
                }

                // Final flush
                let to_flush: Vec<_> = pending.drain(..).collect();
                drop(pending);
                if !to_flush.is_empty() {
                    flush_batch(&store, to_flush).await;
                }

                tracing::info!("TrajectoryCollector shutdown complete");
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Trajectory building and flushing
// ---------------------------------------------------------------------------

/// A trajectory ready to be persisted, with its relation metadata.
struct PendingTrajectory {
    trajectory: Trajectory,
    /// Tool usages per node (indexed by node order).
    tool_usages: Vec<Vec<ToolUsage>>,
    /// Touched entities per node (indexed by node order).
    touched_entities: Vec<Vec<TouchedEntity>>,
}

fn build_trajectory(
    session_id: &str,
    buffer: &SessionBuffer,
    total_reward: f64,
) -> PendingTrajectory {
    let trajectory_id = Uuid::new_v4();
    let now = Utc::now();
    let duration_ms = (now - buffer.started_at).num_milliseconds().max(0) as u64;

    let mut nodes = Vec::with_capacity(buffer.decisions.len());
    let mut tool_usages = Vec::with_capacity(buffer.decisions.len());
    let mut touched_entities = Vec::with_capacity(buffer.decisions.len());

    let mut prev_ts = 0u64;

    for (i, decision) in buffer.decisions.iter().enumerate() {
        let delta_ms = if i == 0 {
            decision.timestamp_ms
        } else {
            decision.timestamp_ms.saturating_sub(prev_ts)
        };
        prev_ts = decision.timestamp_ms;

        nodes.push(TrajectoryNode {
            id: Uuid::new_v4(),
            context_embedding: decision.context_embedding.clone(),
            action_type: decision.action_type.clone(),
            action_params: decision.action_params.clone(),
            alternatives_count: decision.alternatives_count,
            chosen_index: decision.chosen_index,
            confidence: decision.confidence,
            local_reward: 0.0, // Will be filled by RewardDecomposer
            cumulative_reward: 0.0,
            delta_ms,
            order: i,
        });

        tool_usages.push(decision.tool_usages.clone());
        touched_entities.push(decision.touched_entities.clone());
    }

    // Use empty query_embedding for now — DecisionVector builder (T1.3) will fill this
    let query_embedding = if let Some(first) = buffer.decisions.first() {
        if !first.context_embedding.is_empty() {
            first.context_embedding.clone()
        } else {
            vec![0.0; 256]
        }
    } else {
        vec![0.0; 256]
    };

    PendingTrajectory {
        trajectory: Trajectory {
            id: trajectory_id,
            session_id: session_id.to_string(),
            query_embedding,
            total_reward,
            step_count: nodes.len(),
            duration_ms,
            nodes,
            created_at: now,
        },
        tool_usages,
        touched_entities,
    }
}

async fn flush_batch(store: &Neo4jTrajectoryStore, batch: Vec<PendingTrajectory>) {
    let count = batch.len();
    tracing::debug!(count, "Flushing trajectory batch");

    for pending in batch {
        // 1. Store the trajectory + nodes
        if let Err(e) = store.store_trajectory(&pending.trajectory).await {
            tracing::warn!(
                trajectory_id = %pending.trajectory.id,
                error = %e,
                "Failed to store trajectory"
            );
            continue;
        }

        // 2. Link tool usages and touched entities
        for (node, (tools, entities)) in pending.trajectory.nodes.iter().zip(
            pending
                .tool_usages
                .iter()
                .zip(pending.touched_entities.iter()),
        ) {
            if !tools.is_empty() {
                if let Err(e) = store.link_tool_usages_batch(&node.id, tools).await {
                    tracing::warn!(
                        node_id = %node.id,
                        error = %e,
                        "Failed to link tool usages"
                    );
                }
            }

            if !entities.is_empty() {
                if let Err(e) = store.link_touched_entities_batch(&node.id, entities).await {
                    tracing::warn!(
                        node_id = %node.id,
                        error = %e,
                        "Failed to link touched entities"
                    );
                }
            }
        }

        tracing::debug!(
            trajectory_id = %pending.trajectory.id,
            nodes = pending.trajectory.nodes.len(),
            "Flushed trajectory"
        );
    }

    tracing::info!(count, "Trajectory batch flush complete");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    fn make_decision(session_id: &str, action: &str, ts_ms: u64) -> DecisionRecord {
        DecisionRecord {
            session_id: session_id.to_string(),
            context_embedding: vec![],
            action_type: action.to_string(),
            action_params: serde_json::json!({"query": "test"}),
            alternatives_count: 3,
            chosen_index: 0,
            confidence: 0.85,
            tool_usages: vec![ToolUsage {
                tool_name: "code".to_string(),
                action: "search".to_string(),
                params_hash: "abc123".to_string(),
                duration_ms: Some(15),
                success: true,
            }],
            touched_entities: vec![TouchedEntity {
                entity_type: "file".to_string(),
                entity_id: "src/main.rs".to_string(),
                access_mode: "read".to_string(),
                relevance: Some(0.9),
            }],
            timestamp_ms: ts_ms,
        }
    }

    #[test]
    fn test_record_decision_does_not_block() {
        // Verify that record_decision returns instantly even with a full channel
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            // We can't use a real Neo4jTrajectoryStore in unit tests,
            // so just test the channel mechanics.
            let (tx, _rx) = mpsc::channel::<CollectorEvent>(2);
            let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));

            let collector = TrajectoryCollector { tx, enabled };

            // These should never block
            collector.record_decision(make_decision("s1", "code.search", 0));
            collector.record_decision(make_decision("s1", "note.get_context", 100));
            // Channel capacity is 2, this 3rd one should be silently dropped
            collector.record_decision(make_decision("s1", "code.analyze_impact", 200));
        });
    }

    #[test]
    fn test_disabled_collector_skips() {
        let (tx, mut rx) = mpsc::channel::<CollectorEvent>(10);
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let collector = TrajectoryCollector { tx, enabled };

        collector.record_decision(make_decision("s1", "code.search", 0));

        // Channel should be empty since collection is disabled
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_toggle_enabled() {
        let (tx, _rx) = mpsc::channel::<CollectorEvent>(10);
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));

        let collector = TrajectoryCollector { tx, enabled };

        assert!(collector.is_enabled());
        collector.set_enabled(false);
        assert!(!collector.is_enabled());
        collector.set_enabled(true);
        assert!(collector.is_enabled());
    }

    #[test]
    fn test_build_trajectory() {
        let started = Utc::now();
        let buffer = SessionBuffer {
            decisions: vec![
                make_decision("s1", "code.search", 0),
                make_decision("s1", "note.get_context", 50),
                make_decision("s1", "code.analyze_impact", 150),
            ],
            started_at: started,
        };

        let pending = build_trajectory("s1", &buffer, 0.85);

        assert_eq!(pending.trajectory.session_id, "s1");
        assert_eq!(pending.trajectory.step_count, 3);
        assert_eq!(pending.trajectory.total_reward, 0.85);
        assert_eq!(pending.trajectory.nodes.len(), 3);

        // Check ordering
        assert_eq!(pending.trajectory.nodes[0].order, 0);
        assert_eq!(pending.trajectory.nodes[1].order, 1);
        assert_eq!(pending.trajectory.nodes[2].order, 2);

        // Check delta_ms
        assert_eq!(pending.trajectory.nodes[0].delta_ms, 0);
        assert_eq!(pending.trajectory.nodes[1].delta_ms, 50);
        assert_eq!(pending.trajectory.nodes[2].delta_ms, 100);

        // Check tool usages preserved
        assert_eq!(pending.tool_usages.len(), 3);
        assert_eq!(pending.tool_usages[0].len(), 1);
        assert_eq!(pending.tool_usages[0][0].tool_name, "code");

        // Check touched entities preserved
        assert_eq!(pending.touched_entities.len(), 3);
        assert_eq!(pending.touched_entities[0].len(), 1);
        assert_eq!(pending.touched_entities[0][0].entity_type, "file");
    }

    #[test]
    fn bench_record_decision_latency_under_1ms() {
        // Benchmark: record_decision must complete in <1ms (fire-and-forget via try_send).
        // We measure 1000 iterations and assert p99 < 1ms.
        let (tx, _rx) = mpsc::channel::<CollectorEvent>(1000);
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let collector = TrajectoryCollector { tx, enabled };

        let mut durations = Vec::with_capacity(1000);

        for i in 0..1000 {
            let decision = make_decision("bench", "code.search", i);
            let start = std::time::Instant::now();
            collector.record_decision(decision);
            durations.push(start.elapsed());
        }

        durations.sort();
        let p50 = durations[499];
        let p99 = durations[989];
        let max = durations[999];

        eprintln!(
            "record_decision latency: p50={:?}, p99={:?}, max={:?}",
            p50, p99, max
        );

        // p99 must be under 1ms — typically <1μs (just a channel try_send)
        assert!(
            p99 < std::time::Duration::from_millis(1),
            "p99 latency {:?} exceeds 1ms budget",
            p99
        );
    }

    #[test]
    fn bench_record_decision_disabled_latency() {
        // When collection is disabled, record_decision should be near-zero (just an atomic load).
        let (tx, _rx) = mpsc::channel::<CollectorEvent>(10);
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let collector = TrajectoryCollector { tx, enabled };

        let mut durations = Vec::with_capacity(1000);

        for i in 0..1000 {
            let decision = make_decision("bench-off", "code.search", i);
            let start = std::time::Instant::now();
            collector.record_decision(decision);
            durations.push(start.elapsed());
        }

        durations.sort();
        let p99 = durations[989];

        eprintln!("record_decision (disabled) p99={:?}", p99);

        // Should be essentially free
        assert!(
            p99 < std::time::Duration::from_micros(100),
            "disabled p99 latency {:?} is too high",
            p99
        );
    }

    #[test]
    fn test_build_trajectory_empty() {
        let buffer = SessionBuffer {
            decisions: vec![],
            started_at: Utc::now(),
        };

        let pending = build_trajectory("empty", &buffer, 0.0);
        assert_eq!(pending.trajectory.step_count, 0);
        assert_eq!(pending.trajectory.nodes.len(), 0);
    }

    #[test]
    fn test_three_decisions_with_different_tools_and_entities() {
        // Simulates 3 decisions with different tools — verifies that USED_TOOL
        // and TOUCHED_ENTITY relations are correctly captured per node.
        let started = Utc::now();
        let decisions = vec![
            DecisionRecord {
                session_id: "s2".into(),
                context_embedding: vec![],
                action_type: "code.search".into(),
                action_params: serde_json::json!({"query": "TrajectoryStore"}),
                alternatives_count: 5,
                chosen_index: 0,
                confidence: 0.9,
                tool_usages: vec![ToolUsage {
                    tool_name: "code".into(),
                    action: "search".into(),
                    params_hash: "hash1".into(),
                    duration_ms: Some(12),
                    success: true,
                }],
                touched_entities: vec![
                    TouchedEntity {
                        entity_type: "file".into(),
                        entity_id: "src/store.rs".into(),
                        access_mode: "search_hit".into(),
                        relevance: Some(0.95),
                    },
                    TouchedEntity {
                        entity_type: "function".into(),
                        entity_id: "store_trajectory".into(),
                        access_mode: "search_hit".into(),
                        relevance: Some(0.88),
                    },
                ],
                timestamp_ms: 0,
            },
            DecisionRecord {
                session_id: "s2".into(),
                context_embedding: vec![],
                action_type: "note.get_context".into(),
                action_params: serde_json::json!({"entity_type": "file", "entity_id": "src/store.rs"}),
                alternatives_count: 2,
                chosen_index: 0,
                confidence: 0.75,
                tool_usages: vec![ToolUsage {
                    tool_name: "note".into(),
                    action: "get_context".into(),
                    params_hash: "hash2".into(),
                    duration_ms: Some(8),
                    success: true,
                }],
                touched_entities: vec![TouchedEntity {
                    entity_type: "note".into(),
                    entity_id: "note-uuid-123".into(),
                    access_mode: "context_load".into(),
                    relevance: Some(0.7),
                }],
                timestamp_ms: 50,
            },
            DecisionRecord {
                session_id: "s2".into(),
                context_embedding: vec![],
                action_type: "code.analyze_impact".into(),
                action_params: serde_json::json!({"target": "store_trajectory"}),
                alternatives_count: 3,
                chosen_index: 1,
                confidence: 0.82,
                tool_usages: vec![ToolUsage {
                    tool_name: "code".into(),
                    action: "analyze_impact".into(),
                    params_hash: "hash3".into(),
                    duration_ms: Some(25),
                    success: true,
                }],
                touched_entities: vec![
                    TouchedEntity {
                        entity_type: "file".into(),
                        entity_id: "src/store.rs".into(),
                        access_mode: "write".into(),
                        relevance: Some(1.0),
                    },
                    TouchedEntity {
                        entity_type: "file".into(),
                        entity_id: "src/models.rs".into(),
                        access_mode: "read".into(),
                        relevance: Some(0.6),
                    },
                ],
                timestamp_ms: 200,
            },
        ];

        let buffer = SessionBuffer {
            decisions,
            started_at: started,
        };

        let pending = build_trajectory("s2", &buffer, 0.78);

        // 3 nodes, 3 tool usages, 3 entity groups
        assert_eq!(pending.trajectory.nodes.len(), 3);
        assert_eq!(pending.tool_usages.len(), 3);
        assert_eq!(pending.touched_entities.len(), 3);

        // Node 0: code.search → 1 tool, 2 entities
        assert_eq!(pending.tool_usages[0].len(), 1);
        assert_eq!(pending.tool_usages[0][0].tool_name, "code");
        assert_eq!(pending.tool_usages[0][0].action, "search");
        assert_eq!(pending.touched_entities[0].len(), 2);
        assert_eq!(pending.touched_entities[0][0].entity_type, "file");
        assert_eq!(pending.touched_entities[0][1].entity_type, "function");

        // Node 1: note.get_context → 1 tool, 1 entity
        assert_eq!(pending.tool_usages[1].len(), 1);
        assert_eq!(pending.tool_usages[1][0].tool_name, "note");
        assert_eq!(pending.touched_entities[1].len(), 1);
        assert_eq!(pending.touched_entities[1][0].entity_type, "note");

        // Node 2: code.analyze_impact → 1 tool, 2 entities
        assert_eq!(pending.tool_usages[2].len(), 1);
        assert_eq!(pending.tool_usages[2][0].tool_name, "code");
        assert_eq!(pending.tool_usages[2][0].action, "analyze_impact");
        assert_eq!(pending.touched_entities[2].len(), 2);

        // Verify action types on trajectory nodes
        assert_eq!(pending.trajectory.nodes[0].action_type, "code.search");
        assert_eq!(pending.trajectory.nodes[1].action_type, "note.get_context");
        assert_eq!(
            pending.trajectory.nodes[2].action_type,
            "code.analyze_impact"
        );

        // Verify delta_ms
        assert_eq!(pending.trajectory.nodes[0].delta_ms, 0);
        assert_eq!(pending.trajectory.nodes[1].delta_ms, 50);
        assert_eq!(pending.trajectory.nodes[2].delta_ms, 150);
    }

    #[tokio::test]
    async fn test_collector_event_flow() {
        // Verify events flow through the channel correctly
        let (tx, mut rx) = mpsc::channel::<CollectorEvent>(100);
        let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));

        let collector = TrajectoryCollector { tx, enabled };

        // Send 3 decisions for session "flow-test"
        collector.record_decision(make_decision("flow-test", "code.search", 0));
        collector.record_decision(make_decision("flow-test", "note.get_context", 50));
        collector.record_decision(make_decision("flow-test", "code.analyze_impact", 150));
        collector.end_session("flow-test".to_string(), 0.9);

        // Verify 4 events in channel (3 decisions + 1 end_session)
        let mut event_count = 0;
        while let Ok(event) = rx.try_recv() {
            match event {
                CollectorEvent::Decision(d) => {
                    assert_eq!(d.session_id, "flow-test");
                    event_count += 1;
                }
                CollectorEvent::EndSession {
                    session_id,
                    total_reward,
                } => {
                    assert_eq!(session_id, "flow-test");
                    assert!((total_reward - 0.9).abs() < 1e-10);
                    event_count += 1;
                }
                _ => {}
            }
        }
        assert_eq!(event_count, 4);
    }
}
