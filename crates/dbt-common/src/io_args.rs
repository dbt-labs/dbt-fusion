use clap::ValueEnum;
use dbt_serde_yaml::{JsonSchema, Value};
use dbt_telemetry::NodeType;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use std::{
    collections::{BTreeMap, HashSet},
    fmt::{self, Display},
    path::{Path, PathBuf},
    sync::Arc,
};
use strum::EnumIter;
use strum_macros::Display;

use log::LevelFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LocalExecutionBackendKind {
    #[default]
    /// Run models in the current process
    Inline,
    /// Run models in a separate worker process
    Worker,
    // Eventually Service
}

use crate::adapter::AdapterType;
use crate::{
    constants::{DBT_GENERIC_TESTS_DIR_NAME, DBT_SNAPSHOTS_DIR_NAME},
    io_utils::StatusReporter,
    logging::LogFormat,
    node_selector::{
        IndirectSelection, SelectExpression, SelectionCriteria, conjoin_expression,
        parse_model_specifiers,
    },
    pretty_string::BLUE,
    tracing::invocation::with_invocation_mut,
};

// ----------------------------------------------------------------------------------------------
// IO Args
#[derive(Default, Clone)]
pub struct IoArgs {
    pub invocation_id: uuid::Uuid,
    pub show: HashSet<ShowOptions>,
    pub command: String,
    pub in_dir: PathBuf,
    pub out_dir: PathBuf,
    pub log_path: Option<PathBuf>,
    pub otel_file_name: Option<String>,
    pub otel_parquet_file_name: Option<String>,
    pub export_to_otlp: bool,
    pub log_format: LogFormat,
    pub log_level: Option<LevelFilter>,
    pub log_level_file: Option<LevelFilter>,
    pub debug: bool,

    // Flags influencing error/warning behavior
    pub show_all_deprecations: bool,

    /// Optional status reporter for reporting status messages during execution
    pub status_reporter: Option<Arc<dyn StatusReporter>>,
    pub send_anonymous_usage_stats: bool,

    // internal fields
    pub show_timings: bool, // whether to show timings in the status messages
    pub build_cache_url: Option<String>,
    pub build_cache_cas_url: Option<String>,
    pub build_cache_mode: Option<BuildCacheMode>,
    pub beta_use_query_cache: bool,
}
impl IoArgs {
    pub fn is_generated_file(&self, rel_path: &Path) -> bool {
        // Get last component of out_dir (as_os_str returns None if out_dir is empty)
        let out_dir_last = self.out_dir.components().next_back();
        let rel_first = rel_path.components().next();
        out_dir_last == rel_first
    }
}

impl fmt::Debug for IoArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IoArgs")
            .field("invocation_id", &self.invocation_id)
            .field("show", &self.show)
            .field("in_dir", &self.in_dir)
            .field("out_dir", &self.out_dir)
            .field("log_path", &self.log_path)
            .field("otel_file_name", &self.otel_file_name)
            .field("status_reporter", &self.status_reporter.is_some())
            .finish()
    }
}

impl IoArgs {
    /// Given a path, returns a string representation of that path that is
    /// suitable for display in terminal status messages.
    pub fn format_display_path(&self, path: &Path) -> String {
        let in_dir = &self.in_dir;
        let out_dir = &self.out_dir;

        if path.starts_with(in_dir)
            && let Some(relative_path) = diff_paths(path, in_dir)
        {
            return relative_path.to_string_lossy().to_string();
        }
        if path.starts_with(out_dir)
            && let Some(relative_path) = diff_paths(path, out_dir)
        {
            return relative_path.to_string_lossy().to_string();
        }
        if path.is_relative() {
            let target_path = in_dir.join("target").join(path);
            if target_path.exists() {
                return format!("target/{}", path.to_string_lossy());
            }
        }

        path.to_string_lossy().to_string()
    }

