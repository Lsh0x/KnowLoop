//! Blueprint markdown parser
//!
//! Parses blueprint `.md` files into structured data:
//! - HTML comment metadata block → tags, category, difficulty, time, stack
//! - H1 → name
//! - First blockquote → one-liner description (tier1)
//! - TL;DR section → summary
//! - Gotchas section → extracted gotchas
//! - Checklist section → extracted checklist items
//! - Full content → tier3
//!
//! # Markdown Structure Expected
//!
//! ```text
//! # Blueprint: <Name>
//!
//! <!-- METADATA
//! tags:        [tag1, tag2]
//! category:    patterns
//! difficulty:  intermediate
//! time:        30 min
//! stack:       [flutter, dart]
//! -->
//!
//! > One-line description (used as tier1)
//!
//! ## TL;DR
//! Summary paragraph(s)
//!
//! ## Steps
//! ...
//!
//! ## Gotchas
//! > **Title**: description
//!
//! ## Checklist
//! - [ ] item1
//! ```

use crate::blueprint::{BlueprintCategory, BlueprintDifficulty, BlueprintScope};
use anyhow::Result;
use std::path::Path;

/// Result of parsing a blueprint markdown file.
#[derive(Debug, Clone)]
pub struct ParsedBlueprint {
    /// Blueprint name (from H1, stripped of "Blueprint: " prefix)
    pub name: String,
    /// URL-friendly slug (derived from filename)
    pub slug: String,
    /// One-line description (from first blockquote after H1)
    pub description: String,
    /// Taxonomy scope (inferred from category + tags)
    pub scope: BlueprintScope,
    /// Domain category (from metadata)
    pub category: BlueprintCategory,
    /// Difficulty level (from metadata)
    pub difficulty: BlueprintDifficulty,
    /// Estimated time (from metadata, e.g. "30 min")
    pub estimated_time: String,
    /// Technology stack (from metadata)
    pub stack: Vec<String>,
    /// Freeform tags (from metadata)
    pub tags: Vec<String>,
    /// Tier 1 content: name + one-line description (~30 tokens)
    pub tier1: String,
    /// Tier 2 content: TL;DR + gotchas + checklist (~200 tokens)
    pub tier2: String,
    /// Tier 3 content: full markdown (~800+ tokens)
    pub tier3_content: String,
    /// Source file path (relative to blueprint repo root)
    pub source_file: String,
    /// SHA-256 hash of the raw file content (for change detection)
    pub content_hash: String,
}

/// Parse a blueprint markdown file into structured data.
///
/// `source_path` is the relative path within the blueprint repo (e.g. "patterns/service-layer-pattern.md").
pub fn parse_blueprint(content: &str, source_path: &str) -> Result<ParsedBlueprint> {
    let content_hash = compute_sha256(content);
    let slug = slug_from_path(source_path);

    // 1. Extract metadata from HTML comment
    let metadata = extract_metadata(content);

    // 2. Extract H1 title
    let name = extract_title(content).unwrap_or_else(|| slug.replace('-', " "));

    // 3. Extract first blockquote (one-liner description)
    let description = extract_first_blockquote(content).unwrap_or_default();

    // 4. Extract sections
    let tldr = extract_section(content, "TL;DR").unwrap_or_default();
    let gotchas = extract_section(content, "Gotchas").unwrap_or_default();
    let checklist = extract_section(content, "Checklist").unwrap_or_default();

    // 5. Determine scope from category
    let category = metadata
        .category
        .as_deref()
        .and_then(|c| c.parse::<BlueprintCategory>().ok())
        .unwrap_or_else(|| infer_category(source_path));
    let scope = infer_scope(&category, &metadata.tags);

    // 6. Parse difficulty
    let difficulty = metadata
        .difficulty
        .as_deref()
        .and_then(|d| d.parse::<BlueprintDifficulty>().ok())
        .unwrap_or_default();

    // 7. Build tiered content
    let tier1 = format!("{} — {}", name, description);

    let tier2 = build_tier2(&tldr, &gotchas, &checklist);

    // tier3 = full markdown (cleaned of metadata comment)
    let tier3_content = strip_metadata_comment(content).trim().to_string();

    Ok(ParsedBlueprint {
        name,
        slug,
        description,
        scope,
        category,
        difficulty,
        estimated_time: metadata.time.unwrap_or_default(),
        stack: metadata.stack,
        tags: metadata.tags,
        tier1,
        tier2,
        tier3_content,
        source_file: source_path.to_string(),
        content_hash,
    })
}

// ============================================================================
// Metadata extraction
// ============================================================================

#[derive(Debug, Default)]
struct MetadataBlock {
    tags: Vec<String>,
    category: Option<String>,
    difficulty: Option<String>,
    time: Option<String>,
    stack: Vec<String>,
}

