/// Helper module for configurable driver debug logging.
///
/// This module provides utilities to configure driver logging levels via environment variables,
/// allowing users to enable debug logging when troubleshooting connection or authentication issues.
///
/// Following the pattern from dbt-adapters, each adapter can support an environment variable
/// like `DBT_<ADAPTER>_CONNECTOR_DEBUG_LOGGING` to control the logging level of the underlying driver.

use dbt_xdbc::database::LogLevel;
use std::env;

/// Get the log level for a driver from its corresponding environment variable.
///
/// # Arguments
///
/// * `env_var_name` - The name of the environment variable to check (e.g., "DBT_SNOWFLAKE_CONNECTOR_DEBUG_LOGGING")
/// * `default` - The default log level to use if the environment variable is not set or invalid
///
/// # Returns
///
/// Returns the configured `LogLevel` from the environment variable, or the default if not set or invalid.
///
/// # Environment Variable Format
///
/// The environment variable should be set to one of the following values (case-insensitive):
/// - `trace` - Most verbose logging
/// - `debug` - Debug-level logging
/// - `info` - Informational logging
/// - `warn` - Warning-level logging
/// - `error` - Error-level logging
/// - `fatal` - Only fatal errors
/// - `off` - Disable all logging
///
/// # Example
///
/// ```bash
/// # Enable debug logging for Snowflake driver
/// export DBT_SNOWFLAKE_CONNECTOR_DEBUG_LOGGING=debug
/// ```
pub fn get_driver_log_level(env_var_name: &str, default: LogLevel) -> LogLevel {
    env::var(env_var_name)
        .ok()
        .and_then(|val| {
            val.trim()
                .to_lowercase()
                .parse::<LogLevel>()
                .ok()
        })
        .unwrap_or(default)
}

/// Environment variable name for Snowflake connector debug logging.
///
/// Set this environment variable to control the logging level of the Snowflake Go driver.
/// This is compatible with dbt-adapters' `DBT_SNOWFLAKE_CONNECTOR_DEBUG_LOGGING` variable.
pub const SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV: &str = "DBT_SNOWFLAKE_CONNECTOR_DEBUG_LOGGING";

/// Get the Snowflake driver log level.
///
/// Checks the `DBT_SNOWFLAKE_CONNECTOR_DEBUG_LOGGING` environment variable.
/// If not set or invalid, defaults to `LogLevel::Fatal` (minimal logging).
///
/// # Returns
///
/// The configured log level for the Snowflake driver.
pub fn snowflake_log_level() -> LogLevel {
    get_driver_log_level(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV, LogLevel::Fatal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_get_driver_log_level_not_set() {
        let env_var = "DBT_TEST_DRIVER_LOG_LEVEL";
        unsafe {
            env::remove_var(env_var);
        }

        let level = get_driver_log_level(env_var, LogLevel::Error);
        assert_eq!(level.to_string(), "error");
    }

    #[test]
    #[serial]
    fn test_get_driver_log_level_valid() {
        let env_var = "DBT_TEST_DRIVER_LOG_LEVEL";

        for (input, expected) in [
            ("trace", "trace"),
            ("debug", "debug"),
            ("info", "info"),
            ("warn", "warn"),
            ("error", "error"),
            ("fatal", "fatal"),
            ("off", "off"),
            ("DEBUG", "debug"), // case insensitive
            ("  info  ", "info"), // whitespace trimming
        ] {
            unsafe {
                env::set_var(env_var, input);
            }
            let level = get_driver_log_level(env_var, LogLevel::Fatal);
            assert_eq!(level.to_string(), expected, "Failed for input: {}", input);
        }

        unsafe {
            env::remove_var(env_var);
        }
    }

    #[test]
    #[serial]
    fn test_get_driver_log_level_invalid() {
        let env_var = "DBT_TEST_DRIVER_LOG_LEVEL";
        unsafe {
            env::set_var(env_var, "invalid_level");
        }

        let level = get_driver_log_level(env_var, LogLevel::Warn);
        assert_eq!(level.to_string(), "warn");

        unsafe {
            env::remove_var(env_var);
        }
    }

    #[test]
    #[serial]
    fn test_snowflake_log_level_default() {
        unsafe {
            env::remove_var(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV);
        }

        let level = snowflake_log_level();
        assert_eq!(level.to_string(), "fatal");
    }

    #[test]
    #[serial]
    fn test_snowflake_log_level_debug() {
        unsafe {
            env::set_var(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV, "debug");
        }

        let level = snowflake_log_level();
        assert_eq!(level.to_string(), "debug");

        unsafe {
            env::remove_var(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV);
        }
    }

    #[test]
    #[serial]
    fn test_snowflake_log_level_info() {
        unsafe {
            env::set_var(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV, "info");
        }

        let level = snowflake_log_level();
        assert_eq!(level.to_string(), "info");

        unsafe {
            env::remove_var(SNOWFLAKE_CONNECTOR_DEBUG_LOGGING_ENV);
        }
    }
}
