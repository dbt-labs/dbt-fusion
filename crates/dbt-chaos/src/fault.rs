use std::time::Duration;

use dbt_common::AdapterErrorKind;
use serde::{Deserialize, Serialize};

/// The types of faults that can be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Fault {
    Latency(LatencyFault),
    Error(ErrorFault),
    PartialFailure(PartialFailureFault),
    ConnectionDrop(ConnectionDropFault),
    Timeout(TimeoutFault),
}

/// Inject artificial latency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyFault {
    /// Minimum delay in milliseconds.
    pub min_ms: u64,
    /// Maximum delay in milliseconds.
    pub max_ms: u64,
}

impl LatencyFault {
    pub fn new(min_ms: u64, max_ms: u64) -> Self {
        assert!(min_ms <= max_ms, "min_ms must be <= max_ms");
        Self { min_ms, max_ms }
    }

    pub fn min_duration(&self) -> Duration {
        Duration::from_millis(self.min_ms)
    }

    pub fn max_duration(&self) -> Duration {
        Duration::from_millis(self.max_ms)
    }
}

/// Inject an adapter error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorFault {
    pub kind: ChaosErrorKind,
    pub message: String,
}

impl ErrorFault {
    pub fn new(kind: ChaosErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Inject a partial failure (succeed after N attempts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFailureFault {
    /// How many times to fail before succeeding.
    pub fail_count: u32,
    /// The error kind to use for failures.
    pub error_kind: ChaosErrorKind,
    pub message: String,
}

impl PartialFailureFault {
    pub fn new(fail_count: u32, error_kind: ChaosErrorKind, message: impl Into<String>) -> Self {
        Self {
            fail_count,
            error_kind,
            message: message.into(),
        }
    }
}

/// Simulate an abrupt connection drop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDropFault {
    pub message: String,
}

impl ConnectionDropFault {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Simulate an operation timeout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutFault {
    /// How long to wait before timing out, in milliseconds.
    pub delay_ms: u64,
    pub message: String,
}

impl TimeoutFault {
    pub fn new(delay_ms: u64, message: impl Into<String>) -> Self {
        Self {
            delay_ms,
            message: message.into(),
        }
    }
}

/// Error kinds that can be injected, mapping to `AdapterErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChaosErrorKind {
    Internal,
    SqlExecution,
    Configuration,
    Authentication,
    Driver,
    NotFound,
    Cancelled,
    Io,
}

impl ChaosErrorKind {
    pub fn to_adapter_error_kind(self) -> AdapterErrorKind {
        match self {
            ChaosErrorKind::Internal => AdapterErrorKind::Internal,
            ChaosErrorKind::SqlExecution => AdapterErrorKind::SqlExecution,
            ChaosErrorKind::Configuration => AdapterErrorKind::Configuration,
            ChaosErrorKind::Authentication => AdapterErrorKind::Authentication,
            ChaosErrorKind::Driver => AdapterErrorKind::Driver,
            ChaosErrorKind::NotFound => AdapterErrorKind::NotFound,
            ChaosErrorKind::Cancelled => AdapterErrorKind::Cancelled,
            ChaosErrorKind::Io => AdapterErrorKind::Io,
        }
    }
}

/// Which operations a fault rule targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetOperation {
    /// Apply to all operations.
    All,
    /// Apply only to execute calls.
    Execute,
    /// Apply only to new_connection calls.
    NewConnection,
    /// Apply only to shutdown calls.
    Shutdown,
    /// Apply only to metadata calls (get_relation_type, get_columns, list_relations).
    Metadata,
}

impl TargetOperation {
    pub fn matches(&self, operation: &TargetOperation) -> bool {
        matches!(self, TargetOperation::All) || self == operation
    }
}

/// A rule that defines when and how a fault should be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultRule {
    /// The fault to inject.
    pub fault: Fault,
    /// Probability of injection (0.0 to 1.0).
    pub probability: f64,
    /// Which operations this rule targets.
    pub target: TargetOperation,
    /// Only start injecting after this many calls to the target operation.
    #[serde(default)]
    pub after_calls: u64,
    /// Maximum number of times to inject this fault (0 = unlimited).
    #[serde(default)]
    pub max_injections: u64,
}

impl FaultRule {
    pub fn new(fault: Fault, probability: f64, target: TargetOperation) -> Self {
        assert!(
            (0.0..=1.0).contains(&probability),
            "probability must be between 0.0 and 1.0"
        );
        Self {
            fault,
            probability,
            target,
            after_calls: 0,
            max_injections: 0,
        }
    }

    pub fn after_calls(mut self, n: u64) -> Self {
        self.after_calls = n;
        self
    }

    pub fn max_injections(mut self, n: u64) -> Self {
        self.max_injections = n;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chaos_error_kind_mapping() {
        assert_eq!(
            ChaosErrorKind::Driver.to_adapter_error_kind(),
            AdapterErrorKind::Driver
        );
        assert_eq!(
            ChaosErrorKind::Authentication.to_adapter_error_kind(),
            AdapterErrorKind::Authentication
        );
    }

    #[test]
    fn test_target_operation_matches() {
        assert!(TargetOperation::All.matches(&TargetOperation::Execute));
        assert!(TargetOperation::All.matches(&TargetOperation::NewConnection));
        assert!(TargetOperation::Execute.matches(&TargetOperation::Execute));
        assert!(!TargetOperation::Execute.matches(&TargetOperation::NewConnection));
    }

    #[test]
    fn test_fault_rule_builder() {
        let rule = FaultRule::new(
            Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "test")),
            0.5,
            TargetOperation::Execute,
        )
        .after_calls(10)
        .max_injections(5);

        assert_eq!(rule.after_calls, 10);
        assert_eq!(rule.max_injections, 5);
        assert!((rule.probability - 0.5).abs() < f64::EPSILON);
    }
}
