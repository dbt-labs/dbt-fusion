use std::sync::Arc;

use dbt_adapter::adapter_engine::AdapterEngine;
use dbt_adapter::sidecar_client::SidecarClient;

use crate::circuit_breaker::CircuitBreaker;
use crate::config::builder::ChaosBuilder;
use crate::config::predefined;
use crate::config::profile::ChaosProfile;
use crate::controller::ChaosController;
use crate::metrics::MetricsSnapshot;
use crate::proxy::engine_proxy::ChaosEngine;
use crate::proxy::sidecar_proxy::ChaosSidecarClient;

/// Convenience wrapper for chaos testing.
///
/// Holds a `ChaosController` and optional `CircuitBreaker`, and provides
/// factory methods for wrapping `SidecarClient` and `AdapterEngine` instances.
pub struct ChaosTestHarness {
    controller: Arc<ChaosController>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
}

impl ChaosTestHarness {
    /// Create from a builder.
    pub fn from_builder(builder: ChaosBuilder) -> Self {
        let (controller, circuit_breaker) = builder.build();
        Self {
            controller,
            circuit_breaker,
        }
    }

    /// Create from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, dbt_yaml::Error> {
        let profile = ChaosProfile::from_yaml(yaml)?;
        Ok(Self::from_builder(profile.into_builder()))
    }

    // -- Named constructors for predefined profiles --

    /// Create a network-flaky test harness.
    pub fn network_flaky() -> Self {
        Self::from_builder(predefined::network_flaky().with_seed(42))
    }

    /// Create a high-latency test harness.
    pub fn high_latency() -> Self {
        Self::from_builder(predefined::high_latency().with_seed(42))
    }

    /// Create an auth-failures test harness.
    pub fn auth_failures() -> Self {
        Self::from_builder(predefined::auth_failures().with_seed(42))
    }

    /// Create a cascade-failure test harness.
    pub fn cascade_failure() -> Self {
        Self::from_builder(predefined::cascade_failure().with_seed(42))
    }

    /// Create a metadata-chaos test harness.
    pub fn metadata_chaos() -> Self {
        Self::from_builder(predefined::metadata_chaos().with_seed(42))
    }

    /// Wrap a `SidecarClient` with chaos injection.
    pub fn wrap_sidecar_client(
        &self,
        client: Arc<dyn SidecarClient>,
    ) -> Arc<ChaosSidecarClient> {
        Arc::new(ChaosSidecarClient::new(
            client,
            self.controller.clone(),
            self.circuit_breaker.clone(),
        ))
    }

    /// Wrap an `AdapterEngine` with chaos injection.
    pub fn wrap_engine(&self, engine: Arc<dyn AdapterEngine>) -> Arc<ChaosEngine> {
        Arc::new(ChaosEngine::new(
            engine,
            self.controller.clone(),
            self.circuit_breaker.clone(),
        ))
    }

    /// Get a snapshot of the chaos metrics.
    pub fn metrics(&self) -> MetricsSnapshot {
        self.controller.metrics().snapshot()
    }

    /// Reset all counters and state.
    pub fn reset(&self) {
        self.controller.reset();
        if let Some(ref cb) = self.circuit_breaker {
            cb.reset();
        }
    }

    /// Enable or disable chaos injection at runtime.
    pub fn set_enabled(&self, enabled: bool) {
        self.controller.set_enabled(enabled);
    }

    /// Get a reference to the underlying controller.
    pub fn controller(&self) -> &Arc<ChaosController> {
        &self.controller
    }

    /// Get a reference to the circuit breaker, if configured.
    pub fn circuit_breaker(&self) -> Option<&Arc<CircuitBreaker>> {
        self.circuit_breaker.as_ref()
    }
}

// -- Assertion helpers --

/// Assert that at least one chaos fault was injected.
pub fn assert_chaos_occurred(harness: &ChaosTestHarness) {
    let snap = harness.metrics();
    assert!(
        snap.faults_injected > 0,
        "Expected at least one fault injection, but none occurred. \
         Total calls: {}, passthroughs: {}",
        snap.total_calls,
        snap.passthroughs,
    );
}

/// Assert that at least `min_count` error faults were injected.
pub fn assert_error_injections(harness: &ChaosTestHarness, min_count: u64) {
    let snap = harness.metrics();
    assert!(
        snap.error_faults >= min_count,
        "Expected at least {} error injections, but got {}. \
         Total calls: {}, faults: {}",
        min_count,
        snap.error_faults,
        snap.total_calls,
        snap.faults_injected,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::builder::ChaosBuilder;
    use crate::fault::*;

    #[test]
    fn test_harness_from_builder() {
        let harness = ChaosTestHarness::from_builder(
            ChaosBuilder::new()
                .with_error(
                    ChaosErrorKind::Driver,
                    "test",
                    1.0,
                    TargetOperation::All,
                )
                .with_seed(42),
        );

        // Trigger a fault through the controller directly
        let fault = harness.controller.maybe_inject(&TargetOperation::Execute);
        assert!(fault.is_some());

        assert_chaos_occurred(&harness);
    }

    #[test]
    fn test_harness_predefined_constructors() {
        let harness = ChaosTestHarness::network_flaky();
        assert!(harness.circuit_breaker.is_some());

        let harness = ChaosTestHarness::high_latency();
        assert!(harness.circuit_breaker.is_none());
    }

    #[test]
    fn test_harness_reset() {
        let harness = ChaosTestHarness::from_builder(
            ChaosBuilder::new()
                .with_error(
                    ChaosErrorKind::Driver,
                    "test",
                    1.0,
                    TargetOperation::All,
                )
                .with_seed(42),
        );

        harness.controller.maybe_inject(&TargetOperation::Execute);
        assert!(harness.metrics().faults_injected > 0);

        harness.reset();
        assert_eq!(harness.metrics().faults_injected, 0);
    }

    #[test]
    fn test_harness_enable_disable() {
        let harness = ChaosTestHarness::from_builder(
            ChaosBuilder::new()
                .with_error(
                    ChaosErrorKind::Driver,
                    "test",
                    1.0,
                    TargetOperation::All,
                )
                .with_seed(42),
        );

        harness.set_enabled(false);
        assert!(harness
            .controller
            .maybe_inject(&TargetOperation::Execute)
            .is_none());

        harness.set_enabled(true);
        assert!(harness
            .controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
    }

    #[test]
    fn test_harness_from_yaml() {
        let yaml = r#"
name: test
rules:
  - fault: !Error
      kind: Driver
      message: "yaml-configured error"
    probability: 1.0
    target: Execute
    after_calls: 0
    max_injections: 0
seed: 42
"#;

        let harness = ChaosTestHarness::from_yaml(yaml).unwrap();
        let fault = harness.controller.maybe_inject(&TargetOperation::Execute);
        assert!(fault.is_some());
    }
}
