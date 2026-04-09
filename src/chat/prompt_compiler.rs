//! Prompt Compiler Daemon — Intent refinement before agent execution.
//!
//! A single Haiku subprocess spawned at startup that intercepts user messages
//! between the enrichment pipeline and the Claude agent. Transforms vague user
//! input into structured, clear prompts.
//!
//! # Architecture
//!
//! - **One subprocess** for the entire runtime (Haiku, zero tools)
//! - **Session memory in Rust** — `HashMap<(project, session), State>`, not in LLM context
//! - **Self-contained messages** — each `refine()` call includes frontmatter + recent history
//! - **Graceful degradation** — timeout or contention → passthrough (original prompt)
//!
//! # System prompt lifecycle
//!
//! 1. `DEFAULT_COMPILER_PROMPT` hardcoded in this file (source of truth)
//! 2. At startup: load persona `prompt-compiler` from Neo4j if exists, else create from default
//! 3. At runtime: feedback enriches the persona in Neo4j
//! 4. Periodically: manual sync of good patterns back into code

use crate::neo4j::models::{PersonaNode, PersonaOrigin, PersonaStatus};
use crate::neo4j::GraphStore;
use nexus_claude::{ClaudeCodeOptions, InteractiveClient};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Maximum number of recent refinements to keep per session
const MAX_RECENT_REFINEMENTS: usize = 5;

/// Maximum number of recent refinements to include in the Haiku message
const INCLUDE_RECENT: usize = 3;

/// Timeout for a single refinement call (ms)
const REFINE_TIMEOUT_MS: u64 = 500;

/// Timeout for acquiring the lock when contended (ms)
const LOCK_CONTENTION_TIMEOUT_MS: u64 = 100;

/// Minimum token estimate to trigger compilation (skip short commands)
const MIN_TOKENS_FOR_COMPILE: usize = 15;

/// Default system prompt for the Prompt Compiler.
/// This is the source of truth — the Neo4j persona evolves from this base.
pub const DEFAULT_COMPILER_PROMPT: &str = r#"You are a Prompt Compiler. Your ONLY job is to transform raw user input into a structured, clear prompt for a development agent.

## Rules
1. NEVER add information the user didn't imply — only CLARIFY intent
2. NEVER change the user's intent — a "fix" is a bug fix, not a refactor
3. Keep the refined prompt ≤ 1.5x the original length
4. If the input is already clear and actionable, return it UNCHANGED
5. Structure when helpful: [Action] [Target] [Constraints] [Expected outcome]
6. Preserve the user's language (French stays French, English stays English)
7. Don't expand scope — if user says "file X", don't add "and file Y"
8. Don't add tests/docs unless the user mentions them

## Multi-session awareness
Each message has a frontmatter header with project/language/session context.
Use this to calibrate your refinement — Rust debug ≠ Dart UI ≠ architecture discussion.

## Output format
Return ONLY the refined prompt text. No explanation, no markdown wrapper, no prefix.
If the input needs no refinement, return it exactly as-is."#;

/// Context passed to `refine()` for building the self-contained message.
#[derive(Debug, Clone)]
pub struct RefineContext {
    /// Project slug (e.g. "knowloop", "budget")
    pub project_slug: String,
    /// Session UUID
    pub session_id: String,
    /// Primary programming language of the project
    pub primary_language: String,
    /// Conversation type / intent (e.g. "debug", "architect", "plan_run", "general")
    pub conversation_type: String,
    /// Active persona name, if any
    pub persona: Option<String>,
    /// Parent session ID (for forks)
    pub parent_session: Option<String>,
    /// Whether this message is a tool-use continuation
    pub is_tool_continuation: bool,
    /// Fork type (e.g. "plan_run", "user", "agent")
    pub fork_type: Option<String>,
}

/// Per-session compiler state, stored in the Rust-side HashMap.
#[derive(Debug, Clone)]
pub struct SessionCompilerState {
    /// Recent (input, output) pairs for few-shot context
    pub last_refinements: VecDeque<(String, String)>,
    /// Learned hints specific to this session (e.g. "'le truc' = stream_response()")
    pub learned_hints: Vec<String>,
    /// Conversation type for this session
    pub conversation_type: String,
    /// Parent session slug (for inheriting hints)
    pub parent_slug: Option<String>,
}

