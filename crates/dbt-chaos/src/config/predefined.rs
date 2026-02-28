use crate::circuit_breaker::CircuitBreakerConfig;
use crate::config::builder::ChaosBuilder;
use crate::fault::*;

/// Network-flaky profile: intermittent latency, errors, and connection drops.
///
/// - 30% latency (50-500ms) on execute
/// - 10% driver errors on execute
/// - 5% connection drops on new_connection
/// - Circuit breaker enabled
pub fn network_flaky() -> ChaosBuilder {
    ChaosBuilder::new()
        .with_latency(50, 500, 0.3, TargetOperation::Execute)
        .with_error(
            ChaosErrorKind::Driver,
            "chaos: simulated network error",
            0.1,
            TargetOperation::Execute,
        )
        .with_connection_drop(
            "chaos: simulated connection drop",
            0.05,
            TargetOperation::NewConnection,
        )
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_threshold: 5,
            recovery_timeout_secs: 30,
            success_threshold: 2,
        })
}

/// High-latency profile: simulates a slow network.
///
/// - 80% latency (200-2000ms) on execute
/// - 50% latency (100-1000ms) on metadata
pub fn high_latency() -> ChaosBuilder {
    ChaosBuilder::new()
        .with_latency(200, 2000, 0.8, TargetOperation::Execute)
        .with_latency(100, 1000, 0.5, TargetOperation::Metadata)
}

/// Auth failures profile: simulates authentication issues.
///
/// - 30% auth errors on new_connection
/// - 15% auth errors on execute
pub fn auth_failures() -> ChaosBuilder {
    ChaosBuilder::new()
        .with_error(
            ChaosErrorKind::Authentication,
            "chaos: simulated authentication failure",
            0.3,
            TargetOperation::NewConnection,
        )
        .with_error(
            ChaosErrorKind::Authentication,
            "chaos: simulated session expired",
            0.15,
            TargetOperation::Execute,
        )
}

/// Cascade failure profile: aggressive failures to test circuit breaker behavior.
///
/// - 50% driver errors on all operations
/// - Aggressive circuit breaker (threshold=3, timeout=5s)
pub fn cascade_failure() -> ChaosBuilder {
    ChaosBuilder::new()
        .with_error(
            ChaosErrorKind::Driver,
            "chaos: simulated cascade failure",
            0.5,
            TargetOperation::All,
        )
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout_secs: 5,
            success_threshold: 2,
        })
}

/// Metadata chaos profile: targets metadata operations.
///
/// - 20% not-found errors on metadata
/// - 40% latency (50-500ms) on metadata
pub fn metadata_chaos() -> ChaosBuilder {
    ChaosBuilder::new()
        .with_error(
            ChaosErrorKind::NotFound,
            "chaos: simulated relation not found",
            0.2,
            TargetOperation::Metadata,
        )
        .with_latency(50, 500, 0.4, TargetOperation::Metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_flaky_builds() {
        let (controller, cb) = network_flaky().with_seed(42).build();
        assert!(controller.is_enabled());
        assert!(cb.is_some());
    }

    #[test]
    fn test_high_latency_builds() {
        let (controller, cb) = high_latency().with_seed(42).build();
        assert!(controller.is_enabled());
        assert!(cb.is_none());
    }

    #[test]
    fn test_auth_failures_builds() {
        let (controller, cb) = auth_failures().with_seed(42).build();
        assert!(controller.is_enabled());
        assert!(cb.is_none());
    }

    #[test]
    fn test_cascade_failure_builds() {
        let (controller, cb) = cascade_failure().with_seed(42).build();
        assert!(controller.is_enabled());
        assert!(cb.is_some());
    }

    #[test]
    fn test_metadata_chaos_builds() {
        let (controller, cb) = metadata_chaos().with_seed(42).build();
        assert!(controller.is_enabled());
        assert!(cb.is_none());
    }
}
