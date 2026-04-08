//! Blueprint data model and DTOs
//!
//! Defines the core types for the Blueprint Engine:
//! - [`BlueprintNode`]: A curated knowledge template with tiered content
//! - [`BlueprintScope`]: Taxonomy scope (Scaffolding, Feature, Pattern, Process)
//! - [`BlueprintCategory`]: Domain category matching repo folder structure
//! - [`BlueprintStatus`]: Lifecycle status (Draft, Active, Archived)
//! - [`BlueprintRelation`]: Typed relation between blueprints (DependsOn, PairsWith)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

// ============================================================================
// Enums
// ============================================================================

/// Taxonomy scope — what level of work does this blueprint address?
///
/// Determines when an agent should consider this blueprint:
/// - Scaffolding: project bootstrap, architecture decisions
/// - Feature: feature-level implementation (OAuth, payments, push)
/// - Pattern: code-level reusable patterns (ViewModel, service layer)
/// - Process: workflow, CI/CD, release processes
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintScope {
    /// App-level: project kickoff, architecture decisions
    Scaffolding,
    /// Feature-level: OAuth, push notifications, payments
    Feature,
    /// Code-level: AsyncNotifier, ViewModel, service layer
    #[default]
    Pattern,
    /// Workflow-level: PR strategy, release checklist, CI/CD
    Process,
}

impl fmt::Display for BlueprintScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scaffolding => write!(f, "scaffolding"),
            Self::Feature => write!(f, "feature"),
            Self::Pattern => write!(f, "pattern"),
            Self::Process => write!(f, "process"),
        }
    }
}

impl FromStr for BlueprintScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "scaffolding" => Ok(Self::Scaffolding),
            "feature" => Ok(Self::Feature),
            "pattern" => Ok(Self::Pattern),
            "process" => Ok(Self::Process),
            _ => Err(format!("Unknown blueprint scope: {}", s)),
        }
    }
}

/// Domain category — matches the folder structure in the blueprint repo.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintCategory {
    /// project-setup/
    ProjectSetup,
    /// ci-cd/
    CiCd,
    /// architecture/
    Architecture,
    /// workflow/
    Workflow,
    /// patterns/
    #[default]
    Patterns,
    /// testing/
    Testing,
}

impl fmt::Display for BlueprintCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectSetup => write!(f, "project_setup"),
            Self::CiCd => write!(f, "ci_cd"),
            Self::Architecture => write!(f, "architecture"),
            Self::Workflow => write!(f, "workflow"),
            Self::Patterns => write!(f, "patterns"),
            Self::Testing => write!(f, "testing"),
        }
    }
}

impl FromStr for BlueprintCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "project_setup" | "project-setup" => Ok(Self::ProjectSetup),
            "ci_cd" | "ci-cd" => Ok(Self::CiCd),
            "architecture" => Ok(Self::Architecture),
            "workflow" => Ok(Self::Workflow),
            "patterns" => Ok(Self::Patterns),
            "testing" => Ok(Self::Testing),
            _ => Err(format!("Unknown blueprint category: {}", s)),
        }
    }
}

/// Lifecycle status of a Blueprint.
///
/// Simpler than SkillStatus — blueprints are curated, not emergent.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintStatus {
    /// Newly ingested, not yet reviewed
    #[default]
    Draft,
    /// Validated and available for agent use
    Active,
    /// Removed or superseded, preserved for history
    Archived,
}

impl fmt::Display for BlueprintStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
        }
    }
}

impl FromStr for BlueprintStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            _ => Err(format!("Unknown blueprint status: {}", s)),
        }
    }
}

impl BlueprintStatus {
    /// Returns true if transitioning from `self` to `to` is valid.
    ///
    /// Valid transitions:
    /// - Draft → Active, Archived
    /// - Active → Archived
    /// - Same status → always valid (no-op)
    pub fn can_transition_to(self, to: Self) -> bool {
        if self == to {
            return true;
        }
        matches!(
            (self, to),
            (Self::Draft, Self::Active)
                | (Self::Draft, Self::Archived)
                | (Self::Active, Self::Archived)
        )
    }
}

/// Type of relation between two blueprints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintRelationType {
    /// Blueprint A requires Blueprint B (transitive in resolution)
    DependsOn,
    /// Blueprint A complements Blueprint B (suggested together)
    PairsWith,
}

impl fmt::Display for BlueprintRelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DependsOn => write!(f, "DEPENDS_ON"),
            Self::PairsWith => write!(f, "PAIRS_WITH"),
        }
    }
}

