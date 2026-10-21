use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for metaplasticity dynamics and environmental threshold modulation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaplasticityConfig {
    /// Maximum absolute drop in bypass importance threshold at peak volatility (default 0.1875).
    /// For example, at base 0.90 and volatility 0.8: threshold drops by 0.8 * 0.1875 = 0.15 -> 0.75.
    pub max_bypass_drop: f32,
    /// Absolute lower bound for adapted bypass threshold (default 0.50).
    pub min_bypass_threshold: f32,
    /// Reduction factor for base decay stability S_0 at peak volatility (default 0.50).
    /// For example, at base S_0 = 24.0 and volatility 0.8: S_0 = 24.0 * (1.0 - 0.8 * 0.50) = 14.4h.
    pub stability_reduction_factor: f32,
    /// Absolute lower bound for adapted decay stability in hours (default 0.001).
    pub min_decay_stability: f32,
    /// Maximum boost to prune threshold at peak volatility (default 0.05).
    /// For example, at base prune 0.05 and volatility 0.8: prune = 0.05 + 0.8 * 0.05 = 0.09.
    pub max_prune_boost: f32,
    /// Absolute upper bound for adapted prune threshold (default 0.50).
    pub max_prune_threshold: f32,
    /// Half-life in seconds for volatility decaying back to homeostasis (default 3600.0s = 1 hour).
    pub homeostasis_half_life_seconds: f32,
}

impl Default for MetaplasticityConfig {
    fn default() -> Self {
        Self {
            max_bypass_drop: 0.1875,
            min_bypass_threshold: 0.50,
            stability_reduction_factor: 0.50,
            min_decay_stability: 0.001,
            max_prune_boost: 0.05,
            max_prune_threshold: 0.50,
            homeostasis_half_life_seconds: 3600.0,
        }
    }
}

impl MetaplasticityConfig {
    /// Calculate adapted decay stability S_0 for given base and volatility using default config.
    pub fn calculate_adapted_decay_stability(base_s0: f32, volatility: f32) -> f32 {
        Self::default().adapted_decay_stability(base_s0, volatility)
    }

    /// Calculate adapted subconscious bypass threshold for given base and volatility using default config.
    pub fn calculate_adapted_bypass_threshold(base_threshold: f32, volatility: f32) -> f32 {
        Self::default().adapted_bypass_threshold(base_threshold, volatility)
    }

    /// Calculate adapted prune threshold for given base and volatility using default config.
    pub fn calculate_adapted_prune_threshold(base_prune: f32, volatility: f32) -> f32 {
        Self::default().adapted_prune_threshold(base_prune, volatility)
    }

    /// Calculate adapted decay stability S_0 according to this config.
    pub fn adapted_decay_stability(&self, base_s0: f32, volatility: f32) -> f32 {
        let v = volatility.clamp(0.0, 1.0);
        let factor = (1.0 - v * self.stability_reduction_factor).max(0.0);
        (base_s0 * factor).max(self.min_decay_stability)
    }

    /// Calculate adapted subconscious bypass threshold according to this config.
    pub fn adapted_bypass_threshold(&self, base_threshold: f32, volatility: f32) -> f32 {
        let v = volatility.clamp(0.0, 1.0);
        let drop = v * self.max_bypass_drop;
        let candidate = base_threshold - drop;
        let floor = self.min_bypass_threshold;
        let ceiling = base_threshold.max(floor);
        candidate.clamp(floor, ceiling)
    }

    /// Calculate adapted prune threshold according to this config.
    pub fn adapted_prune_threshold(&self, base_prune: f32, volatility: f32) -> f32 {
        let v = volatility.clamp(0.0, 1.0);
        let boost = v * self.max_prune_boost;
        let candidate = base_prune + boost;
        let ceiling = self.max_prune_threshold.max(base_prune);
        candidate.clamp(base_prune, ceiling)
    }
}

/// Calculate adapted decay stability S_0 given base and volatility.
pub fn calculate_adapted_decay_stability(base_s0: f32, volatility: f32) -> f32 {
    MetaplasticityConfig::calculate_adapted_decay_stability(base_s0, volatility)
}

/// Calculate adapted bypass threshold given base and volatility.
pub fn calculate_adapted_bypass_threshold(base_threshold: f32, volatility: f32) -> f32 {
    MetaplasticityConfig::calculate_adapted_bypass_threshold(base_threshold, volatility)
}

/// Calculate adapted prune threshold given base and volatility.
pub fn calculate_adapted_prune_threshold(base_prune: f32, volatility: f32) -> f32 {
    MetaplasticityConfig::calculate_adapted_prune_threshold(base_prune, volatility)
}

/// Runtime state tracking environmental volatility and modulating plasticity thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaplasticityState {
    /// Environmental volatility in range [0.0, 1.0].
    pub volatility: f32,
    /// Metaplasticity configuration parameters.
    pub config: MetaplasticityConfig,
    /// Timestamp of last volatility update.
    pub last_updated_at: DateTime<Utc>,
    /// Accumulated count of code diffs / JTMS invalidations during the active window.
    pub environmental_event_count: usize,
}

impl Default for MetaplasticityState {
    fn default() -> Self {
        Self {
            volatility: 0.0,
            config: MetaplasticityConfig::default(),
            last_updated_at: Utc::now(),
            environmental_event_count: 0,
        }
    }
}