    /// This function takes an artifact path, which may either be a workspace
    /// resource, or some generated temp location, and returns a path to its
    /// corresponding location in the workspace
    pub fn map_to_workspace_path(&self, path: &Path, resource_type: NodeType) -> PathBuf {
        if resource_type == NodeType::UnitTest || resource_type == NodeType::Snapshot {
            let special_component_idx = path.components().position(|c| {
                c.as_os_str() == DBT_GENERIC_TESTS_DIR_NAME
                    || c.as_os_str() == DBT_SNAPSHOTS_DIR_NAME
            });
            if let Some(idx) = special_component_idx {
                // FIXME: this is really a hack, the proper thing to do is to have a
                // semantic representation for each artifact that can generate workspace or
                // temporary paths
                self.out_dir
                    .join(path.components().skip(idx).collect::<PathBuf>())
            } else {
                self.out_dir.join(path)
            }
        } else {
            self.in_dir.join(path)
        }
    }

    pub fn should_show(&self, option: ShowOptions) -> bool {
        (self.show.contains(&option) || option == ShowOptions::All)
            // TODO: temporary logic to avoid showing skipped nodes for compile.
            // Should be centralized across all commands, progress message types, and options.
            && (option != ShowOptions::Completed
                || self.command.as_str() != "compile"
                || self.debug)
    }

    /// Returns true if the build cache should be used (read or readwrite mode, or --use-build-cache flag).
    pub fn should_use_build_cache(&self) -> bool {
        self.build_cache_mode
            .map(|c| matches!(c, BuildCacheMode::Read | BuildCacheMode::ReadWrite))
            .unwrap_or_default()
    }

    /// Returns true if the build cache should be saved (write or readwrite mode).
    pub fn should_save_build_cache(&self) -> bool {
        self.build_cache_mode
            .map(|c| matches!(c, BuildCacheMode::Write | BuildCacheMode::ReadWrite))
            .unwrap_or_default()
    }
}
// ----------------------------------------------------------------------------------------------
// System Args
#[derive(Clone, Debug)]
pub struct SystemArgs {
    pub command: String,
    pub io: IoArgs,
    pub from_main: bool,
    pub num_threads: Option<usize>,
    pub target: Option<String>,
}

// ----------------------------------------------------------------------------------------------
// Eval Args
#[derive(Clone, Default)]
pub struct EvalArgs {
    // The command to run
    pub command: String,
    // io
    pub io: IoArgs,
    // The profile directory to load the profiles from
    pub profiles_dir: Option<PathBuf>,
    // The directory to install packages
    pub packages_install_path: Option<PathBuf>,
    // A package to add to deps
    pub add_package: Option<String>,
    // Upgrade deps
    pub upgrade: bool,
    // Generate lock file only
    pub lock: bool,
    // The profile to use
    pub profile: Option<String>,
    // The target within the profile to use for the dbt run
    pub target: Option<String>,
    // Vars to pass to the jinja environment
    pub vars: BTreeMap<String, Value>,
    // Stop as soon as this stage is reached
    pub phase: Phases,
    // Display rows in different formats, this is .to_string on DisplayFormat; we use a string here to break dep. cycle
    pub format: String,
    /// Limiting number of shown rows. None means no limit, run with --limit -1 to remove limit
    pub limit: Option<usize>,
    /// called as bin or as library
    pub from_main: bool,
    /// The number of threads to use
    pub num_threads: Option<usize>,
    /// yaml selector
    pub selector: Option<String>,
    /// Select nodes to operate on
    pub select: Option<SelectExpression>,
    /// Select nodes to exclude from selected nodes
    pub exclude: Option<SelectExpression>,
    /// Indirect selection mode
    pub indirect_selection: Option<IndirectSelection>,
    /// Show output keys
    pub output_keys: Vec<String>,
    /// Resource types to filter by
    pub resource_types: Vec<ClapResourceType>,
    /// Exclude nodes of a specific type
    pub exclude_resource_types: Vec<ClapResourceType>,
    /// Debug flag
    pub debug: bool,
    /// Set log file format, overriding the default and --log-format setting.
    pub log_format_file: Option<LogFormat>,
    /// Set logging format
    pub log_format: LogFormat,
    /// Set minimum log file severity, overriding the default and --log-level setting.
    pub log_level_file: Option<LevelFilter>,
    /// Set minimum severity for console/log file
    pub log_level: Option<LevelFilter>,
    /// Set 'log-path' for the current run, overriding 'DBT_LOG_PATH'.
    pub log_path: Option<PathBuf>,
    /// The output directory for all produced assets
    pub target_path: Option<PathBuf>,
    /// The directory to load the dbt project from
    pub project_dir: Option<PathBuf>,
    /// Suppress all non-error logging to stdout
    pub quiet: bool,
    /// Write JSON artifacts to disk
    pub write_json: bool,
    /// Write a catalog.json file to the target directory
    pub write_catalog: bool,
    /// Show schema on the command line
    pub schema: Vec<JsonSchemaTypes>,

