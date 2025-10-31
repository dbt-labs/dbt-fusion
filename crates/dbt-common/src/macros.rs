pub use humantime as _vendored_human_time;

/// Format duration with fixed width for alignment (5 characters total)
/// Supports ns, μs, ms, s, m, h for materializations
pub fn format_duration_fixed_width(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs_f64();

    if total_secs >= 999.0 {
        // >= 999 seconds: show fixed indicator for very long operations
        "LONG!!!".to_string()
    } else if total_secs == 0.0 {
        "-------".to_string()
    } else {
        // 0-999 seconds: always show in seconds with 2 decimal places, right-aligned with spaces
        format!("{total_secs:6.2}s")
    }
}

/// Format schema and alias with truncation for long schema names
/// If schema is longer than 20 characters, truncate to "long_schema_na....::alias"
pub fn format_schema_alias(schema: &str, alias: &str) -> String {
    const MAX_SCHEMA_LEN: usize = 200;

    let schema = if schema.len() > MAX_SCHEMA_LEN {
        format!("{}...", &schema[..MAX_SCHEMA_LEN.saturating_sub(4)])
    } else {
        format!("{schema}.")
    };
    format!("{}{}", CYAN.apply_to(schema), BLUE.apply_to(alias))
}

// Re-export dbt_error here such that downstream crates could use
// macros using dbt_error without having to add it as a dependency explicitly.
pub use dbt_error as _dbt_error;

// Re-export dbt-telemetry to allow using it in macros without requiring
// the call-site crate to declare it as a dependency explicitly
pub use dbt_telemetry as _dbt_telemetry;

use crate::pretty_string::{BLUE, CYAN};

/// Format resource type with minimum width for alignment
/// Minimum width is 5 characters (length of "model") but allows longer strings
pub fn format_resource_type_fixed_width(resource_type: &str) -> String {
    use crate::pretty_string::PLAIN;
    const MIN_WIDTH: usize = 5; // Length of "model"

    let formatted = PLAIN.apply_to(resource_type).to_string();

    // Pad if shorter than minimum width, otherwise return as-is
    if resource_type.len() < MIN_WIDTH {
        format!(
            "{}{}",
            formatted,
            " ".repeat(MIN_WIDTH - resource_type.len())
        )
    } else {
        formatted
    }
}

/// Format materialization without fixed width (for end of line)
pub fn format_materialization_suffix(materialization: Option<&str>, desc: Option<&str>) -> String {
    let truncated_mat = match materialization {
        Some("materialized_view") => Some("mat_view"),
        Some("streaming_table") => Some("streaming"),
        // Hide materialization label for tests and unit tests
        Some("test") | Some("unit_test") | Some("unit") | None => None,
        Some(other) => Some(other),
    };
    match (truncated_mat, desc) {
        (Some(mat), Some(desc)) => format!(" ({mat} - {desc})"),
        (Some(mat), None) => format!(" ({mat})"),
        (None, Some(desc)) => format!(" ({desc})"),
        (None, None) => String::new(),
    }
}

/// fsinfo! constructs an FsInfo struct with optional data and desc fields
#[macro_export]
macro_rules! fsinfo {
    // Basic version with just event and target
    ($event:expr, $target:expr) => {
        $crate::logging::FsInfo {
            event: $event,
            target: $target,
            data: None,
            desc: None,
        }
    };
    // Version with desc
    ($event:expr, $target:expr, desc = $desc:expr) => {
        $crate::logging::FsInfo {
            event: $event,
            target: $target,
            data: None,
            desc: Some($desc),
        }
    };
    // Version with data
    ($event:expr, $target:expr, data = $data:expr) => {
        $crate::logging::FsInfo {
            event: $event,
            target: $target,
            data: Some($data),
            desc: None,
        }
    };
    // Version with both data and desc
    ($event:expr, $target:expr, data = $data:expr, desc = $desc:expr) => {
        $crate::logging::FsInfo {
            event: $event,
            target: $target,
            data: Some($data),
            desc: Some($desc),
        }
    };
}

// ------------------------------------------------------------------------------------------------
// The following macros are logging related. They assume that the io args has the function:
// should_show(option: ShowOptions) -> bool
// logger is initialized by init_logger and will specify the output destination and format
// ------------------------------------------------------------------------------------------------

