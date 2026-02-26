use std::sync::Arc;

use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use crate::controller::ChaosController;
use crate::fault::*;

/// Fluent builder for constructing a `ChaosController` and optional `CircuitBreaker`.
pub struct ChaosBuilder {
    rules: Vec<FaultRule>,
    circuit_breaker_config: Option<CircuitBreakerConfig>,
    seed: Option<u64>,
    initially_disabled: bool,
}

impl ChaosBuilder {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            circuit_breaker_config: None,
            seed: None,
            initially_disabled: false,
        }
    }

    /// Add a latency fault rule.
    pub fn with_latency(
        mut self,
        min_ms: u64,
        max_ms: u64,
        probability: f64,
        target: TargetOperation,
    ) -> Self {
        self.rules.push(FaultRule::new(
            Fault::Latency(LatencyFault::new(min_ms, max_ms)),
            probability,
            target,
        ));
        self
    }

    /// Add an error fault rule.
    pub fn with_error(
        mut self,
        kind: ChaosErrorKind,
        message: impl Into<String>,
        probability: f64,
        target: TargetOperation,
    ) -> Self {
        self.rules.push(FaultRule::new(
            Fault::Error(ErrorFault::new(kind, message)),
            probability,
            target,
        ));
        self
    }

    /// Add a connection drop fault rule.
    pub fn with_connection_drop(
        mut self,
        message: impl Into<String>,
        probability: f64,
        target: TargetOperation,
    ) -> Self {
        self.rules.push(FaultRule::new(
            Fault::ConnectionDrop(ConnectionDropFault::new(message)),
            probability,
            target,
        ));
        self
    }

    /// Add a partial failure fault rule.
    pub fn with_partial_failure(
        mut self,
        fail_count: u32,
        error_kind: ChaosErrorKind,
        message: impl Into<String>,
        probability: f64,
        target: TargetOperation,
    ) -> Self {
        self.rules.push(FaultRule::new(
            Fault::PartialFailure(PartialFailureFault::new(fail_count, error_kind, message)),
            probability,
            target,
        ));
        self
    }

    /// Add a timeout fault rule.
    pub fn with_timeout(
        mut self,
        delay_ms: u64,
        message: impl Into<String>,
        probability: f64,
        target: TargetOperation,
    ) -> Self {
        self.rules.push(FaultRule::new(
            Fault::Timeout(TimeoutFault::new(delay_ms, message)),
            probability,
            target,
        ));
        self
    }

    /// Add a pre-built fault rule directly.
    pub fn with_rule(mut self, rule: FaultRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Enable a circuit breaker with the given config.
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker_config = Some(config);
        self
    }

    /// Set a deterministic seed for reproducible fault injection.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Start with chaos injection disabled (call `set_enabled(true)` to activate).
    pub fn initially_disabled(mut self) -> Self {
        self.initially_disabled = true;
        self
    }

    /// Build the `ChaosController` and optional `CircuitBreaker`.
    pub fn build(self) -> (Arc<ChaosController>, Option<Arc<CircuitBreaker>>) {
        let controller = Arc::new(ChaosController::new(self.rules, self.seed));
        if self.initially_disabled {
            controller.set_enabled(false);
        }

        let circuit_breaker = self
            .circuit_breaker_config
            .map(|config| Arc::new(CircuitBreaker::new(config)));

        (controller, circuit_breaker)
    }
}

impl Default for ChaosBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let (controller, cb) = ChaosBuilder::new()
            .with_error(
                ChaosErrorKind::Driver,
                "test error",
                1.0,
                TargetOperation::Execute,
            )
            .with_seed(42)
            .build();

        assert!(controller.is_enabled());
        assert!(cb.is_none());
    }

    #[test]
    fn test_builder_with_circuit_breaker() {
        let (_, cb) = ChaosBuilder::new()
            .with_circuit_breaker(CircuitBreakerConfig::default())
            .build();

        assert!(cb.is_some());
    }

    #[test]
    fn test_builder_initially_disabled() {
        let (controller, _) = ChaosBuilder::new()
            .with_error(
                ChaosErrorKind::Driver,
                "test",
                1.0,
                TargetOperation::All,
            )
            .initially_disabled()
            .with_seed(42)
            .build();

        assert!(!controller.is_enabled());
        // No faults when disabled
        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_none());
    }

    #[test]
    fn test_builder_fluent_chain() {
        let (controller, cb) = ChaosBuilder::new()
            .with_latency(50, 500, 0.3, TargetOperation::Execute)
            .with_error(
                ChaosErrorKind::Driver,
                "driver error",
                0.1,
                TargetOperation::Execute,
            )
            .with_connection_drop(
                "connection lost",
                0.05,
                TargetOperation::NewConnection,
            )
            .with_timeout(5000, "operation timed out", 0.02, TargetOperation::All)
            .with_circuit_breaker(CircuitBreakerConfig {
                failure_threshold: 5,
                recovery_timeout_secs: 30,
                success_threshold: 2,
            })
            .with_seed(12345)
            .build();

        assert!(controller.is_enabled());
        assert!(cb.is_some());
    }
}