    // -- fields from the private branch
    pub internal_packages_install_path: Option<PathBuf>,
    pub update_deps: bool,
    pub replay: Option<ReplayMode>,
    pub static_analysis: StaticAnalysisKind,
    pub interactive: bool,
    pub check_conformance: bool,
    pub skip_semantic_manifest_validation: bool,
    pub export_saved_queries: bool,
    pub task_cache_url: String,
    pub run_cache_mode: RunCacheMode,
    pub show_scans: bool,
    pub max_depth: usize,
    pub use_fqtn: bool,
    pub skip_unreferenced_table_check: bool,
    pub state: Option<PathBuf>,
    pub defer_state: Option<PathBuf>,
    pub connection: bool,
    pub macro_name: String,
    pub macro_args: BTreeMap<String, Value>,
    pub warn_error: bool,
    pub warn_error_options: BTreeMap<String, Value>,
    pub version_check: bool,
    pub defer: Option<bool>,
    pub fail_fast: bool,
    pub empty: bool,
    pub sample: Option<String>,
    pub full_refresh: bool,
    pub favor_state: bool,
    pub refresh_sources: bool,
    pub send_anonymous_usage_stats: bool,
    pub check_all: bool,
    // todo: temporary, until Sampling is public, maps (source) unique_id to renamed (database, schema, table)
    pub sample_renaming: BTreeMap<String, (String, String, String)>,
    pub local_execution_backend: LocalExecutionBackendKind,
}
impl fmt::Debug for EvalArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvalArgs")
            .field("in_dir", &self.io.in_dir)
            .field("out_dir", &self.io.out_dir)
            .field("profiles_dir", &self.profiles_dir)
            .field("packages_install_path", &self.packages_install_path)
            .field("target", &self.target)
            .field("vars", &self.vars)
            .field("show", &self.io.show)
            .field("stage", &self.phase)
            .field("format", &self.format)
            .field("limit", &self.limit)
            .field("invocation_id", &self.io.invocation_id)
            .field("select", &self.select)
            .field("exclude", &self.exclude)
            .field("command", &self.command)
            .field("from_main", &self.from_main)
            .field("num_threads", &self.num_threads)
            .field("output_keys", &self.output_keys)
            .field("indirect_selection", &self.indirect_selection)
            .finish()
    }
}

pub struct EvalArgsBuilder {
    pub args: EvalArgs,
}

impl EvalArgsBuilder {
    pub fn from_eval_args(args: &EvalArgs) -> Self {
        Self { args: args.clone() }
    }
}

impl EvalArgsBuilder {
    /// Configure additional arguments
    pub fn with_additional(
        self,
        target: String,
        threads: Option<usize>,
        adapter_type: Option<AdapterType>,
    ) -> Self {
        self.with_target(target)
            .with_threads(threads)
            .disable_static_analysis_if_not_supported(adapter_type)
    }

