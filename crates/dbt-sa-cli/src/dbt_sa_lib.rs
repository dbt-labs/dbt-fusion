use crate::dbt_sa_clap::{Cli, Commands, ProjectTemplate, from_main};
use dbt_common::cancellation::CancellationToken;
use dbt_common::create_root_info_span;
use dbt_common::io_utils::checkpoint_maybe_exit;
use dbt_common::tracing::{
    emit::{emit_error_log_from_fs_error, emit_info_log_message},
    invocation::create_invocation_attributes,
    metrics::get_exit_code_from_error_counter,
    FsTraceConfig,
    init_tracing
};
use dbt_init::init;
use dbt_jinja_utils::invocation_args::InvocationArgs;
use dbt_jinja_utils::listener::DefaultJinjaTypeCheckEventListenerFactory;
use dbt_loader::clean::execute_clean_command;
use dbt_schemas::man::execute_man_command;

use dbt_common::io_args::{EvalArgs, EvalArgsBuilder};
use dbt_common::{
    ErrorCode, FsResult,
    constants::{DBT_MANIFEST_JSON, INSTALLING, ERROR, PANIC, VALIDATING},
    fs_err, fsinfo,
    io_args::{Phases, SystemArgs},
    logging::init_logger,
    pretty_string::{GREEN, RED},

    show_progress, show_result_with_default_title, stdfs,
    tracing::span_info::record_span_status,
};

use dbt_schemas::schemas::Nodes;
use dbt_schemas::state::{
    GetColumnsInRelationCalls, GetRelationCalls, Macros, PatternedDanglingSources,
};
#[allow(unused_imports)]
use git_version::git_version;

use dbt_schemas::schemas::manifest::build_manifest;
use tracing::Instrument;

use std::process::ExitCode;
use std::sync::Arc;

use dbt_loader::{args::LoadArgs, load};
use dbt_parser::{args::ResolveArgs, resolver::resolve};

use serde_json::to_string_pretty;

use std::io::{self, Write};

