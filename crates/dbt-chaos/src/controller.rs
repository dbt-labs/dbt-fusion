use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use dbt_common::{AdapterError, AdapterErrorKind};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::fault::{
    ConnectionDropFault, ErrorFault, Fault, FaultRule, TargetOperation, TimeoutFault,
};
use crate::metrics::ChaosMetrics;

struct ControllerState {
    /// Per-operation call counters.
    call_counts: CallCounts,
    /// Per-rule injection counts.
    rule_injection_counts: Vec<u64>,
    /// Seeded RNG for deterministic fault injection.
    rng: StdRng,
    /// Per-rule partial failure counters (tracks consecutive failures).
    partial_failure_counts: Vec<u32>,
}

#[derive(Default)]
struct CallCounts {
    execute: u64,
    new_connection: u64,
    shutdown: u64,
    metadata: u64,
    all: u64,
}

impl CallCounts {
    fn get(&self, op: &TargetOperation) -> u64 {
        match op {
            TargetOperation::Execute => self.execute,
            TargetOperation::NewConnection => self.new_connection,
            TargetOperation::Shutdown => self.shutdown,
            TargetOperation::Metadata => self.metadata,
            TargetOperation::All => self.all,
        }
    }

    fn increment(&mut self, op: &TargetOperation) {
        match op {
            TargetOperation::Execute => self.execute += 1,
            TargetOperation::NewConnection => self.new_connection += 1,
            TargetOperation::Shutdown => self.shutdown += 1,
            TargetOperation::Metadata => self.metadata += 1,
            TargetOperation::All => {} // All is only used for rule matching, not as an operation
        }
        self.all += 1;
    }
}

/// The core fault-decision engine shared between proxy types.
pub struct ChaosController {
    rules: Vec<FaultRule>,
    state: Mutex<ControllerState>,
    metrics: ChaosMetrics,
    enabled: AtomicBool,
}

impl ChaosController {
    pub fn new(rules: Vec<FaultRule>, seed: Option<u64>) -> Self {
        let rule_count = rules.len();
        let rng = match seed {
            Some(s) => StdRng::seed_from_u64(s),
            None => StdRng::from_os_rng(),
        };
        Self {
            rules,
            state: Mutex::new(ControllerState {
                call_counts: CallCounts::default(),
                rule_injection_counts: vec![0; rule_count],
                rng,
                partial_failure_counts: vec![0; rule_count],
            }),
            metrics: ChaosMetrics::new(),
            enabled: AtomicBool::new(true),
        }
    }