impl FromStr for BlueprintRelationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "DEPENDS_ON" | "depends_on" => Ok(Self::DependsOn),
            "PAIRS_WITH" | "pairs_with" => Ok(Self::PairsWith),
            _ => Err(format!("Unknown blueprint relation type: {}", s)),
        }
    }
}

/// Difficulty level for a blueprint.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintDifficulty {
    Beginner,
    #[default]
    Intermediate,
    Advanced,
}

impl fmt::Display for BlueprintDifficulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Beginner => write!(f, "beginner"),
            Self::Intermediate => write!(f, "intermediate"),
            Self::Advanced => write!(f, "advanced"),
        }
    }
}

impl FromStr for BlueprintDifficulty {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "beginner" => Ok(Self::Beginner),
            "intermediate" => Ok(Self::Intermediate),
            "advanced" => Ok(Self::Advanced),
            _ => Err(format!("Unknown difficulty: {}", s)),
        }
    }
}

// ============================================================================
// Core entity
// ============================================================================

/// A Blueprint — curated knowledge template with tiered content.
///
/// Blueprints are sourced from markdown files in a git repository,
/// parsed into structured metadata + 3 tiers of content for token-efficient
/// loading. They form a dependency graph (DEPENDS_ON, PAIRS_WITH) and
/// are resolved per-project into a cached manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintNode {
    /// Unique identifier
    pub id: Uuid,
    /// URL-friendly identifier derived from file path (e.g. "riverpod-provider-wiring")
    pub slug: String,
    /// Human-readable name (from H1 in markdown)
    pub name: String,
    /// Short description — the TL;DR section (~1-3 sentences)
    #[serde(default)]
    pub description: String,
    /// Taxonomy scope: what level of work this addresses
    #[serde(default)]
    pub scope: BlueprintScope,
    /// Domain category: matches repo folder structure
    #[serde(default)]
    pub category: BlueprintCategory,
    /// Difficulty level
    #[serde(default)]
    pub difficulty: BlueprintDifficulty,
    /// Estimated time (free-form string like "30 min", "2 hours")
    #[serde(default)]
    pub estimated_time: String,
    /// Technology stack tags (e.g. ["flutter", "dart", "riverpod"])
    #[serde(default)]
    pub stack: Vec<String>,

    // --- Tiered content (token budget control) ---
    /// Tier 1 — Catalog: name + one-line description (~30 tokens)
    /// Auto-generated from the blockquote after H1
    #[serde(default)]
    pub tier1: String,
    /// Tier 2 — Summary: TL;DR + gotchas + checklist (~200 tokens)
    /// Extracted from specific markdown sections
    #[serde(default)]
    pub tier2: String,
    /// Tier 3 — Full: complete markdown content (~800 tokens)
    /// The raw markdown file content
    #[serde(default)]
    pub tier3_content: String,

    // --- Source tracking ---
    /// Path to source file in the blueprint repo (e.g. "patterns/riverpod-provider-wiring.md")
    #[serde(default)]
    pub source_file: String,
    /// SHA-256 hash of the source file content (for change detection)
    #[serde(default)]
    pub content_hash: String,

    // --- Lifecycle ---
    /// Current status
    #[serde(default)]
    pub status: BlueprintStatus,
    /// Schema version (bumped on content updates)
    #[serde(default = "default_version")]
    pub version: i64,

    // --- Categorization ---
    /// Free-form tags for search and filtering
    #[serde(default)]
    pub tags: Vec<String>,

    // --- Metrics ---
    /// Number of projects that have this blueprint in their manifest
    #[serde(default)]
    pub usage_count: i64,

    // --- Timestamps ---
    /// When this blueprint was first ingested
    pub created_at: DateTime<Utc>,
    /// Last modification
    pub updated_at: DateTime<Utc>,
}

fn default_version() -> i64 {
    1
}