#[macro_export]
macro_rules! show_result {
    ( $io:expr, $option:expr, $artifact:expr) => {
        $crate::show_result!($io, $option, $artifact, columns = Option::<&[String]>::None)
    };

    ( $io:expr, $option:expr, $artifact:expr, columns = $columns:expr) => {
        $crate::show_result!($io, $option, $artifact, columns = $columns, unique_id = Option::<&str>::None)
    };

    ( $io:expr, $option:expr, $artifact:expr, columns = $columns:expr, unique_id = $unique_id:expr) => {{
        use $crate::io_args::ShowOptions;
        use dbt_common::constants::INLINE_NODE;
        use serde_json::json;
        if $io.should_show($option) {
            let output = format!("\n{}", $artifact);
            // this preview field and name is used by the dbt-cloud CLI to display the result
            let node_id = $unique_id.unwrap_or(INLINE_NODE);
            let mut data = json!({
                "preview": $artifact.to_string(),
                "unique_id": node_id
            });

            // columns can be used to show column names when the resultset is empty, eg.
            //   { "preview": "[]", "columns": ["column1", "column2"] }
            if let Some(cols) = $columns {
                data["columns"] = json!(cols);
            }

            $crate::_log!(
                $crate::macros::log_adapter::log::Level::Info,
                _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                name= "ShowNode",
                data:serde = data;
                "{}", output
            );
        }
    }};
}

#[macro_export]
macro_rules! show_result_with_default_title {
    ( $io:expr, $option:expr, $artifact:expr) => {{
        use $crate::io_args::ShowOptions;
        if $io.should_show($option) {
            let output = format!("\n{}\n{}", $option.title(), $artifact);
            $crate::_log!(
                $crate::macros::log_adapter::log::Level::Info,
                _INVOCATION_ID_ = $io.invocation_id.as_u128();
                "{}",
                 output
            );
        }
    }};
}

#[macro_export]
macro_rules! show_list_result_with_default_title {
    ( $io:expr, $option:expr, $list_result:expr) => {{
        use $crate::io_args::ShowOptions;
        use $crate::pretty_string::BLUE;
        if $io.should_show($option) {
            $crate::_log!(
                $crate::macros::log_adapter::log::Level::Info,
                "\n{}",
                BLUE.apply_to($option.title())
            );

            for item in $list_result {
                $crate::_log!(
                    $crate::macros::log_adapter::log::Level::Info,
                    _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                    name = "PrintEvent",
                    code = "Z052";
                    "{}",
                    item
                );
            }
        }
    }};
}

#[macro_export]
macro_rules! show_result_with_title {
    ( $io:expr, $option:expr, $title: expr, $artifact:expr) => {{
        use $crate::io_args::ShowOptions;
        use $crate::pretty_string::BLUE;
            use dbt_common::constants::INLINE_NODE;
        use serde_json::json;
        if $io.should_show($option) {
            let output = format!("\n{}\n{}", $title, $artifact);
            $crate::_log!(
                $crate::macros::log_adapter::log::Level::Info,
                _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                name= "ShowNode",
                data:serde = json!({ "preview": $artifact.to_string(), "unique_id": INLINE_NODE });
                "{}", output
            );
        }
    }};
}