    /// Evaluate rules and decide whether to inject a fault for the given operation.
    pub fn maybe_inject(&self, operation: &TargetOperation) -> Option<Fault> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }

        let mut state = self.state.lock().unwrap();
        state.call_counts.increment(operation);
        self.metrics.record_call();

        for (idx, rule) in self.rules.iter().enumerate() {
            if !rule.target.matches(operation) {
                continue;
            }

            // Check after_calls threshold
            let call_count = state.call_counts.get(operation);
            if call_count <= rule.after_calls {
                continue;
            }

            // Check max_injections
            if rule.max_injections > 0
                && state.rule_injection_counts[idx] >= rule.max_injections
            {
                continue;
            }

            // Roll probability
            let roll: f64 = state.rng.random();
            if roll >= rule.probability {
                continue;
            }

            // Handle partial failure specially
            if let Fault::PartialFailure(ref pf) = rule.fault {
                let count = &mut state.partial_failure_counts[idx];
                if *count < pf.fail_count {
                    *count += 1;
                    state.rule_injection_counts[idx] += 1;
                    let fault = Fault::Error(ErrorFault::new(pf.error_kind, &pf.message));
                    self.metrics.record_fault(&fault);
                    return Some(fault);
                }
                // Reset counter and allow passthrough
                *count = 0;
                continue;
            }

            state.rule_injection_counts[idx] += 1;
            self.metrics.record_fault(&rule.fault);
            return Some(rule.fault.clone());
        }

        self.metrics.record_passthrough();
        None
    }

    /// Apply a latency fault by sleeping the current thread.
    pub fn apply_latency(&self, fault: &crate::fault::LatencyFault) {
        let delay = if fault.min_ms == fault.max_ms {
            Duration::from_millis(fault.min_ms)
        } else {
            let ms = {
                let mut state = self.state.lock().unwrap();
                state.rng.random_range(fault.min_ms..=fault.max_ms)
            };
            Duration::from_millis(ms)
        };
        tracing::debug!(delay_ms = delay.as_millis() as u64, "chaos: injecting latency");
        thread::sleep(delay);
    }

    /// Convert an error fault to an AdapterError.
    pub fn fault_to_adapter_error(fault: &ErrorFault) -> AdapterError {
        AdapterError::new(fault.kind.to_adapter_error_kind(), &fault.message)
    }

    /// Convert a connection drop fault to an AdapterError.
    pub fn connection_drop_to_adapter_error(fault: &ConnectionDropFault) -> AdapterError {
        AdapterError::new(AdapterErrorKind::Driver, &fault.message)
    }

    /// Convert a timeout fault to an AdapterError (sleeps first).
    pub fn apply_timeout_fault(fault: &TimeoutFault) -> AdapterError {
        thread::sleep(Duration::from_millis(fault.delay_ms));
        AdapterError::new(AdapterErrorKind::Cancelled, &fault.message)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.call_counts = CallCounts::default();
        for count in state.rule_injection_counts.iter_mut() {
            *count = 0;
        }
        for count in state.partial_failure_counts.iter_mut() {
            *count = 0;
        }
        self.metrics.reset();
    }

    pub fn metrics(&self) -> &ChaosMetrics {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::{ChaosErrorKind, Fault, LatencyFault};

    #[test]
    fn test_controller_disabled() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "test")),
                1.0,
                TargetOperation::All,
            )],
            Some(42),
        );
        controller.set_enabled(false);
        assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());
    }

    #[test]
    fn test_controller_always_inject() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "always")),
                1.0,
                TargetOperation::Execute,
            )],
            Some(42),
        );

        let fault = controller.maybe_inject(&TargetOperation::Execute);
        assert!(fault.is_some());
        assert!(matches!(fault.unwrap(), Fault::Error(_)));
    }

    #[test]
    fn test_controller_never_inject() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "never")),
                0.0,
                TargetOperation::Execute,
            )],
            Some(42),
        );

        for _ in 0..100 {
            assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());
        }
    }

    #[test]
    fn test_controller_target_filtering() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "execute-only")),
                1.0,
                TargetOperation::Execute,
            )],
            Some(42),
        );

        assert!(controller
            .maybe_inject(&TargetOperation::NewConnection)
            .is_none());
        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
    }

    #[test]
    fn test_controller_after_calls() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "delayed")),
                1.0,
                TargetOperation::Execute,
            )
            .after_calls(2)],
            Some(42),
        );

        // First 2 calls should pass through
        assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());
        assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());
        // Third call should inject
        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
    }

    #[test]
    fn test_controller_max_injections() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "limited")),
                1.0,
                TargetOperation::Execute,
            )
            .max_injections(2)],
            Some(42),
        );

        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
        // Third injection should not happen
        assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());
    }

    #[test]
    fn test_controller_reset() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "resettable")),
                1.0,
                TargetOperation::Execute,
            )
            .max_injections(1)],
            Some(42),
        );

        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
        assert!(controller.maybe_inject(&TargetOperation::Execute).is_none());

        controller.reset();
        assert!(controller
            .maybe_inject(&TargetOperation::Execute)
            .is_some());
    }

    #[test]
    fn test_controller_metrics_tracking() {
        let controller = ChaosController::new(
            vec![FaultRule::new(
                Fault::Latency(LatencyFault::new(0, 0)),
                1.0,
                TargetOperation::Execute,
            )],
            Some(42),
        );

        controller.maybe_inject(&TargetOperation::Execute);
        controller.maybe_inject(&TargetOperation::NewConnection); // no rule matches

        let snap = controller.metrics().snapshot();
        assert_eq!(snap.total_calls, 2);
        assert_eq!(snap.faults_injected, 1);
        assert_eq!(snap.latency_faults, 1);
        assert_eq!(snap.passthroughs, 1);
    }

    #[test]
    fn test_seeded_determinism() {
        let make_controller = || {
            ChaosController::new(
                vec![FaultRule::new(
                    Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "deterministic")),
                    0.5,
                    TargetOperation::All,
                )],
                Some(12345),
            )
        };

        let c1 = make_controller();
        let c2 = make_controller();

        let results1: Vec<bool> = (0..20)
            .map(|_| c1.maybe_inject(&TargetOperation::Execute).is_some())
            .collect();
        let results2: Vec<bool> = (0..20)
            .map(|_| c2.maybe_inject(&TargetOperation::Execute).is_some())
            .collect();

        assert_eq!(results1, results2);
    }
}