impl MetaplasticityState {
    pub fn new(config: MetaplasticityConfig) -> Self {
        Self {
            volatility: 0.0,
            config,
            last_updated_at: Utc::now(),
            environmental_event_count: 0,
        }
    }

    pub fn with_volatility(volatility: f32) -> Self {
        Self {
            volatility: volatility.clamp(0.0, 1.0),
            config: MetaplasticityConfig::default(),
            last_updated_at: Utc::now(),
            environmental_event_count: 0,
        }
    }

    /// Calculate adapted decay stability S_0 for given base and volatility.
    pub fn calculate_adapted_decay_stability(base_s0: f32, volatility: f32) -> f32 {
        MetaplasticityConfig::calculate_adapted_decay_stability(base_s0, volatility)
    }

    /// Calculate adapted bypass threshold for given base and volatility.
    pub fn calculate_adapted_bypass_threshold(base_threshold: f32, volatility: f32) -> f32 {
        MetaplasticityConfig::calculate_adapted_bypass_threshold(base_threshold, volatility)
    }

    /// Calculate adapted prune threshold for given base and volatility.
    pub fn calculate_adapted_prune_threshold(base_prune: f32, volatility: f32) -> f32 {
        MetaplasticityConfig::calculate_adapted_prune_threshold(base_prune, volatility)
    }

    /// Convenience method: adapted decay stability using current state volatility.
    pub fn adapted_decay_stability(&self, base_s0: f32) -> f32 {
        self.config
            .adapted_decay_stability(base_s0, self.volatility)
    }

    /// Convenience method: adapted bypass threshold using current state volatility.
    pub fn adapted_bypass_threshold(&self, base_threshold: f32) -> f32 {
        self.config
            .adapted_bypass_threshold(base_threshold, self.volatility)
    }

    /// Convenience method: adapted prune threshold using current state volatility.
    pub fn adapted_prune_threshold(&self, base_prune: f32) -> f32 {
        self.config
            .adapted_prune_threshold(base_prune, self.volatility)
    }

    /// Record environmental turbulence (e.g. AST diffs, JTMS belief invalidations).
    pub fn record_environmental_turbulence(&mut self, intensity: f32) {
        let delta = intensity.clamp(0.0, 1.0) * 0.25;
        self.volatility = (self.volatility + delta).clamp(0.0, 1.0);
        self.environmental_event_count += 1;
        self.last_updated_at = Utc::now();
    }

    /// Decay volatility towards baseline homeostasis (0.0) based on elapsed seconds.
    pub fn decay_towards_homeostasis(&mut self, elapsed_seconds: f32) {
        if elapsed_seconds <= 0.0 || self.volatility <= 0.0 {
            return;
        }
        let half_life = self.config.homeostasis_half_life_seconds.max(1.0);
        let decay_factor = (-elapsed_seconds * (2.0f32.ln()) / half_life).exp();
        self.volatility = (self.volatility * decay_factor).clamp(0.0, 1.0);
        if self.volatility < 1e-3 {
            self.volatility = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metaplasticity_defaults_and_clamping() {
        let state = MetaplasticityState::with_volatility(1.5);
        assert_eq!(state.volatility, 1.0);

        let state_neg = MetaplasticityState::with_volatility(-0.5);
        assert_eq!(state_neg.volatility, 0.0);
    }

    #[test]
    fn test_metaplasticity_exact_adaptation_values() {
        let base_s0 = 24.0;
        let base_bypass = 0.90;
        let base_prune = 0.05;

        // Baseline (volatility = 0.0)
        assert_eq!(calculate_adapted_decay_stability(base_s0, 0.0), 24.0);
        assert_eq!(calculate_adapted_bypass_threshold(base_bypass, 0.0), 0.90);
        assert_eq!(calculate_adapted_prune_threshold(base_prune, 0.0), 0.05);

        // High volatility (volatility = 0.8)
        let s0_adapted = calculate_adapted_decay_stability(base_s0, 0.8);
        let bypass_adapted = calculate_adapted_bypass_threshold(base_bypass, 0.8);
        let prune_adapted = calculate_adapted_prune_threshold(base_prune, 0.8);

        // 24.0 * (1.0 - 0.8 * 0.5) = 24.0 * 0.6 = 14.4
        assert!((s0_adapted - 14.4).abs() < 1e-5);
        // 0.90 - 0.8 * 0.1875 = 0.90 - 0.15 = 0.75
        assert!((bypass_adapted - 0.75).abs() < 1e-5);
        // 0.05 + 0.8 * 0.05 = 0.09
        assert!((prune_adapted - 0.09).abs() < 1e-5);
    }

    #[test]
    fn test_homeostasis_relaxation() {
        let mut state = MetaplasticityState::with_volatility(0.8);
        assert_eq!(state.volatility, 0.8);

        // Relax for 1 half-life (3600 seconds)
        state.decay_towards_homeostasis(3600.0);
        assert!((state.volatility - 0.4).abs() < 0.01);

        // Relax for another 10 half-lives -> reaches 0.0 homeostasis
        state.decay_towards_homeostasis(36000.0);
        assert_eq!(state.volatility, 0.0);
    }
}
