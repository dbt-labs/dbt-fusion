use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;
use dbt_common::cancellation::CancellationTokenSource;
use dbt_sa_lib::dbt_sa_clap::Cli;
use dbt_sa_lib::dbt_sa_lib::{print_trimmed_error, run_with_args};
use pyo3::exceptions::{PySystemExit, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString};

#[allow(non_camel_case_types, unused)]
#[pyclass(unsendable)]
struct dbtResult {
    success: bool,
    exception: Option<PyErr>,
    result: Option<PyObject>,
}

impl dbtResult {
    fn from_py_err(err: PyErr) -> Self {
        dbtResult {
            success: false,
            exception: Some(err),
            result: None,
        }
    }
}

#[allow(non_camel_case_types)]
#[pyclass(unsendable)]
struct dbtRunner;

#[pymethods]
impl dbtRunner {
    #[new]
    fn new() -> Self {
        dbtRunner {}
    }

    /// Invoke dbt
    fn invoke(&self, args: &Bound<'_, PyList>) -> dbtResult {
        let mut str_args: Vec<String> = Vec::new();

        // We are responsible for adding the dbt command at the beginning to make parsing work correctly
        str_args.push("dbt".to_owned());

        for (idx, item) in args.iter().enumerate() {
            // Validate all items passed into list are strings
            if !item.is_instance_of::<PyString>() {
                let err_str = format!(
                    "invoke() must recieve a list of strings. The object at position {} in the list is not a string.",
                    idx
                );

                let res = dbtResult::from_py_err(PyTypeError::new_err(err_str));

                return res;
            }

            // Extract each item into a Rust String
            let s = match item.extract::<String>() {
                Ok(s) => s,
                Err(e) => {
                    let res = dbtResult {
                        success: false,
                        exception: Some(e),
                        result: None,
                    };

                    return res;
                }
            };

            // Prevent users from adding the dbt command itself, we handle that
            if idx == 0 && s.trim() == "dbt" {
                let err_str = "invoke() should only recieve subcommands and arguments for the dbt CLI command, don't pass in `dbt` at the beginning - e.g. [\"init\"], not [\"dbt\", \"init\"]";

                let res = dbtResult::from_py_err(PyValueError::new_err(err_str));

                return res;
            }

            str_args.push(s);
        }

        let cst = CancellationTokenSource::new();
        let token = cst.token();

        let cli = match Cli::try_parse_from(str_args.iter()) {
            Ok(cli) => cli,
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

        let status_code = run_with_args(cli, token);

        if status_code != ExitCode::SUCCESS {
            let res = dbtResult::from_py_err(PySystemExit::new_err(format!(
                "dbt exited with code {:?}.",
                status_code
            )));

            return res;
        }

        dbtResult {
            success: true,
            exception: None,
            result: None,
        }
    }
}

#[pymodule]
fn dbt_fusion(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<dbtRunner>()?;
    Ok(())
}
