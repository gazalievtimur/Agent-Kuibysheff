use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::billing::Money;

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    pub max_iterations: u32,
    pub max_tokens: u64,
    pub max_duration_sec: u64,
    #[serde(default)]
    pub max_cost: Option<Money>,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct RunMetrics {
    started_at: Instant,
    iterations: u32,
    token_usage: TokenUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitExceeded {
    Iterations,
    Tokens,
    Duration,
}

impl Default for RunMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl RunMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            iterations: 0,
            token_usage: TokenUsage::default(),
        }
    }

    /// Checks iteration, token, and duration limits before the next agent step.
    ///
    /// # Errors
    ///
    /// Returns [`LimitExceeded`] when any configured limit is already reached.
    pub fn pre_step_check(&self, limits: &LimitsConfig) -> Result<(), LimitExceeded> {
        if self.iterations >= limits.max_iterations {
            return Err(LimitExceeded::Iterations);
        }
        if self.tokens_limit_hit(limits) {
            return Err(LimitExceeded::Tokens);
        }
        if self.duration_limit_hit(limits) {
            return Err(LimitExceeded::Duration);
        }
        Ok(())
    }

    pub fn begin_iteration(&mut self) {
        self.iterations = self.iterations.saturating_add(1);
    }

    pub fn add_tokens(&mut self, usage: TokenUsage) {
        self.token_usage.prompt_tokens = self
            .token_usage
            .prompt_tokens
            .saturating_add(usage.prompt_tokens);
        self.token_usage.completion_tokens = self
            .token_usage
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        self.token_usage.total_tokens = self
            .token_usage
            .total_tokens
            .saturating_add(usage.total_tokens);
    }

    #[must_use]
    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    #[must_use]
    pub fn tokens(&self) -> TokenUsage {
        self.token_usage
    }

    #[must_use]
    pub fn elapsed_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }

    #[must_use]
    pub fn tokens_limit_hit(&self, limits: &LimitsConfig) -> bool {
        self.token_usage.total_tokens >= limits.max_tokens
    }

    #[must_use]
    pub fn duration_limit_hit(&self, limits: &LimitsConfig) -> bool {
        self.started_at.elapsed() >= Duration::from_secs(limits.max_duration_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iteration_limit_is_enforced() {
        let limits = LimitsConfig {
            max_iterations: 1,
            max_tokens: 1000,
            max_duration_sec: 100,
            max_cost: None,
        };
        let mut metrics = RunMetrics::new();
        assert!(metrics.pre_step_check(&limits).is_ok());
        metrics.begin_iteration();
        assert_eq!(
            metrics.pre_step_check(&limits),
            Err(LimitExceeded::Iterations)
        );
    }

    #[test]
    fn token_limit_is_enforced() {
        let limits = LimitsConfig {
            max_iterations: 100,
            max_tokens: 10,
            max_duration_sec: 100,
            max_cost: None,
        };
        let mut metrics = RunMetrics::new();
        metrics.add_tokens(TokenUsage {
            prompt_tokens: 5,
            completion_tokens: 5,
            total_tokens: 10,
        });
        assert_eq!(metrics.pre_step_check(&limits), Err(LimitExceeded::Tokens));
    }
}