    fn with_target(mut self, target: String) -> Self {
        // Update span info as it is used in telemetry & TUI
        with_invocation_mut(|invocation| {
            if let Some(args) = invocation.eval_args.as_mut() {
                args.target = Some(target.clone());
            };
        });

        self.args.target = Some(target);
        self
    }

    pub fn with_threads(mut self, num_threads: Option<usize>) -> Self {
        // Update span info as it is used in telemetry & TUI
        with_invocation_mut(|invocation| {
            if let Some(args) = invocation.eval_args.as_mut() {
                args.num_threads = num_threads.map(|l| l as u64);
            };
        });

        self.args.num_threads = num_threads;
        self
    }

    /// Disable the static analysis for a specific adapter if the relevant dialect is unsupported.
    /// Otherwise, it's a noop
    pub fn disable_static_analysis_if_not_supported(
        mut self,
        adapter_type: Option<AdapterType>,
    ) -> Self {
        match adapter_type {
            // include adapters that don't support static analysis here
            Some(AdapterType::Salesforce) | None => {
                #[cfg(debug_assertions)]
                {
                    println!(
                        "debug:warning=static analysis for adapter: {:?} is disabled",
                        adapter_type
                    );
                }
                self.args.static_analysis = StaticAnalysisKind::Off;
            }
            _ => {}
        }
        self
    }

    pub fn with_show_scans(mut self, show_scans: bool) -> Self {
        self.args.show_scans = show_scans;
        self
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.args.max_depth = max_depth;
        self
    }

    pub fn with_use_fqtn(mut self, use_fqtn: bool) -> Self {
        self.args.use_fqtn = use_fqtn;
        self
    }

    pub fn build(self) -> EvalArgs {
        self.args
    }
}

impl EvalArgs {
    // this could accept a SelectExpression in case we want to join more complex selections together.
    pub fn set_refined_node_selectors(mut self, predicate: Option<SelectionCriteria>) -> EvalArgs {
        // Convert SelectionCriteria to SelectExpression::Atom first
        let predicate_expr = predicate.map(SelectExpression::Atom);

        self.select = conjoin_expression(self.select.clone(), predicate_expr.clone());
        if self.exclude.is_some() {
            self.exclude = conjoin_expression(self.exclude.clone(), predicate_expr);
        }

        // Update span info as it is used in telemetry & TUI
        with_invocation_mut(|invocation| {
            if let Some(args) = invocation.eval_args.as_mut() {
                args.select = self.select.iter().map(|s| s.to_string()).collect();
                args.exclude = self.exclude.iter().map(|s| s.to_string()).collect();
            };
        });

        self
    }

    pub fn set_schema(mut self, schema: Vec<JsonSchemaTypes>) -> Self {
        self.schema = schema;
        self
    }

    pub fn set_connection(mut self, connection: bool) -> Self {
        self.connection = connection;
        self
    }
}

// ----------------------------------------------------------------------------------------------
// Enums

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default, ValueEnum, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ClapResourceType {
    #[default]
    Model,
    Source,
    Seed,
    Snapshot,
    Test,
    UnitTest,
    Analysis,
    Function,
    SemanticModel,
    Metric,
    SavedQuery,
}

impl Display for ClapResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ClapResourceType::Model => "model",
            ClapResourceType::Source => "source",
            ClapResourceType::Seed => "seed",
            ClapResourceType::Snapshot => "snapshot",
            ClapResourceType::Test => "test",
            ClapResourceType::UnitTest => "unit_test",
            ClapResourceType::Analysis => "analysis",
            ClapResourceType::Function => "function",
            ClapResourceType::SemanticModel => "semantic_model",
            ClapResourceType::Metric => "metric",
            ClapResourceType::SavedQuery => "saved_query",
        };
        write!(f, "{s}")
    }
}

