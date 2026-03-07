//! Event-driven protocol triggers
//!
//! Provides the hook mechanism for auto-triggering protocols after system events
//! (post_sync, post_import, post_plan_complete). Called from REST handlers as
//! fire-and-forget background tasks via `tokio::spawn`.
//!
//! # Event Types
//!
//! - `post_sync` — after `admin(sync_directory)`, `project(sync)`, or `commit(create)` with files
//! - `post_import` — after `skill(import)`
//! - `post_plan_complete` — after a plan reaches `completed` status (future)
//!
//! # Debounce
//!
//! Each protocol has a `last_triggered_at` timestamp. Auto-triggers are skipped
//! if the protocol was triggered less than [`MIN_TRIGGER_INTERVAL_SECS`] seconds ago.

use crate::neo4j::traits::GraphStore;
use crate::protocol::engine;
use std::sync::Arc;
use uuid::Uuid;

/// Minimum interval between auto-triggered runs of the same protocol (5 minutes).
const MIN_TRIGGER_INTERVAL_SECS: i64 = 300;

/// Spawn event-triggered protocol runs in the background.
///
/// Finds all protocols for the given project that listen to the specified event
/// (trigger_mode = Event or Auto, with the event in trigger_config.events),
/// and starts a run for each one.
///
/// This is a fire-and-forget operation — errors are logged, never propagated.
pub fn spawn_event_triggered_protocols(
    store: Arc<dyn GraphStore>,
    project_id: Uuid,
    event: &str,
) {
    let event = event.to_string();
    tokio::spawn(async move {
        if let Err(e) = trigger_protocols_for_event(&*store, project_id, &event).await {
            tracing::warn!(
                %project_id,
                event = %event,
                "Event-triggered protocol hook failed: {}", e
            );
        }
    });
}