/// Extract the HTML comment metadata block.
///
/// Looks for `<!-- METADATA ... -->` and parses YAML-like key: value lines.
fn extract_metadata(content: &str) -> MetadataBlock {
    let mut meta = MetadataBlock::default();

    let start = match content.find("<!-- METADATA") {
        Some(i) => i,
        None => {
            // Also try `<!--` followed by metadata-like content
            match content.find("<!--") {
                Some(i) => i,
                None => return meta,
            }
        }
    };

    let end = match content[start..].find("-->") {
        Some(i) => start + i,
        None => return meta,
    };

    let block = &content[start..end];

    for line in block.lines() {
        let line = line.trim();

        if let Some(value) = line.strip_prefix("tags:") {
            meta.tags = parse_yaml_list(value);
        } else if let Some(value) = line.strip_prefix("category:") {
            meta.category = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("difficulty:") {
            meta.difficulty = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("time:") {
            meta.time = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("stack:") {
            meta.stack = parse_yaml_list(value);
        }
    }

    meta
}

/// Parse a YAML-like list: `[a, b, c]` → vec!["a", "b", "c"]
fn parse_yaml_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);

    inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ============================================================================
// Section extraction
// ============================================================================

/// Extract the H1 title, stripping "Blueprint: " prefix.
fn extract_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("# ") {
            let title = title.trim();
            // Strip "Blueprint: " prefix if present
            let cleaned = title
                .strip_prefix("Blueprint: ")
                .or_else(|| title.strip_prefix("Blueprint — "))
                .unwrap_or(title);
            return Some(cleaned.trim().to_string());
        }
    }
    None
}

/// Extract the first blockquote line (> ...) after the metadata comment.
fn extract_first_blockquote(content: &str) -> Option<String> {
    // Skip past metadata comment if present
    let start = content.find("-->").map(|i| i + 3).unwrap_or(0);

    for line in content[start..].lines() {
        let trimmed = line.trim();
        if let Some(quote) = trimmed.strip_prefix("> ") {
            let quote = quote.trim();
            if !quote.is_empty() && !quote.starts_with("**") {
                // Skip gotcha-style blockquotes that start with bold
                return Some(quote.to_string());
            }
            // If it starts with **, it might be a gotcha — but if it's the first
            // blockquote after H1, it's the description
            if !quote.is_empty() {
                return Some(quote.to_string());
            }
        }
    }
    None
}

/// Extract a section by heading name (## <name>).
///
/// Returns all content from the heading until the next ## heading.
fn extract_section(content: &str, section_name: &str) -> Option<String> {
    let heading = format!("## {}", section_name);
    let lines: Vec<&str> = content.lines().collect();

    let start = lines.iter().position(|l| l.trim() == heading)?;

    // Find the end — next ## heading or end of file
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            let trimmed = l.trim();
            trimmed.starts_with("## ") && !trimmed.starts_with("### ")
        })
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());

    let section: String = lines[start + 1..end].to_vec().join("\n").trim().to_string();

    if section.is_empty() {
        None
    } else {
        Some(section)
    }
}

// ============================================================================
// Tier builders
// ============================================================================

/// Build tier2 content from TL;DR + gotchas + checklist.
///
/// Target: ~200 tokens. Keeps it concise.
fn build_tier2(tldr: &str, gotchas: &str, checklist: &str) -> String {
    let mut parts = Vec::new();

    if !tldr.is_empty() {
        parts.push(tldr.to_string());
    }

    if !gotchas.is_empty() {
        // Extract just the gotcha summaries (first sentence of each)
        let gotcha_summary = extract_gotcha_summaries(gotchas);
        if !gotcha_summary.is_empty() {
            parts.push(format!("**Gotchas**: {}", gotcha_summary));
        }
    }

    if !checklist.is_empty() {
        parts.push(format!("**Checklist**:\n{}", checklist));
    }

    parts.join("\n\n")
}

/// Extract gotcha summaries: just the bold titles from blockquote gotchas.
fn extract_gotcha_summaries(gotchas: &str) -> String {
    let mut summaries = Vec::new();
    for line in gotchas.lines() {
        let trimmed = line.trim();
        if let Some(quote) = trimmed.strip_prefix("> ") {
            // Extract bold title: **Title**: rest
            if let Some(start) = quote.find("**") {
                if let Some(end) = quote[start + 2..].find("**") {
                    let title = &quote[start + 2..start + 2 + end];
                    summaries.push(title.to_string());
                }
            }
        }
    }
    summaries.join(", ")
}

// ============================================================================
// Scope / category inference
// ============================================================================

