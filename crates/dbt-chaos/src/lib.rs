pub mod circuit_breaker;
pub mod config;
pub mod controller;
pub mod fault;
pub mod metrics;
pub mod proxy;
pub mod test_harness;

// Re-export key types for ergonomic usage
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use config::builder::ChaosBuilder;
pub use config::predefined;
pub use config::profile::ChaosProfile;
pub use controller::ChaosController;
pub use fault::{
    ChaosErrorKind, ConnectionDropFault, ErrorFault, Fault, FaultRule, LatencyFault,
    PartialFailureFault, TargetOperation, TimeoutFault,
};
pub use metrics::{ChaosMetrics, MetricsSnapshot};
pub use proxy::engine_proxy::ChaosEngine;
pub use proxy::sidecar_proxy::ChaosSidecarClient;
pub use test_harness::{ChaosTestHarness, assert_chaos_occurred, assert_error_injections};