impl From<&ClapResourceType> for NodeType {
    fn from(value: &ClapResourceType) -> Self {
        match value {
            ClapResourceType::Model => NodeType::Model,
            ClapResourceType::Source => NodeType::Source,
            ClapResourceType::Seed => NodeType::Seed,
            ClapResourceType::Snapshot => NodeType::Snapshot,
            ClapResourceType::Test => NodeType::Test,
            ClapResourceType::UnitTest => NodeType::UnitTest,
            ClapResourceType::Analysis => NodeType::Analysis,
            ClapResourceType::Function => NodeType::Function,
            ClapResourceType::SemanticModel => NodeType::SemanticModel,
            ClapResourceType::Metric => NodeType::Metric,
            ClapResourceType::SavedQuery => NodeType::SavedQuery,
        }
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    Ord,
    ValueEnum,
    Display,
    Default,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum Phases {
    Debug, // dbt debug
    Deps,  // dbt deps
    Parse, // dbt parse
    Format,
    Lint,
    Schedule,
    List, // dbt list
    Freshness,
    JinjaCheck, // dbt jinja-check
    Compile,    // dbt compile
    Show,       // dbt show
    Compare,    // dbt compare
    Sample,     // dbt sample
    Lineage,
    RunOperation,
    #[default]
    All,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Hash, Eq, ValueEnum, Display, EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum JsonSchemaTypes {
    Selector,
    Schema,
    Project,
    Profile,
    Telemetry,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    ValueEnum,
    Default,
    EnumIter,
    Display,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum DisplayFormat {
    #[default]
    Table,
    Csv,
    Tsv,
    Json,
    NdJson,
    Yml,
    /// Output nodes as selector strings (e.g. "source:pkg.source_name.table_name")
    Selector,
    /// Output nodes as search names (node.search_name)
    Name,
    /// Output nodes as file paths (node.original_file_path)
    Path,
}

#[derive(Debug, Clone)]
pub enum ReplayMode {
    DbtReplay(PathBuf),
    FsRecord(PathBuf),
    FsReplay(PathBuf),
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    ValueEnum,
    Display,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum Runtime {
    #[default]
    Local,
    Remote,
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    ValueEnum,
    Display,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum StaticAnalysisKind {
    Unsafe,
    Off,
    #[default]
    On,
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    ValueEnum,
    Display,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum StaticAnalysisOffReason {
    ConfiguredOff,
    UnableToFetchSchema,
    NoDownstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BuildCacheMode {
    Read,
    Write,
    #[default]
    ReadWrite,
}

impl FromStr for StaticAnalysisKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unsafe" => Ok(StaticAnalysisKind::Unsafe),
            "off" => Ok(StaticAnalysisKind::Off),
            "on" => Ok(StaticAnalysisKind::On),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, Display, Serialize, Deserialize, ValueEnum, Default)]
pub enum RunCacheMode {
    #[default]
    Noop,
    ReadWrite,
    WriteOnly,
}

impl RunCacheMode {
    pub fn use_cache(&self) -> bool {
        match self {
            RunCacheMode::ReadWrite => true,
            RunCacheMode::WriteOnly => false,
            RunCacheMode::Noop => false,
        }
    }

    pub fn write_cache(&self) -> bool {
        matches!(self, RunCacheMode::ReadWrite | RunCacheMode::WriteOnly)
    }
}

impl FromStr for RunCacheMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "noop" => Ok(RunCacheMode::Noop),
            "read-write" => Ok(RunCacheMode::ReadWrite),
            "write-only" => Ok(RunCacheMode::WriteOnly),
            _ => Err(format!("Invalid RunCacheMode: {s}")),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum, EnumIter)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum ShowOptions {
    Progress,
    ProgressHydrate,
    ProgressParse,
    ProgressRender,
    ProgressAnalyze,
    ProgressRun,
    Completed,
    InputFiles,
    Manifest,
    Schedule,
    Nodes,
    Instructions,
    SourcedSchemas,
    Schema,
    Data,
    Verdict,
    Stats,
    Lineage,
    All,
    None,
    // hidden internal-only:
    RawLineage,
    TaskGraph,
}

impl ShowOptions {
    pub fn title(&self) -> String {
        match self {
            ShowOptions::InputFiles => BLUE.apply_to("Input files").to_string(),
            ShowOptions::Manifest => BLUE.apply_to("Manifest").to_string(),
            ShowOptions::Schedule => BLUE.apply_to("Schedule").to_string(),
            ShowOptions::Instructions => BLUE.apply_to("Instruction").to_string(),
            ShowOptions::SourcedSchemas => BLUE.apply_to("Sourced schemas").to_string(),
            ShowOptions::Nodes => BLUE.apply_to("Selected nodes").to_string(),
            // remark: we don't use this case, but use compile time and runtime stats
            ShowOptions::Stats => BLUE.apply_to("Statistics").to_string(),
            // remark: these come with own titles..
            ShowOptions::Progress
            | ShowOptions::ProgressHydrate
            | ShowOptions::ProgressRun
            | ShowOptions::ProgressParse
            | ShowOptions::ProgressRender
            | ShowOptions::ProgressAnalyze
            | ShowOptions::Schema
            | ShowOptions::Data
            | ShowOptions::Verdict
            | ShowOptions::Lineage
            | ShowOptions::All
            | ShowOptions::RawLineage
            | ShowOptions::TaskGraph
            | ShowOptions::Completed
            | ShowOptions::None => "".to_string(),
        }
    }
}

#[derive(ValueEnum, Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PersistTarget {
    #[default]
    Warehouse,
    Local,
}
impl Display for PersistTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            PersistTarget::Warehouse => "warehouse",
            PersistTarget::Local => "local",
        };
        write!(f, "{s}")
    }
}
// ----------------------------------------------------------------------------------------------
pub fn check_selector(selector: &str) -> Result<String, String> {
    // Convert the single selector to a vector with one element
    let query = vec![selector.to_string()];
    match parse_model_specifiers(&query) {
        Ok(_) => Ok(selector.to_string()),
        Err(e) => Err(e.pretty()),
    }
}

pub fn check_target(filename: &str) -> Result<String, String> {
    let path = Path::new(filename);
    let err = Err(format!(
        "Input file '{filename}' must have .sql, or .yml extension"
    ));
    // TODO check that this test is universal for all inputs...
    if path.is_dir() {
        Ok(filename.to_owned())
    } else if path.is_file() {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("yml") | Some("sql") => Ok(filename.to_owned()),
            Some(_) => err,
            None => err,
        }
    } else {
        err
    }
}

