//! Project-specific STT dictionaries (cc-rules) for Murmure
//!
//! Each project can have a custom correction dictionary that maps
//! commonly misheard technical terms to their correct spelling.
//! These are stored as TOML files in `resources/cc-rules/{slug}.toml`.
//!
//! Murmure loads the rules from a mounted directory. This module
//! manages the TOML files on the KnowLoop side (CRUD + auto-generation).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info};

/// A single correction rule: mishearing → correct term
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionRule {
    /// What the STT model typically outputs (e.g. "no loop")
    pub pattern: String,
    /// The correct text (e.g. "KnowLoop")
    pub replacement: String,
}

/// A project's full dictionary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDictionary {
    /// Project slug (used as filename)
    pub project_slug: String,
    /// Ordered map of corrections: pattern → replacement
    pub corrections: BTreeMap<String, String>,
}

/// Parsed TOML structure matching Murmure's cc-rules format
#[derive(Debug, Deserialize, Serialize)]
struct CcRulesToml {
    #[serde(default)]
    corrections: BTreeMap<String, String>,
}

/// Manager for project STT dictionaries
#[derive(Clone)]
pub struct DictionaryManager {
    /// Base directory for cc-rules (e.g. `resources/cc-rules/`)
    rules_dir: PathBuf,
}

impl DictionaryManager {
    /// Create a new DictionaryManager.
    ///
    /// `rules_dir` should point to the cc-rules directory
    /// (e.g. `resources/cc-rules/` or the Docker mount path).
    pub fn new(rules_dir: impl AsRef<Path>) -> Self {
        Self {
            rules_dir: rules_dir.as_ref().to_path_buf(),
        }
    }

    /// Ensure the rules directory exists
    async fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.rules_dir)
            .await
            .context("Failed to create cc-rules directory")?;
        Ok(())
    }

    /// Get the TOML file path for a given project slug
    fn toml_path(&self, slug: &str) -> PathBuf {
        self.rules_dir.join(format!("{slug}.toml"))
    }

    /// List all project dictionaries (by slug)
    pub async fn list(&self) -> Result<Vec<String>> {
        self.ensure_dir().await?;
        let mut slugs = Vec::new();
        let mut entries = fs::read_dir(&self.rules_dir)
            .await
            .context("Failed to read cc-rules directory")?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    slugs.push(stem.to_string());
                }
            }
        }
        slugs.sort();
        Ok(slugs)
    }

    /// Get a project's dictionary. Returns empty dict if file doesn't exist.
    pub async fn get(&self, project_slug: &str) -> Result<ProjectDictionary> {
        let path = self.toml_path(project_slug);
        let corrections = if path.exists() {
            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let parsed: CcRulesToml = toml::from_str(&content)
                .with_context(|| format!("Invalid TOML in {}", path.display()))?;
            parsed.corrections
        } else {
            BTreeMap::new()
        };

        Ok(ProjectDictionary {
            project_slug: project_slug.to_string(),
            corrections,
        })
    }

    /// Save a project's dictionary (overwrites existing)
    pub async fn save(&self, dict: &ProjectDictionary) -> Result<()> {
        self.ensure_dir().await?;
        let toml_struct = CcRulesToml {
            corrections: dict.corrections.clone(),
        };
        let content = toml::to_string_pretty(&toml_struct)
            .context("Failed to serialize dictionary to TOML")?;

        let header = format!(
            "# Murmure cc-rules for project: {}\n# Auto-managed by KnowLoop — edit via API or settings\n\n",
            dict.project_slug
        );

        let path = self.toml_path(&dict.project_slug);
        fs::write(&path, format!("{header}{content}"))
            .await
            .with_context(|| format!("Failed to write {}", path.display()))?;

        info!(
            project = %dict.project_slug,
            rules = dict.corrections.len(),
            "Saved STT dictionary"
        );
        Ok(())
    }

    /// Add or update a single correction rule
    pub async fn upsert_rule(
        &self,
        project_slug: &str,
        pattern: &str,
        replacement: &str,
    ) -> Result<ProjectDictionary> {
        let mut dict = self.get(project_slug).await?;
        dict.corrections
            .insert(pattern.to_string(), replacement.to_string());
        self.save(&dict).await?;
        debug!(project = %project_slug, pattern, replacement, "Upserted STT correction rule");
        Ok(dict)
    }

    /// Remove a correction rule by pattern
    pub async fn remove_rule(
        &self,
        project_slug: &str,
        pattern: &str,
    ) -> Result<ProjectDictionary> {
        let mut dict = self.get(project_slug).await?;
        dict.corrections.remove(pattern);
        self.save(&dict).await?;
        debug!(project = %project_slug, pattern, "Removed STT correction rule");
        Ok(dict)
    }

    /// Delete an entire project dictionary
    pub async fn delete(&self, project_slug: &str) -> Result<()> {
        let path = self.toml_path(project_slug);
        if path.exists() {
            fs::remove_file(&path)
                .await
                .with_context(|| format!("Failed to delete {}", path.display()))?;
            info!(project = %project_slug, "Deleted STT dictionary");
        }
        Ok(())
    }

    /// Auto-generate dictionary rules from a list of code symbols.
    ///
    /// Takes symbol names (function names, struct names, module names)
    /// and creates phonetic correction rules for common mishearings.
    pub fn generate_rules_from_symbols(symbols: &[String]) -> BTreeMap<String, String> {
        let mut rules = BTreeMap::new();

        for symbol in symbols {
            // Skip very short or generic names
            if symbol.len() < 4 {
                continue;
            }

            // Convert camelCase/PascalCase to space-separated lowercase for phonetic matching
            let phonetic = camel_to_spaces(symbol);
            if phonetic != symbol.to_lowercase() && !phonetic.is_empty() {
                rules.insert(phonetic, symbol.clone());
            }

            // Convert snake_case to space-separated
            if symbol.contains('_') {
                let spaced = symbol.replace('_', " ");
                if spaced != *symbol {
                    rules.insert(spaced.to_lowercase(), symbol.clone());
                }
            }
        }

        rules
    }

    /// Auto-generate and merge rules from code symbols into a project's dictionary
    pub async fn auto_generate(
        &self,
        project_slug: &str,
        symbols: &[String],
    ) -> Result<ProjectDictionary> {
        let mut dict = self.get(project_slug).await?;
        let generated = Self::generate_rules_from_symbols(symbols);

        let new_count = generated
            .iter()
            .filter(|(k, _)| !dict.corrections.contains_key(*k))
            .count();

        // Only add new rules, don't overwrite manually set ones
        for (pattern, replacement) in generated {
            dict.corrections.entry(pattern).or_insert(replacement);
        }

        self.save(&dict).await?;
        info!(
            project = %project_slug,
            new_rules = new_count,
            total_rules = dict.corrections.len(),
            "Auto-generated STT dictionary from code symbols"
        );
        Ok(dict)
    }
}

