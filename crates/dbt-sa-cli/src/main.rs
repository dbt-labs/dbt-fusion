use clap::Parser;
use clap::error::ErrorKind;

use dbt_common::cancellation::CancellationTokenSource;
use dbt_sa_lib::dbt_sa_clap::Cli;
use dbt_sa_lib::dbt_sa_lib::{print_trimmed_error, run_with_args};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cst = CancellationTokenSource::new();
    // TODO(felipecrv): cancel the token (through the cst) on Ctrl-C
    let token = cst.token();

    let cli = match Cli::try_parse() {
        Ok(cli) => {
            // Continue as normal
            cli
        }
        Err(e) => {
            if e.kind() == ErrorKind::UnknownArgument {
                // todo make this for more than just unknown arguments
                // Only show the actual error message
                let msg = e.to_string(); // includes both "error:" and possibly "tip:"
                print_trimmed_error(msg); // prints to stderr
                std::process::exit(1);
            } else {
                // For other errors, show full help as usual
                e.exit();
            }
        }
    };

    run_with_args(cli, token)
}