#[macro_export]
macro_rules! show_progress {
    ( $io:expr, $info:expr) => {{
        use $crate::io_args::ShowOptions;
        use $crate::pretty_string::pretty_green;
        use $crate::logging::{FsInfo, LogEvent, LogFormat};

        if !$info.is_phase_completed() {

            if let Some(reporter) = &$io.status_reporter {
                reporter.show_progress($info.event.action().as_str(), &$info.target, $info.desc.as_deref());
            }

            // This whole macro became entirely unweldy, the following condition is a VERY
            // temporary bandaid fix for a regression where JSON output was not being emitted
            // for certain progress events. The entire macro is expected to be removed
            // after migration to new tracing-based logging is complete.
            let should_emit_json_event = $io.log_format == LogFormat::Json && ($info.is_phase_render()
                || $info.is_phase_run());

            // TODO: these filtering conditions should be moved to the logger side
            if (
                ($io.should_show(ShowOptions::Progress) && $info.is_phase_unknown())
                || ($io.should_show(ShowOptions::ProgressHydrate) && $info.is_phase_hydrate())
                || ($io.should_show(ShowOptions::ProgressParse) && $info.is_phase_parse())
                || ($io.should_show(ShowOptions::ProgressRender) && $info.is_phase_render())
                || ($io.should_show(ShowOptions::ProgressAnalyze) && $info.is_phase_analyze())
                || ($io.should_show(ShowOptions::ProgressRun) && $info.is_phase_run())
                || should_emit_json_event
            )
                // Do not show parse/compile generic tests
                && !($info.target.contains(dbt_common::constants::DBT_GENERIC_TESTS_DIR_NAME)
                    && ($info.event.action().as_str().contains(dbt_common::constants::PARSING)
                        || $info.event.action().as_str().contains(dbt_common::constants::RENDERING)
                        || $info.event.action().as_str().contains(dbt_common::constants::ANALYZING)))
            {
                let output = pretty_green($info.event.action().as_str(), &$info.target, $info.desc.as_deref());
                let event = $info.event;
                if let Some(data_json) = $info.data {
                    $crate::_log!(event.level(),
                        _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                        _TRACING_HANDLED_ = true,
                        name = event.name(), data:serde = data_json;
                        "{}", output
                    );
                } else {
                    $crate::_log!(event.level(),
                        _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                        _TRACING_HANDLED_ = true,
                        name = event.name();
                        "{}", output
                    );
                }
            }
        }
    }};
}