/// Convert CamelCase/PascalCase to space-separated lowercase
fn camel_to_spaces(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_was_upper = false;

    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 && !prev_was_upper {
                result.push(' ');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
            prev_was_upper = true;
        } else {
            prev_was_upper = false;
            result.push(ch);
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel_to_spaces() {
        assert_eq!(camel_to_spaces("KnowLoop"), "know loop");
        assert_eq!(camel_to_spaces("MurmureClient"), "murmure client");
        assert_eq!(camel_to_spaces("getNextTask"), "get next task");
        assert_eq!(camel_to_spaces("HTMLParser"), "htmlparser");
    }

    #[test]
    fn test_generate_rules_from_symbols() {
        let symbols = vec![
            "KnowLoop".to_string(),
            "MurmureClient".to_string(),
            "get_next_task".to_string(),
            "db".to_string(), // too short, should be skipped
        ];
        let rules = DictionaryManager::generate_rules_from_symbols(&symbols);

        assert_eq!(rules.get("know loop"), Some(&"KnowLoop".to_string()));
        assert_eq!(
            rules.get("murmure client"),
            Some(&"MurmureClient".to_string())
        );
        assert_eq!(
            rules.get("get next task"),
            Some(&"get_next_task".to_string())
        );
        assert!(!rules.contains_key("db"));
    }

    #[tokio::test]
    async fn test_dictionary_crud() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = DictionaryManager::new(dir.path());

        // Initially empty
        let dict = mgr.get("test-project").await.unwrap();
        assert!(dict.corrections.is_empty());

        // Add rules
        mgr.upsert_rule("test-project", "no loop", "KnowLoop")
            .await
            .unwrap();
        mgr.upsert_rule("test-project", "mur mure", "Murmure")
            .await
            .unwrap();

        let dict = mgr.get("test-project").await.unwrap();
        assert_eq!(dict.corrections.len(), 2);
        assert_eq!(
            dict.corrections.get("no loop"),
            Some(&"KnowLoop".to_string())
        );

        // Remove a rule
        mgr.remove_rule("test-project", "mur mure").await.unwrap();
        let dict = mgr.get("test-project").await.unwrap();
        assert_eq!(dict.corrections.len(), 1);

        // List
        let slugs = mgr.list().await.unwrap();
        assert_eq!(slugs, vec!["test-project"]);

        // Delete
        mgr.delete("test-project").await.unwrap();
        let slugs = mgr.list().await.unwrap();
        assert!(slugs.is_empty());
    }
}
