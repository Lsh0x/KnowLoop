//! Proxy models for fast reward estimation during MCTS simulation.
//!
//! V1: GDS heuristic — reward = f(action_type_prior, remaining_steps, source_reward)
//! V2 (future): Small MLP trained on real trajectories.

/// Trait for proxy reward estimation used in MCTS simulation.
///
/// A proxy model must be fast (<100μs per call) since it's invoked
/// hundreds of times during a single MCTS rollout.
pub trait ProxyModel: Send + Sync {
    /// Estimate the reward of taking a specific action at a decision point.
    ///
    /// # Arguments
    /// * `action_type` — MCP tool.action (e.g., "code.search")
    /// * `action_params` — serialized action parameters
    /// * `remaining_steps` — how many decision points remain in the trajectory
    /// * `source_reward` — the total reward of the source trajectory (baseline)
    fn estimate_reward(
        &self,
        action_type: &str,
        action_params: &serde_json::Value,
        remaining_steps: usize,
        source_reward: f64,
    ) -> f64;
}

// ---------------------------------------------------------------------------
// V1: GDS Heuristic Proxy
// ---------------------------------------------------------------------------

/// Heuristic-based proxy using action type priors derived from GDS analysis.
///
/// Each action type has an estimated "information gain" based on typical
/// graph analysis patterns:
/// - Exploration actions (search, find_references) → higher gain early
/// - Context actions (get_context, get_propagated) → higher gain mid-trajectory
/// - Impact actions (analyze_impact, get_health) → higher gain late
///
/// This is a rough approximation — enough for MCTS exploration.
#[derive(Debug, Clone)]
pub struct GdsHeuristicProxy {
    /// Noise factor (0.0 = deterministic, 1.0 = max randomness).
    pub noise: f64,
}

impl Default for GdsHeuristicProxy {
    fn default() -> Self {
        Self { noise: 0.1 }
    }
}

impl GdsHeuristicProxy {
    /// Get the prior reward weight for an action type.
    ///
    /// Higher values = actions typically associated with higher trajectory rewards.
    fn action_prior(action_type: &str) -> f64 {
        match action_type {
            // High-value exploration actions
            "code.search" | "code.search_project" => 0.7,
            "code.find_references" => 0.65,
            "code.get_call_graph" => 0.6,

            // High-value context actions
            "note.get_context" | "note.get_propagated" => 0.75,
            "note.search_semantic" => 0.7,
            "decision.search_semantic" => 0.65,

            // High-value analysis actions
            "code.analyze_impact" => 0.8,
            "code.get_health" => 0.55,
            "code.get_architecture" => 0.5,

            // Skill/protocol actions
            "skill.activate" => 0.6,
            "protocol.route" => 0.55,

            // Low-level actions
            "code.get_file_symbols" => 0.45,
            "code.get_file_dependencies" => 0.5,

            // Default for unknown actions
            _ => 0.4,
        }
    }

    /// Phase multiplier based on where we are in the trajectory.
    ///
    /// Returns a multiplier in [0.5, 1.5]:
    /// - Early phase (>50% remaining): exploration gets bonus
    /// - Mid phase (25-50% remaining): context gets bonus
    /// - Late phase (<25% remaining): impact analysis gets bonus
    fn phase_multiplier(action_type: &str, remaining_ratio: f64) -> f64 {
        let is_exploration = action_type.contains("search")
            || action_type.contains("find_references")
            || action_type.contains("get_architecture");

        let is_context = action_type.contains("get_context")
            || action_type.contains("get_propagated")
            || action_type.contains("activate");

        let is_impact = action_type.contains("analyze_impact")
            || action_type.contains("get_health")
            || action_type.contains("get_call_graph");

        if remaining_ratio > 0.5 {
            // Early phase: exploration bonus
            if is_exploration {
                1.3
            } else if is_context {
                1.0
            } else {
                0.8
            }
        } else if remaining_ratio > 0.25 {
            // Mid phase: context bonus
            if is_context {
                1.3
            } else if is_exploration {
                1.0
            } else if is_impact {
                1.1
            } else {
                0.9
            }
        } else {
            // Late phase: impact bonus
            if is_impact {
                1.4
            } else if is_context {
                1.1
            } else {
                0.7
            }
        }
    }
}