/// Core logic: find matching protocols and start runs.
async fn trigger_protocols_for_event(
    store: &dyn GraphStore,
    project_id: Uuid,
    event: &str,
) -> anyhow::Result<()> {
    // List all protocols for the project (reasonable upper bound — few protocols per project)
    let (protocols, _) = store.list_protocols(project_id, None, 100, 0).await?;

    let now = chrono::Utc::now();
    let mut triggered_count = 0u32;

    for protocol in &protocols {
        // 1. Check trigger_mode listens to events
        if !protocol.trigger_mode.listens_to_events() {
            continue;
        }

        // 2. Check trigger_config contains the event
        let has_event = protocol
            .trigger_config
            .as_ref()
            .map(|c| c.events.iter().any(|e| e == event))
            .unwrap_or(false);

        if !has_event {
            continue;
        }

        // 3. Debounce: skip if last triggered less than MIN_TRIGGER_INTERVAL ago
        if let Some(last) = protocol.last_triggered_at {
            let elapsed_secs = (now - last).num_seconds();
            if elapsed_secs < MIN_TRIGGER_INTERVAL_SECS {
                tracing::debug!(
                    protocol_id = %protocol.id,
                    protocol_name = %protocol.name,
                    event = %event,
                    elapsed_secs,
                    "Skipping event trigger: debounce ({elapsed_secs}s < {MIN_TRIGGER_INTERVAL_SECS}s)"
                );
                continue;
            }
        }

        // 4. Start the run with triggered_by set
        let triggered_by = format!("event:{event}");
        match engine::start_run(store, protocol.id, None, None, Some(&triggered_by)).await {
            Ok(run) => {
                tracing::info!(
                    protocol_id = %protocol.id,
                    protocol_name = %protocol.name,
                    run_id = %run.id,
                    event = %event,
                    "Auto-triggered protocol run"
                );

                // 5. Update last_triggered_at on the protocol
                let mut updated_protocol = protocol.clone();
                updated_protocol.last_triggered_at = Some(now);
                updated_protocol.updated_at = now;
                if let Err(e) = store.upsert_protocol(&updated_protocol).await {
                    tracing::warn!(
                        protocol_id = %protocol.id,
                        "Failed to update last_triggered_at: {}", e
                    );
                }

                triggered_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    protocol_id = %protocol.id,
                    protocol_name = %protocol.name,
                    event = %event,
                    "Failed to auto-trigger protocol: {}", e
                );
            }
        }
    }

    if triggered_count > 0 {
        tracing::info!(
            %project_id,
            event = %event,
            count = triggered_count,
            "Event-triggered {triggered_count} protocol run(s)"
        );
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neo4j::mock::MockGraphStore;
    use crate::protocol::{
        Protocol, ProtocolCategory, ProtocolState, ProtocolTransition, TriggerConfig, TriggerMode,
    };

    /// Helper to set up a 3-state protocol with event trigger config.
    async fn setup_event_triggered_protocol(
        store: &MockGraphStore,
        trigger_mode: TriggerMode,
        events: Vec<String>,
    ) -> (Uuid, Protocol) {
        let project_id = Uuid::new_v4();
        let project = crate::neo4j::models::ProjectNode {
            id: project_id,
            name: "test-project".to_string(),
            slug: "test-project".to_string(),
            description: None,
            root_path: "/tmp/test".to_string(),
            created_at: chrono::Utc::now(),
            last_synced: None,
            analytics_computed_at: None,
            last_co_change_computed_at: None,
        };
        store.create_project(&project).await.unwrap();

        let protocol_id = Uuid::new_v4();
        let start_state = ProtocolState::start(protocol_id, "Start");
        let done_state = ProtocolState::terminal(protocol_id, "Done");

        let mut protocol = Protocol::new_full(
            project_id,
            "Test Event Protocol",
            "Protocol for event-trigger testing",
            start_state.id,
            vec![done_state.id],
            ProtocolCategory::System,
        );
        protocol.id = protocol_id;
        protocol.trigger_mode = trigger_mode;
        protocol.trigger_config = Some(TriggerConfig {
            events,
            schedule: None,
            conditions: vec![],
        });

        store.upsert_protocol(&protocol).await.unwrap();
        store.upsert_protocol_state(&start_state).await.unwrap();
        store.upsert_protocol_state(&done_state).await.unwrap();

        let t1 =
            ProtocolTransition::new(protocol_id, start_state.id, done_state.id, "complete");
        store.upsert_protocol_transition(&t1).await.unwrap();

        (project_id, protocol)
    }

    #[tokio::test]
    async fn test_event_trigger_starts_run() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Event,
            vec!["post_sync".to_string()],
        )
        .await;

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        // Verify a run was created
        let (runs, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(runs[0].triggered_by, "event:post_sync");
    }

    #[tokio::test]
    async fn test_event_trigger_auto_mode() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Auto,
            vec!["post_sync".to_string(), "post_import".to_string()],
        )
        .await;

        // post_sync should trigger
        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        let (runs, _) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].triggered_by, "event:post_sync");
    }

    #[tokio::test]
    async fn test_event_trigger_ignores_manual_mode() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Manual,
            vec!["post_sync".to_string()],
        )
        .await;

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        // No run should be created
        let (runs, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn test_event_trigger_ignores_scheduled_only() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Scheduled,
            vec!["post_sync".to_string()],
        )
        .await;

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        let (_, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_event_trigger_wrong_event_ignored() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Event,
            vec!["post_import".to_string()],
        )
        .await;

        // post_sync should NOT trigger a protocol listening only to post_import
        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        let (_, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_event_trigger_debounce() {
        let store = MockGraphStore::new();
        let (project_id, mut protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Event,
            vec!["post_sync".to_string()],
        )
        .await;

        // Set last_triggered_at to 1 minute ago (within debounce window)
        protocol.last_triggered_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(60));
        store.upsert_protocol(&protocol).await.unwrap();

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        // Should be debounced — no run created
        let (_, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_event_trigger_after_debounce_window() {
        let store = MockGraphStore::new();
        let (project_id, mut protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Event,
            vec!["post_sync".to_string()],
        )
        .await;

        // Set last_triggered_at to 10 minutes ago (outside debounce window)
        protocol.last_triggered_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(600));
        store.upsert_protocol(&protocol).await.unwrap();

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        // Should trigger — debounce window has passed
        let (runs, total) = store
            .list_protocol_runs(protocol.id, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(runs[0].triggered_by, "event:post_sync");
    }

    #[tokio::test]
    async fn test_event_trigger_no_protocols() {
        let store = MockGraphStore::new();
        let project_id = Uuid::new_v4();

        // Create only the project (no protocols)
        let project = crate::neo4j::models::ProjectNode {
            id: project_id,
            name: "empty-project".to_string(),
            slug: "empty-project".to_string(),
            description: None,
            root_path: "/tmp/empty".to_string(),
            created_at: chrono::Utc::now(),
            last_synced: None,
            analytics_computed_at: None,
            last_co_change_computed_at: None,
        };
        store.create_project(&project).await.unwrap();

        // Should succeed silently (no protocols to trigger)
        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_event_trigger_updates_last_triggered_at() {
        let store = MockGraphStore::new();
        let (project_id, protocol) = setup_event_triggered_protocol(
            &store,
            TriggerMode::Event,
            vec!["post_sync".to_string()],
        )
        .await;

        assert!(protocol.last_triggered_at.is_none());

        trigger_protocols_for_event(&store, project_id, "post_sync")
            .await
            .unwrap();

        // Verify last_triggered_at was updated
        let updated = store.get_protocol(protocol.id).await.unwrap().unwrap();
        assert!(updated.last_triggered_at.is_some());
    }
}
