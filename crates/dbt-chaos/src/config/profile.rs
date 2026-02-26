use serde::{Deserialize, Serialize};

use crate::circuit_breaker::CircuitBreakerConfig;
use crate::config::builder::ChaosBuilder;
use crate::fault::FaultRule;

/// A YAML-deserializable chaos profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosProfile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub rules: Vec<FaultRule>,
    #[serde(default)]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    #[serde(default)]
    pub seed: Option<u64>,
}

impl ChaosProfile {
    /// Parse a chaos profile from YAML.
    pub fn from_yaml(yaml: &str) -> Result<Self, dbt_yaml::Error> {
        dbt_yaml::from_str(yaml)
    }

    /// Convert this profile into a `ChaosBuilder`.
    pub fn into_builder(self) -> ChaosBuilder {
        let mut builder = ChaosBuilder::new();

        for rule in self.rules {
            builder = builder.with_rule(rule);
        }

        if let Some(cb_config) = self.circuit_breaker {
            builder = builder.with_circuit_breaker(cb_config);
        }

        if let Some(seed) = self.seed {
            builder = builder.with_seed(seed);
        }

        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_from_yaml() {
        let yaml = r#"
name: test-profile
description: A test chaos profile
seed: 42
rules:
  - fault: !Error
      kind: Driver
      message: "simulated driver error"
    probability: 0.5
    target: Execute
    after_calls: 0
    max_injections: 0
  - fault: !Latency
      min_ms: 100
      max_ms: 500
    probability: 0.3
    target: All
    after_calls: 0
    max_injections: 0
circuit_breaker:
  failure_threshold: 5
  recovery_timeout_secs: 30
  success_threshold: 2
"#;

        let profile = ChaosProfile::from_yaml(yaml).unwrap();
        assert_eq!(profile.name, "test-profile");
        assert_eq!(profile.rules.len(), 2);
        assert_eq!(profile.seed, Some(42));
        assert!(profile.circuit_breaker.is_some());
    }

    #[test]
    fn test_profile_into_builder() {
        let profile = ChaosProfile {
            name: "test".to_string(),
            description: "test profile".to_string(),
            rules: vec![FaultRule::new(
                crate::fault::Fault::Error(crate::fault::ErrorFault::new(
                    crate::fault::ChaosErrorKind::Driver,
                    "test",
                )),
                1.0,
                crate::fault::TargetOperation::Execute,
            )],
            circuit_breaker: Some(CircuitBreakerConfig::default()),
            seed: Some(42),
        };

        let (controller, cb) = profile.into_builder().build();
        assert!(controller.is_enabled());
        assert!(cb.is_some());
    }

    #[test]
    fn test_profile_minimal_yaml() {
        let yaml = r#"
name: minimal
rules:
  - fault: !Timeout
      delay_ms: 1000
      message: "timed out"
    probability: 1.0
    target: All
    after_calls: 0
    max_injections: 0
"#;

        let profile = ChaosProfile::from_yaml(yaml).unwrap();
        assert_eq!(profile.name, "minimal");
        assert_eq!(profile.rules.len(), 1);
        assert!(profile.circuit_breaker.is_none());
        assert!(profile.seed.is_none());
    }
}