#[macro_export]
macro_rules! show_completed {
    // Generic completion for node materializations (model/seed/snapshot)
    (
        $io:expr,
        $task:expr,
        $node_info:expr,
        $node_status:expr,
        $display_path:expr,
        $start_time:expr,
        $end_time:expr,
        $with_cache:expr
    ) => {{
        use $crate::io_args::ShowOptions;
        use $crate::constants::SUCCEEDED;
        use $crate::logging::LogEvent;
        use $crate::stats::NodeStatus;
        use $crate::macros::{format_duration_fixed_width, format_schema_alias, format_resource_type_fixed_width, format_materialization_suffix};
        use $crate::pretty_string::{GREEN, BLUE};
        use $crate::macros::_vendored_human_time::{format_duration, FormattedDuration};
        use chrono::Utc;

        if !matches!($node_status, &NodeStatus::NoOp) {
            let log_event: LogEvent = $node_status.clone().into();
            let duration = $end_time
                .signed_duration_since($start_time)
                .to_std()
                .unwrap_or_default();
            let resource_type_str = $task.resource_type();
            let materialization = $task.base().materialized.to_string();
            // Default schema/alias
            let mut schema = $task.base().schema.clone();
            let mut alias = $task.base().alias.clone();
            // For unit tests: display test schema + unit test name
            if resource_type_str == NodeType::UnitTest {
                schema = format!("{}_dbt_test__audit", schema);
                alias = $task.common().name.clone();
            }

            let desc = if matches!($node_status, NodeStatus::Succeeded) {
                $with_cache.then_some("New changes detected".to_string())
            } else if matches!($node_status, NodeStatus::TestWarned | NodeStatus::Errored) &&
                      let Some(location) = $node_info.defined_at {
                Some(location.to_string())
            } else {
                $node_status.get_message()
            };

            if let Some(reporter) = &$io.status_reporter {
                reporter.show_progress(log_event.action().as_str(), &$display_path, desc.as_deref());
            }

            if $io.should_show(ShowOptions::Completed) {
                let schema_alias = format_schema_alias(&schema, &alias);
                let resource_type_formatted = format_resource_type_fixed_width(resource_type_str.as_ref());
                let materialization_suffix = format_materialization_suffix(Some(&materialization), desc.as_deref());
                let duration = format_duration_fixed_width(duration);
                let output = format!(
                    "{} [{}] {} {}{}",
                    log_event.formatted_action(),
                    duration,
                    resource_type_formatted,
                    schema_alias,
                    materialization_suffix
                );

                $crate::_log!(
                    log_event.level(),
                    _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                    _TRACING_HANDLED_ = true,
                    name = log_event.name(),
                    data:serde = dbt_serde_yaml::to_value($node_info).expect("Failed to serialize node info");
                    "{}",
                    output
                );
            }
        }
    }};
    // Show completed for multiple nodes
    // Generic completion for node materializations (model/seed/snapshot)
    (
        $io:expr,
        $node_info:expr,
        $node_status:expr,
        $start_time:expr,
        $end_time:expr,
        $skipped_test_unique_ids:expr,
        $skipped_test_node_names:expr
    ) => {{
        use $crate::io_args::ShowOptions;
        use $crate::constants::SUCCEEDED;
        use $crate::logging::LogEvent;
        use $crate::stats::NodeStatus;
        use $crate::macros::{format_duration_fixed_width, format_schema_alias, format_resource_type_fixed_width, format_materialization_suffix};
        use $crate::pretty_string::{GREEN, BLUE, CYAN, YELLOW};
        use $crate::macros::_vendored_human_time::{format_duration, FormattedDuration};
        use chrono::Utc;

        if matches!($node_status, &NodeStatus::SkippedUpstreamFailed) {

            let message = if $skipped_test_node_names.len() > 3 {
                format!(
                    "{} and {} others",
                    $skipped_test_node_names
                        .iter()
                        .take(2)
                        .map(|name| format!("'{}'", YELLOW.apply_to(name)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    $skipped_test_node_names.len() - 2
                )
            } else {
                format!(
                    "{}",
                    $skipped_test_node_names
                        .iter()
                        .map(|name| format!("'{}'", YELLOW.apply_to(name)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let log_event: LogEvent = $node_status.clone().into();
            let duration = $end_time
                .signed_duration_since($start_time)
                .to_std()
                .unwrap_or_default();

            // If unique_ids starts with test or unit_test then resource type should be test,unit_test
            let any_test = $skipped_test_unique_ids.iter().any(|id| id.starts_with("test"));
            let any_unit_test = $skipped_test_unique_ids.iter().any(|id| id.starts_with("unit_test"));
            let resource_type = match (any_test, any_unit_test) {
                (true, true) => "test,unit_test",
                (true, false) => "test",
                (false, true) => "unit_test",
                (false, false) => "unknown",
            };

            if $io.should_show(ShowOptions::Completed) {
                let resource_type_formatted = format_resource_type_fixed_width(resource_type);
                let duration = format_duration_fixed_width(duration);
                let output = format!(
                    "{} [{}] {} {}",
                    log_event.formatted_action(),
                    duration,
                    resource_type_formatted,
                    message
                );

                $crate::_log!(
                    log_event.level(),
                    _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                    _TRACING_HANDLED_ = true,
                    name = log_event.name(),
                    data:serde = dbt_serde_yaml::to_value($node_info).expect("Failed to serialize node info");
                    "{}",
                    output
                );
            }
        }
    }};
}

#[macro_export]
macro_rules! show_info {
    ( $io:expr, $info:expr) => {{
        use $crate::io_args::ShowOptions;
        use $crate::pretty_string::pretty_green;
        use $crate::logging::{FsInfo, LogEvent};


        if let Some(reporter) = &$io.status_reporter {
            reporter.show_progress($info.event.action().as_str(), &$info.target, $info.desc.as_deref());
        }

        // TODO: these filtering conditions should be moved to the logger side
        if (
            ($io.should_show(ShowOptions::Progress) && $info.is_phase_unknown())
            || ($io.should_show(ShowOptions::ProgressParse) && $info.is_phase_parse())
            || ($io.should_show(ShowOptions::ProgressRender) && $info.is_phase_render())
            || ($io.should_show(ShowOptions::ProgressAnalyze) && $info.is_phase_analyze())
            || ($io.should_show(ShowOptions::ProgressRun) && $info.is_phase_run())
        )
            // Do not show parse/compile generic tests
            && !($info.target.contains(dbt_common::constants::DBT_GENERIC_TESTS_DIR_NAME)
                && ($info.event.action().as_str().contains(dbt_common::constants::PARSING)
                    || $info.event.action().as_str().contains(dbt_common::constants::RENDERING)
                    || $info.event.action().as_str().contains(dbt_common::constants::ANALYZING)))
        {
            let output = pretty_green($info.event.action().as_str(), &$info.target, $info.desc.as_deref());
            let event = $info.event;
            if let Some(data_json) = $info.data {
                $crate::_log!(event.level(),
                    _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                    name = event.name(), data:serde = data_json;
                     "{}", output
                );
            } else {
                $crate::_log!(event.level(),
                    _INVOCATION_ID_ = $io.invocation_id.as_u128(),
                    name = event.name();
                     "{}", output
                );
            }
        }
    }};
}

#[macro_export]
/// Display a progress bar or spinner with optional context items.
///
/// Each progress bar or spinner must have a unique identifier (`uid`), which is
/// a string that is displayed as a prefix to the left of the progress bar or
/// spinner. It is the caller's responsibility to ensure that the `uid` is
/// unique -- only a single progress bar or spinner with a given `uid` will be
/// displayed at a time, if a bar or spinner with a given `uid` is already
/// displayed, subsequent calls to this macro with the same `uid` will be
/// silently ignored.
///
/// When a progress bar or spinner is active, it can be associated with context
/// items that provide additional information about the progress being made.
/// Context items will be displayed as a list of items on the right side of the
/// progress bar or spinner, as much as space allows.
///
/// All variants of this macro returns a scope guard that will automatically
/// remove the progress bar or item when it goes out of scope.
macro_rules! with_progress {

    // Start a new spinner
    ($io:expr, spinner => $uid:expr ) => {{
        use $crate::logging::ProgressBarGuard;
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::start_spinner($uid.into());

            "Starting spinner with uid: {}",
            $uid
        );
        ProgressBarGuard::new(
            $io.invocation_id.as_u128(),
            TermEvent::remove_spinner($uid.into())
        )
    }};

    // Add a context item to the spinner
    ($io:expr, spinner => $uid:expr, item => $item:expr ) => {{
        use $crate::logging::ProgressBarGuard;
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::add_spinner_context_item($uid.into(), $item.into());

            "Starting item: {} on spinner: {}",
            $item, $uid
        );
        ProgressBarGuard::new(
            $io.invocation_id.as_u128(),
            TermEvent::finish_spinner_context_item($uid.into(), $item.into())
        )
    }};

    // Start a new progress bar with a total length
    ($io:expr, bar => $uid:expr, length => $total:expr ) => {{
        use $crate::logging::ProgressBarGuard;
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::start_bar($uid.into(), $total as u64);

            "Starting progress bar with uid: {}, total: {}",
            $uid, $total
        );
        ProgressBarGuard::new(
            $io.invocation_id.as_u128(),
            TermEvent::remove_bar($uid.into())
        )
    }};

    // Add a context item to the progress bar and increment the progress bar by
    // one
    ($io:expr, bar => $uid:expr, item => $item:expr ) => {{
        use $crate::logging::ProgressBarGuard;
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::add_bar_context_item($uid.into(), $item.into());

            "Starting item: {} on progress bar: {}",
            $item, $uid
        );
        ProgressBarGuard::new(
            $io.invocation_id.as_u128(),
            TermEvent::finish_bar_context_item($uid.into(), $item.into(), None)
        )
    }};
}

#[macro_export]
/// Show a new progress bar or spinner, or add an in-progress item to an
/// existing one
macro_rules! start_progress {

    ($io:expr, spinner => $uid:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::start_spinner($uid.into());

            "Starting spinner with uid: {}",
            $uid
        );
    }};

    ($io:expr, bar => $uid:expr, length => $total:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::start_bar($uid.into(), $total.into());

            "Starting progress bar with uid: {}, total: {}",
            $uid, $total
        );
    }};

    ($io:expr, spinner => $uid:expr, item => $item:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::add_spinner_context_item($uid.into(), $item.into());

            "Updating progress for uid: {}, item: {}",
            $uid, $item
        );
    }};

    ($io:expr, bar => $uid:expr, item => $item:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::add_bar_context_item($uid.into(), $item.into());

            "Updating progress for uid: {}, item: {}",
            $uid, $item
        );
    }};
}