impl ProxyModel for GdsHeuristicProxy {
    fn estimate_reward(
        &self,
        action_type: &str,
        _action_params: &serde_json::Value,
        remaining_steps: usize,
        source_reward: f64,
    ) -> f64 {
        let prior = Self::action_prior(action_type);
        let total_steps = remaining_steps + 1; // +1 for current step
        let remaining_ratio = remaining_steps as f64 / total_steps as f64;
        let phase_mult = Self::phase_multiplier(action_type, remaining_ratio);

        // Base reward: source reward scaled by action prior and phase
        let base = source_reward * prior * phase_mult;

        // Add controlled noise for exploration diversity
        let noise_val = if self.noise > 0.0 {
            // Deterministic noise based on action type hash (reproducible)
            let hash = action_type
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            let pseudo_random = ((hash % 1000) as f64 / 1000.0) - 0.5; // [-0.5, 0.5]
            pseudo_random * self.noise * source_reward
        } else {
            0.0
        };

        (base + noise_val).max(0.0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gds_proxy_rewards_are_positive() {
        let proxy = GdsHeuristicProxy::default();
        let actions = [
            "code.search",
            "note.get_context",
            "code.analyze_impact",
            "skill.activate",
            "unknown.action",
        ];

        for action in &actions {
            let reward = proxy.estimate_reward(action, &serde_json::Value::Null, 5, 1.0);
            assert!(
                reward >= 0.0,
                "Reward for {} should be non-negative, got {}",
                action,
                reward
            );
            assert!(reward.is_finite(), "Reward for {} should be finite", action);
        }
    }

    #[test]
    fn test_phase_affects_reward() {
        let proxy = GdsHeuristicProxy { noise: 0.0 };

        // Early phase: exploration should score higher
        let early_search = proxy.estimate_reward("code.search", &serde_json::Value::Null, 8, 1.0);
        let early_impact =
            proxy.estimate_reward("code.analyze_impact", &serde_json::Value::Null, 8, 1.0);

        // Late phase: impact should score higher
        let late_search = proxy.estimate_reward("code.search", &serde_json::Value::Null, 1, 1.0);
        let late_impact =
            proxy.estimate_reward("code.analyze_impact", &serde_json::Value::Null, 1, 1.0);

        // Early: search should be relatively higher vs impact
        // Late: impact should be relatively higher vs search
        assert!(
            early_search / early_impact > late_search / late_impact,
            "Search should be relatively more valuable early than late"
        );
    }

    #[test]
    fn test_source_reward_scales_output() {
        let proxy = GdsHeuristicProxy { noise: 0.0 };

        let low = proxy.estimate_reward("code.search", &serde_json::Value::Null, 5, 0.5);
        let high = proxy.estimate_reward("code.search", &serde_json::Value::Null, 5, 1.0);

        assert!(
            high > low,
            "Higher source reward should produce higher estimate"
        );
        assert!(
            (high / low - 2.0).abs() < 0.1,
            "Should scale roughly linearly"
        );
    }

    #[test]
    fn test_deterministic_noise() {
        let proxy = GdsHeuristicProxy { noise: 0.1 };

        let r1 = proxy.estimate_reward("code.search", &serde_json::Value::Null, 5, 1.0);
        let r2 = proxy.estimate_reward("code.search", &serde_json::Value::Null, 5, 1.0);

        assert!(
            (r1 - r2).abs() < 1e-10,
            "Noise should be deterministic for same inputs"
        );
    }

    #[test]
    fn test_correlation_with_real_rewards() {
        // Simulate a basic correlation test:
        // Actions known to be high-value should get higher rewards
        let proxy = GdsHeuristicProxy { noise: 0.0 };

        let high_value =
            proxy.estimate_reward("code.analyze_impact", &serde_json::Value::Null, 2, 1.0);
        let low_value =
            proxy.estimate_reward("code.get_file_symbols", &serde_json::Value::Null, 2, 1.0);

        assert!(
            high_value > low_value,
            "analyze_impact ({}) should score higher than get_file_symbols ({})",
            high_value,
            low_value
        );
    }
}