pub fn check_var(vars: &str) -> Result<BTreeMap<String, Value>, String> {
    // Handle empty input
    if vars.trim().is_empty() {
        return Err("Empty vars input is not valid".into());
    }

    // Strip outer quotes if present
    let vars = vars.trim().trim_matches('\'');

    // Check if the input is already wrapped in curly braces
    let yaml_str = if vars.trim().starts_with('{') {
        vars.to_string()
    } else {
        // Handle single key-value pair separated by a colon
        if vars.trim().matches(':').count() != 1 {
            return Err(format!(
                "Invalid key-value pair: '{vars}'. Expected format: 'key: value'."
            ));
        }
        vars.to_string()
    };

    // Try parsing as YAML first
    match dbt_serde_yaml::from_str::<BTreeMap<String, Value>>(&yaml_str) {
        Ok(btree) => {
            // Disallow the '{key:value}' format for flow-style YAML syntax
            // to prevent key:value: None interpretation: https://stackoverflow.com/a/70909331
            for key in btree.keys() {
                if key.contains(':') {
                    return Err(format!(
                        "Invalid key-value pair: '{key}'. Value must start with a space after colon."
                    ));
                }
            }
            Ok(btree)
        }
        Err(_) => {
            // If YAML parsing fails, try JSON
            match serde_json::from_str(&yaml_str) {
                Ok(btree) => Ok(btree),
                Err(_) => Err(
                    "Invalid YAML/JSON format. Expected format: 'key: value' or '{key: value, ..}'. Note both argument forms must be just one shell token"
                        .to_string(),
                ),
            }
        }
    }
}

