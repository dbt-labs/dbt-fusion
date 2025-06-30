use chrono::{DateTime, Local};
use humantime::format_duration;
use std::fmt;
use std::time::{Duration, SystemTime};
use strum_macros::EnumString;

// ------------------------------------------------------------------------------------------------
// Trivial Stats, foundation for run-results

#[derive(Debug, Clone)]
pub struct Stats {
    pub stats: Vec<Stat>,
}

#[derive(EnumString, PartialEq, Debug, Clone, Copy, Default)]
pub enum NodeStatus {
    // the following states can be reported on the makefile
    #[default]
    Succeeded,
    Errored,
    SkippedUpstreamReused,
    SkippedUpstreamFailed,
    ReusedNoChanges,
    ReusedStillFresh,
    NoOp,
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match self {
            NodeStatus::Succeeded => "success",
            NodeStatus::Errored => "error",
            NodeStatus::SkippedUpstreamReused | NodeStatus::SkippedUpstreamFailed => "skipped",
            NodeStatus::ReusedNoChanges | NodeStatus::ReusedStillFresh => "reused",
            NodeStatus::NoOp => "noop",
        };
        write!(f, "{status_str}")
    }
}

#[derive(Debug, Clone)]
pub struct Stat {
    pub unique_id: String,
    pub num_rows: Option<usize>,
    pub start_time: SystemTime,
    pub end_time: SystemTime,
    pub status: NodeStatus,
}

impl Stat {
    pub fn new(
        unique_id: String,
        start_time: SystemTime,
        num_rows: Option<usize>,
        status: NodeStatus,
    ) -> Self {
        let end_time = SystemTime::now();
        Stat {
            unique_id,
            num_rows,
            start_time,
            end_time,
            status,
        }
    }

    pub fn get_duration(&self) -> Duration {
        self.end_time
            .duration_since(self.start_time)
            .unwrap_or_default()
    }

    pub fn format_time(system_time: SystemTime) -> String {
        let datetime: DateTime<Local> = DateTime::from(system_time);
        datetime.format("%H:%M:%S").to_string()
    }
    pub fn status_string(&self) -> String {
        if self.status == NodeStatus::Succeeded && self.unique_id.starts_with("test.")
            || self.unique_id.starts_with("unit_test.")
        {
            match self.num_rows {
                Some(0) => "Passed".to_string(),
                Some(_) => "Failed".to_string(),
                None => "Succeeded".to_string(),
            }
        } else {
            format!("{:?}", self.status)
        }
    }
    pub fn result_status_string(&self) -> String {
        match self.status {
            NodeStatus::Succeeded => {
                if self.unique_id.starts_with("test.") || self.unique_id.starts_with("unit_test.") {
                    match self.num_rows {
                        Some(0) => "pass".to_string(),
                        Some(_) => "fail".to_string(),
                        // Using "success" as fallback, though tests should have pass/fail
                        None => "success".to_string(),
                    }
                } else {
                    "success".to_string()
                }
            }
            NodeStatus::Errored => "error".to_string(),
            NodeStatus::SkippedUpstreamReused => "skipped".to_string(),
            NodeStatus::SkippedUpstreamFailed => "skipped".to_string(),
            NodeStatus::ReusedNoChanges => "reused".to_string(),
            NodeStatus::ReusedStillFresh => "reused".to_string(),
            NodeStatus::NoOp => "skipped".to_string(),
        }
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let all_num_rows_none = self.stats.iter().all(|stat| stat.num_rows.is_none());

        // Calculate the maximum length of the unique_id values
        let max_unique_id_len = self
            .stats
            .iter()
            .map(|stat| stat.unique_id.len())
            .max()
            .unwrap_or(0); // Default to 35 if no stats are present

        if all_num_rows_none {
            writeln!(
                f,
                "{:<width$} | {:<9} | {:<10} | {:<10} | {:<10}",
                "Unique ID",
                "Status",
                "Start Time",
                "End Time",
                "Duration",
                width = max_unique_id_len
            )?;
            // Calculate the total width of the line
            let total_width = max_unique_id_len + 3 + 9 + 3 + 10 + 3 + 10 + 3 + 10; // 3 is for the spaces and separators

            writeln!(f, "{}", "-".repeat(total_width))?;
            for stat in &self.stats {
                writeln!(
                    f,
                    "{:<width$} | {:<9} | {:<10} | {:<10} | {:<10}",
                    stat.unique_id,
                    stat.status_string(),
                    Stat::format_time(stat.start_time),
                    Stat::format_time(stat.end_time),
                    format_duration(stat.get_duration()).to_string(),
                    width = max_unique_id_len
                )?;
            }
        } else {
            writeln!(
                f,
                "{:<width$} | {:<9} | {:<8} | {:<10} | {:<10} | {:<10}",
                "Unique ID",
                "Status",
                "Num Rows",
                "Start Time",
                "End Time",
                "Duration",
                width = max_unique_id_len
            )?;
            let total_width = max_unique_id_len + 3 + 9 + 3 + 8 + 3 + 10 + 3 + 10; // 3 is for the spaces and separators

            writeln!(f, "{}", "-".repeat(total_width))?;

            for stat in &self.stats {
                writeln!(
                    f,
                    "{:<width$} | {:<9} | {:<8} | {:<10} | {:<10} | {:<10}",
                    stat.unique_id,
                    stat.status_string(),
                    stat.num_rows.map_or("-".to_string(), |num| num.to_string()),
                    Stat::format_time(stat.start_time),
                    Stat::format_time(stat.end_time),
                    format_duration(stat.get_duration()).to_string(),
                    width = max_unique_id_len
                )?;
            }
        }
        Ok(())
    }
}