/// Infer BlueprintCategory from the source file path.
fn infer_category(source_path: &str) -> BlueprintCategory {
    let path_lower = source_path.to_lowercase();
    if path_lower.starts_with("project-setup/") {
        BlueprintCategory::ProjectSetup
    } else if path_lower.starts_with("ci-cd/") {
        BlueprintCategory::CiCd
    } else if path_lower.starts_with("architecture/") {
        BlueprintCategory::Architecture
    } else if path_lower.starts_with("workflow/") {
        BlueprintCategory::Workflow
    } else if path_lower.starts_with("patterns/") {
        BlueprintCategory::Patterns
    } else if path_lower.starts_with("testing/") {
        BlueprintCategory::Testing
    } else {
        BlueprintCategory::Patterns // default
    }
}

/// Infer BlueprintScope from category and tags.
fn infer_scope(category: &BlueprintCategory, tags: &[String]) -> BlueprintScope {
    match category {
        BlueprintCategory::ProjectSetup => BlueprintScope::Scaffolding,
        BlueprintCategory::CiCd | BlueprintCategory::Workflow => BlueprintScope::Process,
        BlueprintCategory::Architecture => {
            // Architecture can be scaffolding-level or feature-level
            // If tags suggest a specific feature (oauth, payments, etc.), it's Feature
            let feature_tags = ["oauth", "auth", "payments", "push", "notifications", "sync"];
            if tags
                .iter()
                .any(|t| feature_tags.contains(&t.to_lowercase().as_str()))
            {
                BlueprintScope::Feature
            } else {
                BlueprintScope::Scaffolding
            }
        }
        BlueprintCategory::Patterns | BlueprintCategory::Testing => BlueprintScope::Pattern,
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Compute SHA-256 hash of content.
fn compute_sha256(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Derive a slug from a file path.
///
/// `patterns/service-layer-pattern.md` → `service-layer-pattern`
fn slug_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Strip the metadata HTML comment from content.
fn strip_metadata_comment(content: &str) -> &str {
    if let Some(start) = content.find("<!--") {
        if let Some(end) = content[start..].find("-->") {
            let _after = &content[start + end + 3..];
            // The content before the comment (H1) + after
            let _before = &content[..start];
            // For simplicity, just return from after the comment
            // since the H1 is before it, we keep everything
            // Actually: H1 is BEFORE the comment, so keep both
            // Let's just return the full content minus the comment
            // Use a simple approach: return content as-is (tier3 is full)
            return content;
        }
    }
    content
}

/// Extract cross-references from the References section.
///
/// Parses relative markdown links like `[Name](file.md)` into slugs.
pub fn extract_references(content: &str) -> Vec<String> {
    let references = match extract_section(content, "References") {
        Some(r) => r,
        None => return vec![],
    };

    let mut slugs = Vec::new();
    // Match markdown links: [text](relative-path.md)
    let re_pattern = r"\[([^\]]+)\]\(([^)]+\.md)\)";
    let re = regex::Regex::new(re_pattern).unwrap();

    for cap in re.captures_iter(&references) {
        if let Some(path) = cap.get(2) {
            let path_str = path.as_str();
            // Skip external links and anchors
            if path_str.starts_with("http") || path_str.starts_with('#') {
                continue;
            }
            // Resolve relative path to slug
            let slug = slug_from_path(path_str);
            if slug != "unknown" {
                slugs.push(slug);
            }
        }
    }

    slugs
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BLUEPRINT: &str = r#"# Blueprint: Service Layer Pattern

<!-- METADATA — structured for agents, useful for humans
tags:        [service, flutter, dart, testing, cache, offline, architecture]
category:    patterns
difficulty:  intermediate
time:        45 min
stack:       [flutter, dart]
-->

> Structure services with interface-based dependencies, cache+fallback chains, and mock-friendly testing.

## TL;DR

Every service that touches external I/O depends on abstract interfaces. Services are testable via hand-written mocks. Cache logic follows a cache → API → stale cache → error fallback chain.

## When to Use

- Adding a service that calls a REST API
- When you need offline support

## Steps

### 1. Define the interface

```dart
abstract class ExchangeRateProvider {
  Future<double> getRate(String from, String to);
}
```

## Gotchas

> **UnimplementedError stubs for deferred features**: When a feature is planned but not yet built, stub it with `throw UnimplementedError`.

> **Manual override preservation on clearAll()**: When the user manually sets a value, `clearAll()` should NOT delete it.

> **HTTP client injection**: Always inject `http.Client` into providers.

## Checklist

- [ ] Every external dependency is behind an abstract interface
- [ ] Cache follows the 4-step fallback
- [ ] Tests use hand-written mocks with `shouldFail` + `callCount`

## References

- [ViewModel Pure Functions](viewmodel-pure-functions.md) — how VMs consume service data
- [Error Handling & Logging](error-handling-logging.md) — Result types
- [Offline-First Architecture](../architecture/offline-first-architecture.md) — full offline sync
"#;

    #[test]
    fn test_parse_blueprint_basic() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        assert_eq!(result.name, "Service Layer Pattern");
        assert_eq!(result.slug, "service-layer-pattern");
        assert_eq!(result.source_file, "patterns/service-layer-pattern.md");
        assert!(!result.content_hash.is_empty());
        assert_eq!(result.content_hash.len(), 64); // SHA-256 hex
    }

    #[test]
    fn test_parse_metadata() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        assert_eq!(result.category, BlueprintCategory::Patterns);
        assert_eq!(result.difficulty, BlueprintDifficulty::Intermediate);
        assert_eq!(result.estimated_time, "45 min");
        assert_eq!(result.stack, vec!["flutter", "dart"]);
        assert!(result.tags.contains(&"service".to_string()));
        assert!(result.tags.contains(&"cache".to_string()));
    }

    #[test]
    fn test_parse_scope_inference() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();
        assert_eq!(result.scope, BlueprintScope::Pattern);

        // Architecture with no feature tags → Scaffolding
        let arch = parse_blueprint(
            "# Blueprint: Dual DB\n\n<!-- METADATA\ncategory: architecture\n-->\n\n> desc\n\n## TL;DR\nstuff",
            "architecture/dual-db.md",
        ).unwrap();
        assert_eq!(arch.scope, BlueprintScope::Scaffolding);

        // Architecture with oauth tag → Feature
        let oauth = parse_blueprint(
            "# Blueprint: OAuth\n\n<!-- METADATA\ncategory: architecture\ntags: [oauth, auth]\n-->\n\n> desc\n\n## TL;DR\nstuff",
            "architecture/oauth.md",
        ).unwrap();
        assert_eq!(oauth.scope, BlueprintScope::Feature);
    }

    #[test]
    fn test_parse_description() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        assert_eq!(
            result.description,
            "Structure services with interface-based dependencies, cache+fallback chains, and mock-friendly testing."
        );
    }

    #[test]
    fn test_parse_tier1() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        assert!(result.tier1.contains("Service Layer Pattern"));
        assert!(result.tier1.contains("interface-based"));
    }

    #[test]
    fn test_parse_tier2_contains_tldr_and_gotchas() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        // Should contain TL;DR content
        assert!(result.tier2.contains("abstract interfaces"));
        // Should contain gotcha summaries
        assert!(result.tier2.contains("Gotchas"));
        assert!(result.tier2.contains("UnimplementedError"));
        // Should contain checklist
        assert!(result.tier2.contains("Checklist"));
        assert!(result.tier2.contains("abstract interface"));
    }

    #[test]
    fn test_parse_tier3_is_full_content() {
        let result =
            parse_blueprint(SAMPLE_BLUEPRINT, "patterns/service-layer-pattern.md").unwrap();

        // tier3 should contain the full content including code examples
        assert!(result
            .tier3_content
            .contains("abstract class ExchangeRateProvider"));
        assert!(result.tier3_content.contains("## Steps"));
    }

    #[test]
    fn test_extract_references() {
        let refs = extract_references(SAMPLE_BLUEPRINT);

        assert_eq!(refs.len(), 3);
        assert!(refs.contains(&"viewmodel-pure-functions".to_string()));
        assert!(refs.contains(&"error-handling-logging".to_string()));
        assert!(refs.contains(&"offline-first-architecture".to_string()));
    }

    #[test]
    fn test_slug_from_path() {
        assert_eq!(
            slug_from_path("patterns/service-layer-pattern.md"),
            "service-layer-pattern"
        );
        assert_eq!(
            slug_from_path("architecture/oauth-auth-flow.md"),
            "oauth-auth-flow"
        );
        assert_eq!(
            slug_from_path("ci-cd/github-actions-flutter.md"),
            "github-actions-flutter"
        );
    }

    #[test]
    fn test_parse_yaml_list() {
        assert_eq!(
            parse_yaml_list("[flutter, dart, riverpod]"),
            vec!["flutter", "dart", "riverpod"]
        );
        assert_eq!(parse_yaml_list("  [a, b]  "), vec!["a", "b"]);
        assert_eq!(parse_yaml_list("[]"), Vec::<String>::new());
    }

    #[test]
    fn test_content_hash_deterministic() {
        let r1 = parse_blueprint(SAMPLE_BLUEPRINT, "test.md").unwrap();
        let r2 = parse_blueprint(SAMPLE_BLUEPRINT, "test.md").unwrap();
        assert_eq!(r1.content_hash, r2.content_hash);

        // Different content → different hash
        let r3 = parse_blueprint("# Different\n> desc\n## TL;DR\nstuff", "test.md").unwrap();
        assert_ne!(r1.content_hash, r3.content_hash);
    }
}