pub fn check_env_var(vars: &str) -> Result<HashMap<String, String>, String> {
    let config = vars;
    if config.starts_with('{') {
        let yaml_hashmap: Result<HashMap<String, String>, dbt_serde_yaml::Error> =
            dbt_serde_yaml::from_str(config);

        match yaml_hashmap {
            Ok(x) => Ok(x),
            Err(err) => Err(err.to_string()),
        }
    } else {
        let path = Path::new(config);
        if path.is_file() {
            if path.extension().unwrap() == "yml" {
                match fs::read_to_string(path) {
                    Ok(yaml_data) => {
                        let yaml_hashmap: Result<HashMap<String, String>, dbt_serde_yaml::Error> =
                            dbt_serde_yaml::from_str(&yaml_data);

                        match yaml_hashmap {
                            Ok(x) => Ok(x),
                            Err(err) => Err(err.to_string()),
                        }
                    }
                    Err(err) => Err(err.to_string()),
                }
            } else {
                Err("File must have a .yml extension".into())
            }
        } else {
            Err("Value must be a .yml file or a yml string like so: '{ dialect: trino }'".into())
        }
    }
}

pub fn validate_project_name(name: &str) -> Result<String, String> {
    // Check if the name contains only letters, digits, and underscores
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Ok(name.to_string())
    } else {
        Err(format!(
            "{name} is not a valid project name. Only letters, digits and underscore are valid characters in a project name."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_single_var() {
        let result = check_var("key: value").unwrap();
        let expected_result = BTreeMap::from([(
            "key".to_string(),
            dbt_serde_yaml::from_str("value").unwrap(),
        )]);

        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_check_single_bracket_var() {
        let result = check_var("{key: value}").unwrap();
        let expected_result = BTreeMap::from([(
            "key".to_string(),
            dbt_serde_yaml::from_str("value").unwrap(),
        )]);

        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_check_multiple_bracket_var() {
        let result = check_var("{key: value, key2: value2}").unwrap();
        let expected_result = BTreeMap::from([
            (
                "key".to_string(),
                dbt_serde_yaml::from_str("value").unwrap(),
            ),
            (
                "key2".to_string(),
                dbt_serde_yaml::from_str("value2").unwrap(),
            ),
        ]);

        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_check_var_invalid() {
        let invalid_vars = vec![
            "key",                    // Missing colon
            "key:value",              // Missing space after colon
            "key: value:with:colons", // Value with colons
            "{key:value}",            // Flow-style YAML syntax without space after colon
        ];

        for var in invalid_vars {
            assert!(check_var(var).is_err(), "Should have failed: {var}");
        }
    }

    #[test]
    fn test_validate_project_name_valid() {
        let valid_names = vec![
            "my_project",
            "project123",
            "Project_Name",
            "test_project_1",
            "a",
            "project_with_underscores_and_numbers123",
        ];

        for name in valid_names {
            assert_eq!(validate_project_name(name).unwrap(), name);
        }
    }

    #[test]
    fn test_validate_project_name_invalid() {
        let invalid_names = vec![
            "my-cool-project",      // Contains hyphen
            "project with spaces",  // Contains spaces
            "project.with.dots",    // Contains dots
            "project/with/slashes", // Contains slashes
            "project@symbol",       // Contains @ symbol
            "project#hash",         // Contains # symbol
        ];

        for name in invalid_names {
            let result = validate_project_name(name);
            assert!(result.is_err(), "Should have failed: {name}");
            assert_eq!(
                result.unwrap_err(),
                format!(
                    "{name} is not a valid project name. Only letters, digits and underscore are valid characters in a project name."
                )
            );
        }
    }
}
