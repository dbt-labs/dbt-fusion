use std::sync::atomic::{AtomicU64, Ordering};

/// Lock-free atomic counters for chaos observability.
pub struct ChaosMetrics {
    pub(crate) total_calls: AtomicU64,
    pub(crate) faults_injected: AtomicU64,
    pub(crate) latency_faults: AtomicU64,
    pub(crate) error_faults: AtomicU64,
    pub(crate) partial_failure_faults: AtomicU64,
    pub(crate) connection_drop_faults: AtomicU64,
    pub(crate) timeout_faults: AtomicU64,
    pub(crate) passthroughs: AtomicU64,
}

impl ChaosMetrics {
    pub fn new() -> Self {
        Self {
            total_calls: AtomicU64::new(0),
            faults_injected: AtomicU64::new(0),
            latency_faults: AtomicU64::new(0),
            error_faults: AtomicU64::new(0),
            partial_failure_faults: AtomicU64::new(0),
            connection_drop_faults: AtomicU64::new(0),
            timeout_faults: AtomicU64::new(0),
            passthroughs: AtomicU64::new(0),
        }
    }

    pub fn record_call(&self) {
        self.total_calls.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_fault(&self, fault: &crate::fault::Fault) {
        self.faults_injected.fetch_add(1, Ordering::Relaxed);
        match fault {
            crate::fault::Fault::Latency(_) => {
                self.latency_faults.fetch_add(1, Ordering::Relaxed);
            }
            crate::fault::Fault::Error(_) => {
                self.error_faults.fetch_add(1, Ordering::Relaxed);
            }
            crate::fault::Fault::PartialFailure(_) => {
                self.partial_failure_faults.fetch_add(1, Ordering::Relaxed);
            }
            crate::fault::Fault::ConnectionDrop(_) => {
                self.connection_drop_faults.fetch_add(1, Ordering::Relaxed);
            }
            crate::fault::Fault::Timeout(_) => {
                self.timeout_faults.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn record_passthrough(&self) {
        self.passthroughs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_calls: self.total_calls.load(Ordering::Relaxed),
            faults_injected: self.faults_injected.load(Ordering::Relaxed),
            latency_faults: self.latency_faults.load(Ordering::Relaxed),
            error_faults: self.error_faults.load(Ordering::Relaxed),
            partial_failure_faults: self.partial_failure_faults.load(Ordering::Relaxed),
            connection_drop_faults: self.connection_drop_faults.load(Ordering::Relaxed),
            timeout_faults: self.timeout_faults.load(Ordering::Relaxed),
            passthroughs: self.passthroughs.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.total_calls.store(0, Ordering::Relaxed);
        self.faults_injected.store(0, Ordering::Relaxed);
        self.latency_faults.store(0, Ordering::Relaxed);
        self.error_faults.store(0, Ordering::Relaxed);
        self.partial_failure_faults.store(0, Ordering::Relaxed);
        self.connection_drop_faults.store(0, Ordering::Relaxed);
        self.timeout_faults.store(0, Ordering::Relaxed);
        self.passthroughs.store(0, Ordering::Relaxed);
    }
}

impl Default for ChaosMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Point-in-time snapshot of chaos metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub total_calls: u64,
    pub faults_injected: u64,
    pub latency_faults: u64,
    pub error_faults: u64,
    pub partial_failure_faults: u64,
    pub connection_drop_faults: u64,
    pub timeout_faults: u64,
    pub passthroughs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::{ErrorFault, ChaosErrorKind, Fault, LatencyFault};

    #[test]
    fn test_metrics_counting() {
        let metrics = ChaosMetrics::new();

        metrics.record_call();
        metrics.record_call();
        metrics.record_fault(&Fault::Latency(LatencyFault::new(10, 100)));
        metrics.record_fault(&Fault::Error(ErrorFault::new(ChaosErrorKind::Driver, "test")));
        metrics.record_passthrough();

        let snap = metrics.snapshot();
        assert_eq!(snap.total_calls, 2);
        assert_eq!(snap.faults_injected, 2);
        assert_eq!(snap.latency_faults, 1);
        assert_eq!(snap.error_faults, 1);
        assert_eq!(snap.passthroughs, 1);
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = ChaosMetrics::new();
        metrics.record_call();
        metrics.record_fault(&Fault::Latency(LatencyFault::new(10, 100)));

        metrics.reset();
        let snap = metrics.snapshot();
        assert_eq!(snap.total_calls, 0);
        assert_eq!(snap.faults_injected, 0);
    }
}
