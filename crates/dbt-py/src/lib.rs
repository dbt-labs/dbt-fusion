use std::process::ExitCode;

use clap::Parser;
use dbt_common::cancellation::CancellationTokenSource;
use dbt_sa_lib::dbt_sa_clap::Cli;
use dbt_sa_lib::dbt_sa_lib::run_with_args;
use pyo3::exceptions::{PySystemExit, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString};

#[allow(non_camel_case_types, unused)]
#[derive(Debug)]
#[pyclass]
struct dbtResult {
    #[pyo3(get)]
    success: bool,
    #[pyo3(get)]
    exception: Option<PyErr>,
    #[pyo3(get)]
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

#[pymethods]
impl dbtResult {
    fn __repr__(&self) -> String {
        let success_str = match &self.success {
            true => "True".to_owned(),
            false => "False".to_owned()
        };

        let exception_str = match &self.exception {
            Some(e) => format!("\"{}\"", e.to_string()),
            None => "None".to_owned()
        };

        let result_str = match &self.result {
            Some(res) => res.to_string(),
            None => "None".to_owned()
        };

        format!("dbtResult {{ success: {}, exception: {}, result: {} }}", success_str, exception_str, result_str)
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }
}

#[allow(non_camel_case_types)]
#[pyclass]
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

                return dbtResult::from_py_err(PyTypeError::new_err(err_str));
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

                return dbtResult::from_py_err(PyValueError::new_err(err_str));
            }

            str_args.push(s);
        }

        let cst = CancellationTokenSource::new();
        let token = cst.token();

        let cli = match Cli::try_parse_from(str_args.iter()) {
            Ok(cli) => cli,
            Err(e) => {
                return dbtResult::from_py_err(PyValueError::new_err(e.to_string()));
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
    m.add_class::<dbtResult>()?;
    Ok(())
}