#[macro_export]
macro_rules! finish_progress {
    ($io:expr, spinner => $uid:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::remove_spinner($uid.into());

            "Finishing spinner with uid: {}",
            $uid
        );
    }};

    ($io:expr, bar => $uid:expr) => {{
        use $crate::logging::TermEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _TERM_EVENT_:serde = TermEvent::remove_bar($uid.into());

            "Finishing progress bar with uid: {}",
            $uid
        );
    }};

    ($io:expr, bar => $uid:expr, item => $item:expr, outcome => $outcome:expr) => {{
        use $crate::logging::TermEvent;
        use $crate::logging::StatEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _STAT_EVENT_:serde = $crate::logging::StatEvent::counter(
                $outcome,
                1
            ),
            _TERM_EVENT_:serde = TermEvent::finish_bar_context_item($uid.into(), $item.into());

            "Finishing item: {} on progress bar: {}",
            $item, $uid
        );
    }};

    ($io:expr, spinner => $uid:expr, outcome => $outcome:expr) => {{
        use $crate::logging::TermEvent;
        use $crate::logging::StatEvent;

        $crate::_log!(
            $crate::macros::log_adapter::log::Level::Info,
            _INVOCATION_ID_ = $io.invocation_id.as_u128(),
            _TERM_ONLY_ = true,
            _STAT_EVENT_:serde = StatEvent::counter(
                $outcome.into(),
                1
            ),
            _TERM_EVENT_:serde = TermEvent::finish_spinner_context_item($uid.into(), "".into());

            "Finishing spinner with uid: {}, outcome: {}",
            $uid, $outcome
        );
    }};

}