impl SessionCompilerState {
    /// Create a new empty state from the refine context.
    pub fn new(ctx: &RefineContext) -> Self {
        Self {
            last_refinements: VecDeque::with_capacity(MAX_RECENT_REFINEMENTS),
            learned_hints: Vec::new(),
            conversation_type: ctx.conversation_type.clone(),
            parent_slug: ctx.parent_session.clone(),
        }
    }

    /// Push a new refinement exchange, evicting the oldest if at capacity.
    pub fn push_exchange(&mut self, input: &str, output: &str) {
        if self.last_refinements.len() >= MAX_RECENT_REFINEMENTS {
            self.last_refinements.pop_front();
        }
        self.last_refinements
            .push_back((input.to_string(), output.to_string()));
    }

    /// Format the N most recent refinements for injection into the Haiku message.
    pub fn format_recent(&self, n: usize) -> String {
        if self.last_refinements.is_empty() {
            return String::new();
        }
        self.last_refinements
            .iter()
            .rev()
            .take(n)
            .rev()
            .map(|(inp, out)| {
                format!(
                    "- Input: {:?} → Output: {:?}",
                    truncate(inp, 100),
                    truncate(out, 150)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Format learned hints for injection.
    pub fn format_hints(&self) -> String {
        if self.learned_hints.is_empty() {
            return String::new();
        }
        self.learned_hints
            .iter()
            .map(|h| format!("- {}", h))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Feedback signal sent to the compiler after a stream completes.
#[derive(Debug, Clone)]
pub enum CompilerFeedback {
    /// The agent completed the task without the user needing to reformulate
    Success {
        session_id: String,
        project_slug: String,
    },
    /// The user reformulated their message (original → new)
    Reformulation {
        session_id: String,
        project_slug: String,
        original: String,
        reformulated: String,
    },
    /// The agent asked for clarification (prompt was ambiguous)
    ClarificationNeeded {
        session_id: String,
        project_slug: String,
    },
}

impl CompilerFeedback {
    /// Format as a message to send to the compiler daemon.
    pub fn to_message(&self) -> String {
        match self {
            Self::Success {
                session_id,
                project_slug,
            } => {
                format!("[FEEDBACK project={} session={}] ✓ Last refinement was effective — task completed without reformulation.", project_slug, &session_id[..8.min(session_id.len())])
            }
            Self::Reformulation {
                session_id,
                project_slug,
                original,
                reformulated,
            } => {
                format!(
                    "[FEEDBACK project={} session={}] ✗ User reformulated.\nBefore: {:?}\nAfter: {:?}\nLearn from this — the refinement missed the user's true intent.",
                    project_slug, &session_id[..8.min(session_id.len())],
                    truncate(original, 200), truncate(reformulated, 200)
                )
            }
            Self::ClarificationNeeded {
                session_id,
                project_slug,
            } => {
                format!("[FEEDBACK project={} session={}] ⚠ Agent asked for clarification — the prompt was still ambiguous after refinement.", project_slug, &session_id[..8.min(session_id.len())])
            }
        }
    }
}

/// The Prompt Compiler daemon.
///
/// Spawns a single Haiku subprocess at startup. All sessions share this subprocess,
/// but each gets isolated context via the Rust-side `session_memory` HashMap.
pub struct PromptCompiler {
    /// The Haiku subprocess (behind Mutex for sequential access)
    client: Arc<Mutex<InteractiveClient>>,
    /// Per-(project, session) compiler state
    session_memory: RwLock<HashMap<(String, String), SessionCompilerState>>,
    /// Total refinements performed (stats)
    pub refinement_count: AtomicU64,
    /// Total bypasses (passthrough due to should_compile=false or contention)
    pub bypass_count: AtomicU64,
}

impl PromptCompiler {
    /// Spawn the Prompt Compiler daemon with the given system prompt.
    ///
    /// The subprocess is a Haiku instance with zero tools and a dedicated
    /// system prompt for prompt refinement.
    pub async fn start(system_prompt: &str) -> anyhow::Result<Self> {
        info!("[prompt_compiler] Starting Haiku daemon...");

        let options = ClaudeCodeOptions::builder()
            .model("claude-sonnet-4-20250514")
            .cwd(".")
            .system_prompt(system_prompt)
            .permission_mode(nexus_claude::PermissionMode::BypassPermissions)
            .max_turns(1)
            .include_partial_messages(false)
            .build();

        let mut client = InteractiveClient::new(options)
            .map_err(|e| anyhow::anyhow!("Failed to create PromptCompiler client: {}", e))?;

        client
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect PromptCompiler client: {}", e))?;

        info!("[prompt_compiler] Haiku daemon started successfully");

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            session_memory: RwLock::new(HashMap::new()),
            refinement_count: AtomicU64::new(0),
            bypass_count: AtomicU64::new(0),
        })
    }

    /// Name of the persona used by the Prompt Compiler in Neo4j.
    pub const PERSONA_NAME: &'static str = "prompt-compiler";

    /// Start the Prompt Compiler with persona lookup from Neo4j.
    ///
    /// 1. Looks for a global persona named "prompt-compiler" in Neo4j
    /// 2. If found → uses its `system_prompt_override` as system prompt
    /// 3. If not found → creates the persona from `DEFAULT_COMPILER_PROMPT`
    /// 4. Falls back to hardcoded default if Neo4j is unavailable
    pub async fn start_with_persona(graph: &Arc<dyn GraphStore>) -> anyhow::Result<Self> {
        let system_prompt = match Self::load_or_create_persona(graph).await {
            Ok(prompt) => prompt,
            Err(e) => {
                warn!(
                    "[prompt_compiler] Failed to load/create persona: {} — using hardcoded default",
                    e
                );
                DEFAULT_COMPILER_PROMPT.to_string()
            }
        };

        Self::start(&system_prompt).await
    }

    /// Load the prompt-compiler persona from Neo4j, or create it if absent.
    ///
    /// Returns the system prompt to use (from persona.system_prompt_override or default).
    async fn load_or_create_persona(graph: &Arc<dyn GraphStore>) -> anyhow::Result<String> {
        // Search among global personas
        let globals = graph.list_global_personas().await?;
        let existing = globals.iter().find(|p| p.name == Self::PERSONA_NAME);

        if let Some(persona) = existing {
            let prompt = persona
                .system_prompt_override
                .as_deref()
                .unwrap_or(DEFAULT_COMPILER_PROMPT);
            info!(
                "[prompt_compiler] Loaded persona '{}' (id={}, prompt_len={})",
                Self::PERSONA_NAME,
                persona.id,
                prompt.len()
            );
            Ok(prompt.to_string())
        } else {
            // Create the persona
            info!(
                "[prompt_compiler] Persona '{}' not found — creating from default",
                Self::PERSONA_NAME
            );
            let persona = PersonaNode {
                id: uuid::Uuid::new_v4(),
                project_id: None, // Global persona
                name: Self::PERSONA_NAME.to_string(),
                description:
                    "Prompt Compiler daemon — refines user prompts before agent execution. \
                    Transforms vague input into structured, clear prompts."
                        .to_string(),
                status: PersonaStatus::Active,
                complexity_default: None,
                timeout_secs: None,
                max_cost_usd: None,
                model_preference: Some("haiku".to_string()),
                system_prompt_override: Some(DEFAULT_COMPILER_PROMPT.to_string()),
                energy: 1.0,
                cohesion: 0.0,
                activation_count: 0,
                success_rate: 0.0,
                avg_duration_secs: 0.0,
                last_activated: Some(chrono::Utc::now()),
                energy_boost_accumulated: 0.0,
                energy_history: vec![],
                origin: PersonaOrigin::Manual,
                created_at: chrono::Utc::now(),
                updated_at: None,
            };

            graph.create_persona(&persona).await?;
            info!(
                "[prompt_compiler] Created persona '{}' (id={})",
                Self::PERSONA_NAME,
                persona.id
            );

            Ok(DEFAULT_COMPILER_PROMPT.to_string())
        }
    }

    /// Check whether a prompt should go through the compiler.
    ///
    /// Returns `false` for short commands, tool-use continuations, and plan_run forks.
    pub fn should_compile(prompt: &str, ctx: &RefineContext) -> bool {
        // Skip tool-use continuations (agent flow)
        if ctx.is_tool_continuation {
            return false;
        }

        // Skip plan_run forks (already structured prompts)
        if ctx.fork_type.as_deref() == Some("plan_run") {
            return false;
        }

        // Skip very short inputs (direct commands)
        let estimated_tokens = prompt.split_whitespace().count();
        if estimated_tokens < MIN_TOKENS_FOR_COMPILE {
            return false;
        }

        true
    }

    /// Refine a user prompt.
    ///
    /// Builds a self-contained message with session context, sends to Haiku,
    /// and stores the result. On timeout or contention, returns the original prompt.
    pub async fn refine(&self, prompt: &str, ctx: &RefineContext) -> String {
        let start = std::time::Instant::now();

        // Try to acquire the lock — if contended, passthrough
        let client_guard = match tokio::time::timeout(
            Duration::from_millis(LOCK_CONTENTION_TIMEOUT_MS),
            self.client.lock(),
        )
        .await
        {
            Ok(guard) => guard,
            Err(_) => {
                debug!("[prompt_compiler] Lock contended, passthrough");
                self.bypass_count.fetch_add(1, Ordering::Relaxed);
                return prompt.to_string();
            }
        };

        // Build the self-contained message
        let message = self.build_compiler_message(prompt, ctx).await;

        // Send to Haiku with timeout
        let result = match tokio::time::timeout(
            Duration::from_millis(REFINE_TIMEOUT_MS),
            Self::send_and_extract(client_guard, &message),
        )
        .await
        {
            Ok(Ok(refined)) => {
                let elapsed = start.elapsed().as_millis();
                debug!(
                    "[prompt_compiler] Refined in {}ms: {:?} → {:?}",
                    elapsed,
                    truncate(prompt, 50),
                    truncate(&refined, 80)
                );
                self.refinement_count.fetch_add(1, Ordering::Relaxed);

                // Store in session memory
                let key = (ctx.project_slug.clone(), ctx.session_id.clone());
                let mut memory = self.session_memory.write().await;
                let state = memory
                    .entry(key)
                    .or_insert_with(|| SessionCompilerState::new(ctx));
                state.push_exchange(prompt, &refined);

                refined
            }
            Ok(Err(e)) => {
                warn!("[prompt_compiler] Error: {} — passthrough", e);
                self.bypass_count.fetch_add(1, Ordering::Relaxed);
                prompt.to_string()
            }
            Err(_) => {
                warn!(
                    "[prompt_compiler] Timeout ({}ms) — passthrough",
                    REFINE_TIMEOUT_MS
                );
                self.bypass_count.fetch_add(1, Ordering::Relaxed);
                prompt.to_string()
            }
        };

        result
    }

    /// Send a message to the compiler and extract the text response.
    async fn send_and_extract(
        mut client: tokio::sync::MutexGuard<'_, InteractiveClient>,
        message: &str,
    ) -> anyhow::Result<String> {
        // send_and_receive returns Vec<Message>
        let messages = client
            .send_and_receive(message.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("Compiler send failed: {}", e))?;

        // Extract text from assistant messages
        let mut text = String::new();
        for msg in &messages {
            if let nexus_claude::Message::Assistant { message, .. } = msg {
                for block in &message.content {
                    if let nexus_claude::ContentBlock::Text(t) = block {
                        text.push_str(&t.text);
                    }
                }
            }
        }

        if text.is_empty() {
            anyhow::bail!("Empty response from compiler");
        }

        Ok(text.trim().to_string())
    }

    /// Build the self-contained message for the Haiku compiler.
    ///
    /// Includes frontmatter (project, language, session type) + recent history
    /// from this session + learned hints. The Haiku doesn't need to remember
    /// anything — all context is in this message.
    async fn build_compiler_message(&self, prompt: &str, ctx: &RefineContext) -> String {
        let memory = self.session_memory.read().await;
        let key = (ctx.project_slug.clone(), ctx.session_id.clone());
        let state = memory.get(&key);

        let mut parts = Vec::with_capacity(4);

        // Frontmatter
        parts.push(format!(
            "---\nproject: {}\nlanguage: {}\nsession_type: {}\npersona: {}\n---",
            ctx.project_slug,
            ctx.primary_language,
            ctx.conversation_type,
            ctx.persona.as_deref().unwrap_or("none"),
        ));

        // Recent history (if any)
        if let Some(s) = state {
            let recent = s.format_recent(INCLUDE_RECENT);
            if !recent.is_empty() {
                parts.push(format!(
                    "## Recent refinements for this session\n{}",
                    recent
                ));
            }

            let hints = s.format_hints();
            if !hints.is_empty() {
                parts.push(format!("## Learned hints\n{}", hints));
            }
        }

        // The actual prompt to refine
        parts.push(format!("Refine this prompt:\n{}", prompt));

        parts.join("\n\n")
    }

    /// Send feedback to the compiler daemon (fire-and-forget).
    ///
    /// This enriches the Haiku's conversation context with success/failure signals,
    /// allowing it to learn which refinement patterns work.
    pub async fn feedback(&self, feedback: CompilerFeedback) {
        let msg = feedback.to_message();
        let client = self.client.clone();

        tokio::spawn(async move {
            // Best-effort — don't block on lock
            match tokio::time::timeout(Duration::from_millis(200), client.lock()).await {
                Ok(mut guard) => {
                    if let Err(e) = guard.send_and_receive(msg).await {
                        debug!("[prompt_compiler] Feedback send failed: {}", e);
                    }
                }
                Err(_) => {
                    debug!("[prompt_compiler] Feedback skipped (lock contended)");
                }
            }
        });
    }

    /// Get stats for monitoring.
    pub fn stats(&self) -> (u64, u64) {
        (
            self.refinement_count.load(Ordering::Relaxed),
            self.bypass_count.load(Ordering::Relaxed),
        )
    }

    /// Detect feedback signals from a completed stream and send to the compiler.
    ///
    /// Called by PostStreamHandler after each agent response. Detects:
    /// - Clarification patterns in agent response
    /// - Reformulation by comparing current user prompt with previous
    /// - Success (no reformulation, no clarification → effective refinement)
    pub async fn detect_and_send_feedback(
        &self,
        user_prompt: &str,
        assistant_text: &str,
        project_slug: &str,
        session_id: &str,
    ) {
        // Only send feedback if we actually refined this prompt
        let key = (project_slug.to_string(), session_id.to_string());
        let had_refinement = {
            let memory = self.session_memory.read().await;
            memory
                .get(&key)
                .is_some_and(|s| !s.last_refinements.is_empty())
        };
        if !had_refinement {
            return;
        }

        // 1. Check if agent asked for clarification
        if Self::detect_clarification(assistant_text) {
            info!(
                "[prompt_compiler] Detected clarification request in session {}",
                &session_id[..8.min(session_id.len())]
            );
            self.feedback(CompilerFeedback::ClarificationNeeded {
                session_id: session_id.to_string(),
                project_slug: project_slug.to_string(),
            })
            .await;

            // Learn hint: this kind of prompt was ambiguous
            self.add_hint(
                project_slug,
                session_id,
                format!(
                    "Ambiguous pattern: {:?} → agent asked for clarification",
                    truncate(user_prompt, 80)
                ),
            )
            .await;
            return;
        }

        // 2. Check if this looks like a reformulation of the previous prompt
        if let Some(prev_input) = self.get_previous_input(project_slug, session_id).await {
            if Self::detect_reformulation(&prev_input, user_prompt) {
                info!(
                    "[prompt_compiler] Detected reformulation in session {}",
                    &session_id[..8.min(session_id.len())]
                );
                self.feedback(CompilerFeedback::Reformulation {
                    session_id: session_id.to_string(),
                    project_slug: project_slug.to_string(),
                    original: prev_input,
                    reformulated: user_prompt.to_string(),
                })
                .await;
                return;
            }
        }

        // 3. Default: success signal (refinement was effective)
        self.feedback(CompilerFeedback::Success {
            session_id: session_id.to_string(),
            project_slug: project_slug.to_string(),
        })
        .await;
    }

    /// Detect if the agent's response contains clarification-seeking patterns.
    fn detect_clarification(assistant_text: &str) -> bool {
        let text_lower = assistant_text.to_lowercase();
        let first_500 = if text_lower.len() > 500 {
            &text_lower[..500]
        } else {
            &text_lower
        };

        // Only check the beginning — clarifications typically appear at the start
        let patterns = [
            "could you clarify",
            "can you clarify",
            "do you mean",
            "did you mean",
            "could you specify",
            "can you specify",
            "what do you mean by",
            "which one",
            "which file",
            "a few questions",
            "before i proceed",
            "before proceeding",
            "just to confirm",
            "je veux clarifier",
            "tu veux dire",
            "peux-tu préciser",
            "lequel",
            "c'est-à-dire",
            "tu parles de",
            "quelques questions",
            "avant de commencer",
        ];

        patterns.iter().any(|p| first_500.contains(p))
    }

    /// Detect if two consecutive user messages are reformulations of each other.
    ///
    /// Uses simple heuristics: word overlap ratio. If the user rephrases a similar
    /// request with different words, it's a reformulation signal.
    fn detect_reformulation(previous: &str, current: &str) -> bool {
        let prev_words: std::collections::HashSet<&str> = previous
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();

        let curr_words: std::collections::HashSet<&str> = current
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|w| w.len() > 2)
            .collect();

        if prev_words.is_empty() || curr_words.is_empty() {
            return false;
        }

        let intersection = prev_words.intersection(&curr_words).count();
        let union = prev_words.union(&curr_words).count();

        if union == 0 {
            return false;
        }

        let jaccard = intersection as f64 / union as f64;

        // 30-70% overlap = reformulation (too low = different topic, too high = repetition)
        (0.3..=0.7).contains(&jaccard)
    }

    /// Get the previous raw user input for a session (before refinement).
    async fn get_previous_input(&self, project_slug: &str, session_id: &str) -> Option<String> {
        let memory = self.session_memory.read().await;
        let key = (project_slug.to_string(), session_id.to_string());
        let state = memory.get(&key)?;

        if state.last_refinements.len() < 2 {
            return None;
        }

        // Get the second-to-last input (the one before the current)
        let idx = state.last_refinements.len() - 2;
        Some(state.last_refinements[idx].0.clone())
    }

    /// Add a learned hint to a session's compiler state.
    async fn add_hint(&self, project_slug: &str, session_id: &str, hint: String) {
        let key = (project_slug.to_string(), session_id.to_string());
        let mut memory = self.session_memory.write().await;
        if let Some(state) = memory.get_mut(&key) {
            if state.learned_hints.len() < 20 {
                // Cap hints
                state.learned_hints.push(hint);
            }
        }
    }

    /// Persist all learned hints to Neo4j as notes linked to the prompt-compiler persona.
    ///
    /// Called at shutdown or periodically. Converts session hints into persistent
    /// knowledge notes of type `prompt_pattern`.
    pub async fn persist_hints(&self, graph: &Arc<dyn GraphStore>) {
        let memory = self.session_memory.read().await;

        let mut all_hints: Vec<String> = Vec::new();
        for ((_project, _session), state) in memory.iter() {
            for hint in &state.learned_hints {
                if !all_hints.contains(hint) {
                    all_hints.push(hint.clone());
                }
            }
        }

        if all_hints.is_empty() {
            debug!("[prompt_compiler] No hints to persist");
            return;
        }

        info!(
            "[prompt_compiler] Persisting {} unique hints as notes",
            all_hints.len()
        );

        // Create a single aggregated note per session with all hints
        let content = format!(
            "## Prompt Compiler — Learned Patterns\n\n{}\n\n_Auto-generated from feedback signals._",
            all_hints
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{}. {}", i + 1, h))
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Create a global pattern note with the hints
        let mut note = crate::notes::models::Note::new(
            None, // Global
            crate::notes::models::NoteType::Pattern,
            content,
            "prompt-compiler".to_string(),
        );
        note.tags = vec!["prompt-compiler".to_string(), "auto-generated".to_string()];

        let note_id = note.id;
        match graph.create_note(&note).await {
            Ok(_) => {
                info!(
                    "[prompt_compiler] Persisted {} hints as note {}",
                    all_hints.len(),
                    note_id
                );
            }
            Err(e) => {
                warn!("[prompt_compiler] Failed to persist hints: {}", e);
            }
        }
    }
}

/// Truncate a string to `max_len` chars, appending "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_compile_short_input() {
        let ctx = RefineContext {
            project_slug: "test".into(),
            session_id: "abc".into(),
            primary_language: "rust".into(),
            conversation_type: "general".into(),
            persona: None,
            parent_session: None,
            is_tool_continuation: false,
            fork_type: None,
        };

        // Short input → skip
        assert!(!PromptCompiler::should_compile("oui", &ctx));
        assert!(!PromptCompiler::should_compile("git status", &ctx));

        // Long enough → compile
        assert!(PromptCompiler::should_compile(
            "fix the regression in the chat pipeline that causes messages to be dropped when the enrichment stage times out",
            &ctx
        ));
    }

    #[test]
    fn test_should_compile_tool_continuation() {
        let ctx = RefineContext {
            project_slug: "test".into(),
            session_id: "abc".into(),
            primary_language: "rust".into(),
            conversation_type: "general".into(),
            persona: None,
            parent_session: None,
            is_tool_continuation: true,
            fork_type: None,
        };
        assert!(!PromptCompiler::should_compile(
            "some long prompt that would normally be compiled by the system",
            &ctx
        ));
    }

    #[test]
    fn test_should_compile_plan_run() {
        let ctx = RefineContext {
            project_slug: "test".into(),
            session_id: "abc".into(),
            primary_language: "rust".into(),
            conversation_type: "general".into(),
            persona: None,
            parent_session: None,
            is_tool_continuation: false,
            fork_type: Some("plan_run".into()),
        };
        assert!(!PromptCompiler::should_compile(
            "implement the feature according to the task description and acceptance criteria",
            &ctx
        ));
    }

    #[test]
    fn test_session_state_push_exchange() {
        let ctx = RefineContext {
            project_slug: "test".into(),
            session_id: "abc".into(),
            primary_language: "rust".into(),
            conversation_type: "debug".into(),
            persona: None,
            parent_session: None,
            is_tool_continuation: false,
            fork_type: None,
        };

        let mut state = SessionCompilerState::new(&ctx);
        for i in 0..7 {
            state.push_exchange(&format!("input {}", i), &format!("output {}", i));
        }
        // Should keep only MAX_RECENT_REFINEMENTS
        assert_eq!(state.last_refinements.len(), MAX_RECENT_REFINEMENTS);
        // Oldest should be evicted
        assert_eq!(state.last_refinements.front().unwrap().0, "input 2");
    }

    #[test]
    fn test_format_recent() {
        let ctx = RefineContext {
            project_slug: "test".into(),
            session_id: "abc".into(),
            primary_language: "rust".into(),
            conversation_type: "debug".into(),
            persona: None,
            parent_session: None,
            is_tool_continuation: false,
            fork_type: None,
        };

        let mut state = SessionCompilerState::new(&ctx);
        state.push_exchange("fix le truc", "Fix the regression in stream_response");
        state.push_exchange("continue", "Continue fixing stream_response");

        let formatted = state.format_recent(3);
        assert!(formatted.contains("fix le truc"));
        assert!(formatted.contains("Continue fixing"));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world foo bar", 11), "hello world...");
    }

    #[test]
    fn test_compiler_feedback_format() {
        let fb = CompilerFeedback::Success {
            session_id: "abc12345-def".into(),
            project_slug: "knowloop".into(),
        };
        let msg = fb.to_message();
        assert!(msg.contains("project=knowloop"));
        assert!(msg.contains("session=abc12345"));
        assert!(msg.contains("✓"));
    }

    #[test]
    fn test_detect_clarification_english() {
        assert!(PromptCompiler::detect_clarification(
            "Could you clarify what you mean by 'the pipeline'? There are several pipelines in the codebase."
        ));
        assert!(PromptCompiler::detect_clarification(
            "Before I proceed, I want to make sure I understand the requirement correctly."
        ));
        assert!(PromptCompiler::detect_clarification(
            "Which file are you referring to? There are multiple candidates."
        ));
    }

    #[test]
    fn test_detect_clarification_french() {
        assert!(PromptCompiler::detect_clarification(
            "Peux-tu préciser ce que tu veux dire par 'le truc' ?"
        ));
        assert!(PromptCompiler::detect_clarification(
            "Avant de commencer, je voudrais confirmer l'approche."
        ));
    }

    #[test]
    fn test_detect_clarification_negative() {
        // Normal productive responses should NOT trigger
        assert!(!PromptCompiler::detect_clarification(
            "I'll fix the bug in stream_response. The issue is in the retry loop."
        ));
        assert!(!PromptCompiler::detect_clarification(
            "Here's the implementation for the new feature."
        ));
    }

    #[test]
    fn test_detect_reformulation_overlap() {
        // Similar topic, different phrasing → reformulation
        assert!(PromptCompiler::detect_reformulation(
            "fix the bug in the chat pipeline where messages are dropped",
            "the chat pipeline drops messages when enrichment times out, please fix it",
        ));

        // Completely different topics → not reformulation
        assert!(!PromptCompiler::detect_reformulation(
            "fix the bug in the chat pipeline",
            "add a new API endpoint for user profiles",
        ));

        // Nearly identical → not reformulation (too high overlap = repetition)
        assert!(!PromptCompiler::detect_reformulation(
            "fix the bug in the chat pipeline",
            "fix the bug in the chat pipeline please",
        ));
    }

    #[test]
    fn test_detect_reformulation_short_inputs() {
        // Too short → skip
        assert!(!PromptCompiler::detect_reformulation("go", "ok"));
        assert!(!PromptCompiler::detect_reformulation("", "fix bug"));
    }
}