// Vars
const FS_DEFAULT_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Maximum number of threads used for running blocking operations (based on the tokio runtime
/// default).
///
/// These threads are used mostly for blocking I/O operations, so they don't really
/// consume CPU resources. That's why we can afford and should have a lot of them.
const FS_DEFAULT_MAX_BLOCKING_THREADS: usize = 512;

// ------------------------------------------------------------------------------------------------

pub async fn execute_fs(
    system_arg: SystemArgs,
    cli: Cli,
    token: CancellationToken,
) -> FsResult<i32> {
    // Resolve EvalArgs from SystemArgs and Cli. This will create out folders,
    // for commands that need it and canonicalize the paths. May error on invalid paths.
    let eval_arg = cli.to_eval_args(system_arg)?;

    init_logger((&eval_arg.io).into()).expect("Failed to initialize logger");

    // Create the Invocation span as a new root
    let invocation_span = create_root_info_span(create_invocation_attributes("dbt-sa", &eval_arg));

    let result = do_execute_fs(&eval_arg, cli, token)
        .instrument(invocation_span.clone())
        .await;

    // Record span run result
    match result {
        Ok(0) => record_span_status(&invocation_span, None),
        Ok(_) => record_span_status(&invocation_span, Some("Executed with errors")),
        Err(ref e) => record_span_status(&invocation_span, Some(format!("Error: {e}").as_str())),
    };

    result
}

#[allow(clippy::cognitive_complexity)]
async fn do_execute_fs(eval_arg: &EvalArgs, cli: Cli, token: CancellationToken) -> FsResult<i32> {
    if let Commands::Man(_) = &cli.command {
        return match execute_man_command(eval_arg).await {
            Ok(code) => Ok(code),
            Err(e) => {
                emit_error_log_from_fs_error(&e, eval_arg.io.status_reporter.as_ref());

                Ok(1)
            }
        };
    } else if let Commands::Init(init_args) = &cli.command {
        // Handle init command
        use dbt_init::init::run_init_workflow;

        show_progress!(
            &eval_arg.io,
            fsinfo!(
                INSTALLING.into(),
                "dbt project and profile setup".to_string()
            )
        );

        let project_name = if init_args.project_name == "jaffle_shop" {
            None // Use default
        } else {
            Some(init_args.project_name.clone())
        };

        let project_template = match init_args.sample {
            ProjectTemplate::JaffleShop => init::assets::ProjectTemplateAsset::JaffleShop,
            ProjectTemplate::MomsFlowerShop => init::assets::ProjectTemplateAsset::MomsFlowerShop,
        };

        match run_init_workflow(
            project_name,
            init_args.skip_profile_setup,
            init_args.common_args.profile.clone(), // Get profile from common args
            &project_template,
        )
        .await
        {
            Ok(()) => {
                // If profile setup was not skipped, run debug to validate credentials
                if init_args.skip_profile_setup {
                    return Ok(0);
                }

                emit_info_log_message(format!(
                    "{} profile inputs, adapters, and connection\n", // Add empty line for spacing
                    GREEN.apply_to(VALIDATING)
                ));
            }
            Err(e) => {
                emit_error_log_from_fs_error(&e, eval_arg.io.status_reporter.as_ref());

                return Ok(1);
            }
        }
    }

    // Handle project specific commands
    match execute_setup_and_all_phases(eval_arg, cli, &token).await {
        Ok(code) => Ok(code),
        Err(e) => {
            emit_error_log_from_fs_error(&e, eval_arg.io.status_reporter.as_ref());

            Ok(1)
        }
    }
}

#[allow(clippy::cognitive_complexity)]
async fn execute_setup_and_all_phases(
    eval_arg: &EvalArgs,
    cli: Cli,
    token: &CancellationToken,
) -> FsResult<i32> {
    // Header ..
    // current_exe errors when running in dbt-cloud
    // https://github.com/rust-lang/rust/issues/46090
    #[cfg(debug_assertions)]
    {
        use chrono::{DateTime, Local};
        use dbt_common::constants::DBT_SA_CLI;
        use std::env;
        let exe_path = env::current_exe()
            .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to get current exe path: {}", e))?;
        let modified_time = stdfs::last_modified(&exe_path)?;

        // Convert SystemTime to DateTime<Local>
        let datetime: DateTime<Local> = DateTime::from(modified_time);
        let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
        let build_time = if eval_arg.from_main {
            let git_hash = git_version!(fallback = "unknown");
            format!(
                "{} ({} {})",
                env!("CARGO_PKG_VERSION"),
                git_hash,
                formatted_time
            )
        } else {
            "".to_string()
        };
        let info = fsinfo!(DBT_SA_CLI.into(), build_time);
        show_progress!(&eval_arg.io, info);
    }

    // Check if the command is `Clean`
    if let Commands::Clean(ref clean_args) = cli.command {
        match execute_clean_command(eval_arg, &clean_args.files, token).await {
            Ok(code) => Ok(code),
            Err(e) => {
                emit_error_log_from_fs_error(&e, eval_arg.io.status_reporter.as_ref());

                Ok(1)
            }
        }
    } else {
        // Execute all steps of all other commands, if any throws an error we stop
        match execute_all_phases(eval_arg, &cli, token).await {
            Ok(code) => Ok(code),
            Err(e) => {
                emit_error_log_from_fs_error(&e, eval_arg.io.status_reporter.as_ref());

                Ok(1)
            }
        }
    }
}

#[allow(clippy::cognitive_complexity)]
async fn execute_all_phases(
    arg: &EvalArgs,
    _cli: &Cli,
    token: &CancellationToken,
) -> FsResult<i32> {
    // Loads all .yml files + collects all included files
    let load_args = LoadArgs::from_eval_args(arg);
    let invocation_args = InvocationArgs::from_eval_args(arg);
    let (dbt_state, _dbt_cloud_config) = load(&load_args, &invocation_args, token).await?;

    let arg = EvalArgsBuilder::from_eval_args(arg)
        .with_additional(
            dbt_state.dbt_profile.target.to_string(),
            dbt_state.dbt_profile.threads,
            dbt_state.dbt_profile.db_config.adapter_type_if_supported(),
        )
        .build();

    show_result_with_default_title!(&arg.io, ShowOptions::InputFiles, &dbt_state.to_string());

    // This also exits the init command b/c init `to_eval_args` sets the phase to debug
    if let Some(exit_code) = checkpoint_maybe_exit(&arg, Phases::Debug) {
        return Ok(exit_code);
    }
    if let Some(exit_code) = checkpoint_maybe_exit(&arg, Phases::Deps) {
        return Ok(exit_code);
    }

    // Parses (dbt parses) all .sql files with execute == false
    let resolve_args = ResolveArgs::try_from_eval_args(&arg)?;
    let invocation_args = InvocationArgs::from_eval_args(&arg);
    let (resolved_state, _jinja_env) = resolve(
        &resolve_args,
        &invocation_args,
        Arc::new(dbt_state),
        Macros::default(),
        Nodes::default(),
        GetRelationCalls::default(),
        GetColumnsInRelationCalls::default(),
        PatternedDanglingSources::default(),
        token,
        Arc::new(DefaultJinjaTypeCheckEventListenerFactory::default()), // TODO: use option<>
    )
    .await?;

    let dbt_manifest = build_manifest(&arg.io.invocation_id.to_string(), &resolved_state);

    if arg.write_json {
        let dbt_manifest_path = arg.io.out_dir.join(DBT_MANIFEST_JSON);
        stdfs::create_dir_all(dbt_manifest_path.parent().unwrap())?;
        stdfs::write(dbt_manifest_path, serde_json::to_string(&dbt_manifest)?)?;
    }

    show_result_with_default_title!(
        &arg.io,
        ShowOptions::Manifest,
        to_string_pretty(&dbt_manifest)?
    );

    Ok(get_exit_code_from_error_counter())
}


pub fn run_with_args(cli: Cli, token: CancellationToken) -> ExitCode {
    let arg = from_main(&cli);

    // Init tracing
    let mut telemetry_handle = match init_tracing(FsTraceConfig::new_from_io_args(
        arg.command,
        cli.project_dir().as_ref(),
        cli.target_path().as_ref(),
        &arg.io,
        "dbt-sa",
    )) {
        Ok(handle) => handle,
        Err(e) => {
            let msg = e.to_string();
            print_trimmed_error(msg);
            std::process::exit(1);
        }
    };

    // XXX: when dbt-sa-cli and dbt-cli are unified, this will be the event emitter
    // we inject into execute_fs. This instantiation is here as proof that our build
    // and dependencies are configured such that private .proto files aren't linked
    // into the SA code.
    let _event_emitter = vortex_events::fusion_sa_event_emitter(false);

    // Setup tokio runtime and set stack-size to 8MB
    // DO NOT USE Rayon, it is not compatible with Tokio

    let tokio_rt = match arg.num_threads {
        Some(1) => {
            // Simiulate single-threaded runtime
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(FS_DEFAULT_STACK_SIZE)
                .worker_threads(1)
                .max_blocking_threads(1)
                .build()
                .expect("failed to initialize 'single-threaded' tokio runtime")
        }
        // Uncomment this if you want to limit the number of threads in multi-threaded runtime
        // Some(num_threads) if num_threads > 1 => {
        //     // Multi-threaded runtime: limit to num_threads
        //     tokio::runtime::Builder::new_multi_thread()
        //         .enable_all()
        //         .worker_threads(num_threads)
        //         .max_blocking_threads(FS_DEFAULT_MAX_BLOCKING_THREADS)
        //         .thread_stack_size(FS_DEFAULT_STACK_SIZE)
        //         .build()
        //         .expect("failed to initialize multi-threaded tokio runtime")
        // }
        _ => {
            // Multi-threaded runtime: use default (max parallelism)
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .max_blocking_threads(FS_DEFAULT_MAX_BLOCKING_THREADS)
                .thread_stack_size(FS_DEFAULT_STACK_SIZE)
                .build()
                .expect("failed to initialize default multi-threaded tokio runtime")
        }
    };

    // If execution panics, exit with a status 2 (but not if RUST_BACKTRACE is
    // set to 1, in which case we want to see the backtrace):
    if std::env::var("RUST_BACKTRACE").unwrap_or_default() != "1" {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{} {}", RED.apply_to(PANIC), info);
            let _ = io::stdout().flush();
            let _ = io::stderr().flush();

            std::process::exit(2);
        }));
    }

    // Run within the process span
    let future = Box::pin(execute_fs(arg, cli, token));

    let result = tokio_rt.block_on(async { tokio_rt.spawn(future).await.unwrap() });

    // Shut down telemetry
    for err in telemetry_handle.shutdown() {
        eprintln!("{}", err.pretty());
    }

    // Remove the panic hook
    let _ = std::panic::take_hook();

    // Handle regular execution
    match result {
        Ok(code) => {
            // If exec succeeds, exit with status 0 or 1
            // for 1 it is assumed that the  error was already printed)
            assert!(code == 0 || code == 1);
            ExitCode::from(code as u8)
        }
        Err(_err) => {
            // If any step fails, assume error is already printed, just exit with a status 1
            // show_progress_exit!(arg, start);
            ExitCode::from(1)
        }
    }

}

pub fn print_trimmed_error(msg: String) {
    let mut stderr = io::stderr();

    let mut lines = msg.lines();
    let mut command = String::new();

    for line in lines.by_ref() {
        if let Some(rest) = line.strip_prefix("error:") {
            let _ = write!(stderr, "{}:", RED.apply_to("error"));
            let _ = writeln!(stderr, "{rest}");
        } else if let Some(rest) = line.trim_start().strip_prefix("tip:") {
            let prefix = if line.starts_with("tip:") { "" } else { "  " };
            let _ = write!(stderr, "{}{}", prefix, GREEN.apply_to("tip"));
            let _ = writeln!(stderr, ":{rest}");
        } else if line.trim().starts_with("Usage:") {
            //let command = drop "Usage:"; take everything until the first '<'; trim
            command = line.strip_prefix("Usage:").unwrap_or(line).to_string();
            command = command
                .split_once('<')
                .unwrap_or(("", ""))
                .0
                .trim()
                .to_string();
            break; // stop before dumping giant usage block
        } else {
            let _ = writeln!(stderr, "{line}");
        }
    }

    // Always print this footer
    let _ = writeln!(stderr, "\nFor more information, try '{command} --help'.");
}