impl BlueprintNode {
    /// Create a new blueprint with minimal required fields.
    pub fn new(slug: impl Into<String>, name: impl Into<String>) -> Self {
        let now = Utc::now();
        let slug = slug.into();
        let name = name.into();
        Self {
            id: Uuid::new_v4(),
            slug,
            name: name.clone(),
            description: String::new(),
            scope: BlueprintScope::default(),
            category: BlueprintCategory::default(),
            difficulty: BlueprintDifficulty::default(),
            estimated_time: String::new(),
            stack: vec![],
            tier1: String::new(),
            tier2: String::new(),
            tier3_content: String::new(),
            source_file: String::new(),
            content_hash: String::new(),
            status: BlueprintStatus::Draft,
            version: 1,
            tags: vec![],
            usage_count: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

// ============================================================================
// Relation between blueprints
// ============================================================================

/// A directed relation between two blueprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintRelation {
    /// Source blueprint ID
    pub from_id: Uuid,
    /// Target blueprint ID
    pub to_id: Uuid,
    /// Type of relation
    pub relation_type: BlueprintRelationType,
    /// When this relation was created
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Manifest entry (cached resolution result)
// ============================================================================

/// A single entry in a project's blueprint manifest.
///
/// The manifest is stored as a note (type: "blueprint_manifest") linked
/// to the project. It contains a JSON array of these entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Blueprint UUID
    pub blueprint_id: Uuid,
    /// Blueprint slug for human readability
    pub slug: String,
    /// Blueprint name
    pub name: String,
    /// Scope for filtering by task type
    pub scope: BlueprintScope,
    /// Category for additional filtering
    pub category: BlueprintCategory,
    /// Relevance score (0.0-1.0) — how well this blueprint matches the project
    pub relevance: f64,
}

// ============================================================================
// DTOs — Request/Response
// ============================================================================

/// Request to create a blueprint manually (sync creates them automatically).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBlueprintRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scope: Option<BlueprintScope>,
    #[serde(default)]
    pub category: Option<BlueprintCategory>,
    #[serde(default)]
    pub difficulty: Option<BlueprintDifficulty>,
    #[serde(default)]
    pub estimated_time: Option<String>,
    #[serde(default)]
    pub stack: Option<Vec<String>>,
    #[serde(default)]
    pub tier1: Option<String>,
    #[serde(default)]
    pub tier2: Option<String>,
    #[serde(default)]
    pub tier3_content: Option<String>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Request to update an existing blueprint (partial update).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateBlueprintRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scope: Option<BlueprintScope>,
    #[serde(default)]
    pub category: Option<BlueprintCategory>,
    #[serde(default)]
    pub difficulty: Option<BlueprintDifficulty>,
    #[serde(default)]
    pub estimated_time: Option<String>,
    #[serde(default)]
    pub stack: Option<Vec<String>>,
    #[serde(default)]
    pub tier1: Option<String>,
    #[serde(default)]
    pub tier2: Option<String>,
    #[serde(default)]
    pub tier3_content: Option<String>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub status: Option<BlueprintStatus>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Query parameters for listing blueprints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListBlueprintsQuery {
    /// Filter by scope
    #[serde(default)]
    pub scope: Option<BlueprintScope>,
    /// Filter by category
    #[serde(default)]
    pub category: Option<BlueprintCategory>,
    /// Filter by stack (any match)
    #[serde(default)]
    pub stack: Option<String>,
    /// Filter by status
    #[serde(default)]
    pub status: Option<BlueprintStatus>,
    /// Search in name/description
    #[serde(default)]
    pub search: Option<String>,
    /// Content tier to return (1, 2, or 3). Default: 1
    #[serde(default = "default_tier")]
    pub tier: i32,
    /// Pagination limit
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Pagination offset
    #[serde(default)]
    pub offset: usize,
}

fn default_tier() -> i32 {
    1
}
fn default_limit() -> usize {
    50
}

/// Response for a blueprint at a specific tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintResponse {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub scope: BlueprintScope,
    pub category: BlueprintCategory,
    pub difficulty: BlueprintDifficulty,
    pub estimated_time: String,
    pub stack: Vec<String>,
    pub status: BlueprintStatus,
    pub tags: Vec<String>,
    pub usage_count: i64,
    pub version: i64,
    /// Content at the requested tier
    pub content: String,
    /// Which tier was returned
    pub tier: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl BlueprintNode {
    /// Produce a response at the specified tier level.
    pub fn to_response(&self, tier: i32) -> BlueprintResponse {
        let content = match tier {
            1 => self.tier1.clone(),
            2 => self.tier2.clone(),
            3 => self.tier3_content.clone(),
            _ => self.tier1.clone(), // default to tier 1
        };
        BlueprintResponse {
            id: self.id,
            slug: self.slug.clone(),
            name: self.name.clone(),
            scope: self.scope,
            category: self.category,
            difficulty: self.difficulty,
            estimated_time: self.estimated_time.clone(),
            stack: self.stack.clone(),
            status: self.status,
            tags: self.tags.clone(),
            usage_count: self.usage_count,
            version: self.version,
            content,
            tier: tier.clamp(1, 3),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}