// --------------------------------------------------------------------------------------------------

/// Returns the fully qualified name of the current function.
#[macro_export]
macro_rules! current_function_name {
    () => {{
        fn f() {}
        fn type_name_of_val<T>(_: T) -> &'static str {
            ::std::any::type_name::<T>()
        }
        let mut name = type_name_of_val(f).strip_suffix("::f").unwrap_or("");
        while let Some(rest) = name.strip_suffix("::{{closure}}") {
            name = rest;
        }
        name
    }};
}

/// Returns just the name of the current function without the module path.
#[macro_export]
macro_rules! current_function_short_name {
    () => {{
        fn f() {}
        fn type_name_of_val<T>(_: T) -> &'static str {
            ::std::any::type_name::<T>()
        }
        let mut name = type_name_of_val(f).strip_suffix("::f").unwrap_or("");
        // If this macro is used in a closure, the last path segment will be {{closure}}
        // but we want to ignore it
        // Caveat: for example, this is the case if you use this macro in a a async test function annotated with #[tokio::test]
        while let Some(rest) = name.strip_suffix("::{{closure}}") {
            name = rest;
        }
        name.split("::").last().unwrap_or("")
    }};
}

/// Returns the path to the crate of the caller
#[macro_export]
macro_rules! this_crate_path {
    () => {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    };
}

#[cfg(test)]
mod tests {
    // top-level function test
    fn test_function_1() -> &'static str {
        current_function_short_name!()
    }

    mod nested {
        pub fn test_nested_function() -> &'static str {
            current_function_short_name!()
        }
    }

    #[test]
    fn test_current_function_short_name() {
        assert_eq!(test_function_1(), "test_function_1");
        assert_eq!(nested::test_nested_function(), "test_nested_function");

        let closure = || current_function_short_name!();
        assert_eq!(closure(), "test_current_function_short_name");
    }

    // top-level function test
    fn test_function_2() -> &'static str {
        current_function_name!()
    }

    #[test]
    fn test_current_function_name() {
        assert_eq!(
            test_function_2(),
            "dbt_common::macros::tests::test_function_2"
        );

        // test closure
        let closure: fn() -> &'static str = || current_function_name!();
        let closure_name = closure();
        assert_eq!(
            closure_name,
            "dbt_common::macros::tests::test_current_function_name"
        );
    }
}

/// This module contains a workaround for
///
///     non-primitive cast: `&[(&str, Value<'_>); 1]` as `&[(&str, Value<'_>)]`rust-analyzer(E0605)
///
/// TODO: remove this once the issue is fixed in upstream (either by 'rust-analyzer', or by 'log' crate)
#[macro_use]
pub mod log_adapter {
    pub use log;

