use std::sync::Mutex;
use std::time::{Duration, Instant};

use dbt_common::{AdapterError, AdapterErrorKind, AdapterResult};
use serde::{Deserialize, Serialize};

/// Circuit breaker state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation; failures are being counted.
    Closed,
    /// Circuit is open; calls are rejected without attempting the inner operation.
    Open,
    /// Trial state; a limited number of calls are allowed through to test recovery.
    HalfOpen,
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before the circuit opens.
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,
    /// How long the circuit stays open before transitioning to half-open, in seconds.
    #[serde(default = "default_recovery_timeout_secs")]
    pub recovery_timeout_secs: u64,
    /// Number of consecutive successes in half-open state required to close the circuit.
    #[serde(default = "default_success_threshold")]
    pub success_threshold: u32,
}

fn default_failure_threshold() -> u32 {
    5
}
fn default_recovery_timeout_secs() -> u64 {
    30
}
fn default_success_threshold() -> u32 {
    2
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            recovery_timeout_secs: default_recovery_timeout_secs(),
            success_threshold: default_success_threshold(),
        }
    }
}

struct CircuitBreakerState {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
}

/// A three-state circuit breaker (Closed / Open / HalfOpen).
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    inner: Mutex<CircuitBreakerState>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                last_failure_time: None,
            }),
        }
    }

    /// Check if a call is allowed. Returns `Err` if the circuit is open
    /// and the recovery timeout hasn't elapsed.
    pub fn pre_call(&self) -> AdapterResult<()> {
        let mut state = self.inner.lock().unwrap();

        match state.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                let recovery_timeout =
                    Duration::from_secs(self.config.recovery_timeout_secs);
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= recovery_timeout {
                        state.state = CircuitState::HalfOpen;
                        state.success_count = 0;
                        tracing::info!("circuit breaker: transitioning to HalfOpen");
                        Ok(())
                    } else {
                        Err(AdapterError::new(
                            AdapterErrorKind::Driver,
                            "Circuit breaker is open: too many recent failures",
                        ))
                    }
                } else {
                    // No last failure recorded, transition to half-open
                    state.state = CircuitState::HalfOpen;
                    state.success_count = 0;
                    Ok(())
                }
            }
            CircuitState::HalfOpen => Ok(()),
        }
    }

    /// Record a successful call.
    pub fn record_success(&self) {
        let mut state = self.inner.lock().unwrap();

        match state.state {
            CircuitState::HalfOpen => {
                state.success_count += 1;
                if state.success_count >= self.config.success_threshold {
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                    tracing::info!("circuit breaker: transitioning to Closed");
                }
            }
            CircuitState::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            CircuitState::Open => {}
        }
    }

    /// Record a failed call.
    pub fn record_failure(&self) {
        let mut state = self.inner.lock().unwrap();

        match state.state {
            CircuitState::Closed => {
                state.failure_count += 1;
                if state.failure_count >= self.config.failure_threshold {
                    state.state = CircuitState::Open;
                    state.last_failure_time = Some(Instant::now());
                    tracing::warn!(
                        threshold = self.config.failure_threshold,
                        "circuit breaker: transitioning to Open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open goes back to open
                state.state = CircuitState::Open;
                state.last_failure_time = Some(Instant::now());
                state.success_count = 0;
                tracing::warn!("circuit breaker: HalfOpen -> Open (failure during probe)");
            }
            CircuitState::Open => {
                state.last_failure_time = Some(Instant::now());
            }
        }
    }

    pub fn state(&self) -> CircuitState {
        self.inner.lock().unwrap().state
    }

    pub fn reset(&self) {
        let mut state = self.inner.lock().unwrap();
        state.state = CircuitState::Closed;
        state.failure_count = 0;
        state.success_count = 0;
        state.last_failure_time = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(threshold: u32) -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: threshold,
            recovery_timeout_secs: 1,
            success_threshold: 2,
        }
    }

    #[test]
    fn test_starts_closed() {
        let cb = CircuitBreaker::new(test_config(3));
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.pre_call().is_ok());
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new(test_config(3));

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(cb.pre_call().is_err());
    }

    #[test]
    fn test_success_resets_failure_count() {
        let cb = CircuitBreaker::new(test_config(3));

        cb.record_failure();
        cb.record_failure();
        cb.record_success(); // Resets failure count
        cb.record_failure();
        cb.record_failure();
        // Should still be closed since success reset the counter
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            recovery_timeout_secs: 0, // Instant recovery for testing
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for recovery timeout (0 seconds)
        std::thread::sleep(Duration::from_millis(10));
        assert!(cb.pre_call().is_ok()); // Transitions to HalfOpen
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen); // Need 2 successes

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed); // Now closed
    }

    #[test]
    fn test_half_open_failure_reopens() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            recovery_timeout_secs: 0,
            success_threshold: 2,
        };
        let cb = CircuitBreaker::new(config);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        std::thread::sleep(Duration::from_millis(10));
        cb.pre_call().unwrap(); // Transitions to HalfOpen
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_failure(); // Goes back to Open
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_reset() {
        let cb = CircuitBreaker::new(test_config(1));
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.pre_call().is_ok());
    }
}