    #[macro_export]
    #[clippy::format_args]
    macro_rules! _log {
        // log!(logger: my_logger, target: "my_target", Level::Info, "a {} event", "log");
        (logger: $logger:expr, target: $target:expr, $lvl:expr, $($arg:tt)+) => ({
            $crate::__log!(
                logger: $crate::macros::log_adapter::log::__log_logger!($logger),
                target: $target,
                $lvl,
                $($arg)+
            )
        });

        // log!(logger: my_logger, Level::Info, "a log event")
        (logger: $logger:expr, $lvl:expr, $($arg:tt)+) => ({
            $crate::__log!(
                logger: $crate::macros::log_adapter::log::__log_logger!($logger),
                target: $crate::macros::log_adapter::log::__private_api::module_path!(),
                $lvl,
                $($arg)+
            )
        });

        // log!(target: "my_target", Level::Info, "a log event")
        (target: $target:expr, $lvl:expr, $($arg:tt)+) => ({
            $crate::__log!(
                logger: $crate::macros::log_adapter::log::__log_logger!(__log_global_logger),
                target: $target,
                $lvl,
                $($arg)+
            )
        });

        // log!(Level::Info, "a log event")
        ($lvl:expr, $($arg:tt)+) => ({
            $crate::__log!(
                logger: $crate::macros::log_adapter::log::__log_logger!(__log_global_logger),
                target: $crate::macros::log_adapter::log::__private_api::module_path!(),
                $lvl,
                $($arg)+
            )
        });
    }

    #[doc(hidden)]
    #[macro_export]
    macro_rules! __log {
        // log!(logger: my_logger, target: "my_target", Level::Info, key1:? = 42, key2 = true; "a {} event", "log");
        (logger: $logger:expr, target: $target:expr, $lvl:expr, $($key:tt $(:$capture:tt)? $(= $value:expr)?),+; $($arg:tt)+) => ({
            let lvl = $lvl;
            if lvl <= $crate::macros::log_adapter::log::STATIC_MAX_LEVEL && lvl <= $crate::macros::log_adapter::log::max_level() {
                $crate::macros::log_adapter::log::__private_api::log(
                    $logger,
                    format_args!($($arg)+),
                    lvl,
                    &($target, $crate::macros::log_adapter::log::__private_api::module_path!(), $crate::macros::log_adapter::log::__private_api::loc()),
                    [$(($crate::macros::log_adapter::log::__log_key!($key), $crate::macros::log_adapter::log::__log_value!($key $(:$capture)* = $($value)*))),+].as_slice(),
                );
            }
        });

        // log!(logger: my_logger, target: "my_target", Level::Info, "a {} event", "log");
        (logger: $logger:expr, target: $target:expr, $lvl:expr, $($arg:tt)+) => ({
            let lvl = $lvl;
            if lvl <= $crate::macros::log_adapter::log::STATIC_MAX_LEVEL && lvl <= $crate::macros::log_adapter::log::max_level() {
                $crate::macros::log_adapter::log::__private_api::log(
                    $logger,
                    format_args!($($arg)+),
                    lvl,
                    &($target, $crate::macros::log_adapter::log::__private_api::module_path!(), $crate::macros::log_adapter::log::__private_api::loc()),
                    (),
                );
            }
        });
    }
}

#[macro_export]
macro_rules! show_selected_nodes_summary {
    ($io:expr, $schedule:expr, $nodes:expr) => {{
        use $crate::io_args::ShowOptions;
        use std::collections::BTreeMap;

        if $io.should_show(ShowOptions::Progress) {
            // Count nodes by resource type
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();

            for node_id in &$schedule.selected_nodes {
                if let Some(node) = $nodes.get_node(node_id) {
                    let resource_type = node.resource_type();
                    *counts.entry(resource_type).or_insert(0) += 1;
                }
            }

            // Build the summary string
            let mut parts = Vec::new();
            for (resource_type, count) in counts {
                let resource_name = match resource_type {
                    "model" => "models",
                    "test" => "data tests",
                    "unit_test" => "unit tests",
                    "source" => "sources",
                    "seed" => "seeds",
                    "snapshot" => "snapshots",
                    "analysis" => "analyses",
                    "operation" => "operations",
                    "exposure" => "exposures",
                    "metric" => "metrics",
                    "macro" => "macros",
                    "group" => "groups",
                    "semantic_model" => "semantic models",
                    "saved_query" => "saved queries",
                    _ => resource_type,
                };
                parts.push(format!("{} {}", count, resource_name));
            }

            if !parts.is_empty() {
                let summary = format!("Found {}", parts.join(", "));
                $crate::_log!(
                    $crate::macros::log_adapter::log::Level::Info,
                    _INVOCATION_ID_ = $io.invocation_id.as_u128();
                    "{}",
                    summary
                );
            }
        }
    }};
}
