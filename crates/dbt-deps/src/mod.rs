mod steps;

mod github_client;
mod hub_client;
pub mod package_listing;
pub mod private_package;
pub mod semver;
pub mod types;
pub mod utils;

use dbt_common::fsinfo;
use dbt_common::io_args::IoArgs;
use dbt_common::{
    constants::{FETCHING, INSTALLING, LOADING},
    err, show_progress, stdfs, ErrorCode, FsResult,
};
use dbt_jinja_utils::{jinja_environment::JinjaEnvironment, phases::load::RenderSecretScope};
use dbt_schemas::schemas::packages::{DbtPackagesLock, UpstreamProject};
use hub_client::{HubClient, DBT_HUB_URL};
use std::{collections::BTreeMap, path::Path};
use steps::{
    compute_package_lock, install_packages, load_dbt_packages, try_load_valid_dbt_packages_lock,
};

#[allow(clippy::cognitive_complexity)]
/// Loads and installs packages, and returns the packages lock and the dependencies map
pub async fn get_or_install_packages(
    io: &IoArgs,
    env: &mut JinjaEnvironment<'static>,
    packages_install_path: &Path,
    install_deps: bool,
    vars: BTreeMap<String, dbt_serde_yaml::Value>,
) -> FsResult<(DbtPackagesLock, Vec<UpstreamProject>)> {
    let mut hub_registry = HubClient::new(DBT_HUB_URL);

    let package_render_scope = RenderSecretScope::new(env, vars);
    let (package_def, package_yml_name) = load_dbt_packages(io, &io.in_dir)?;

    // Store projects for later use if package_def exists
    let projects = if let Some(ref packages) = package_def {
        packages.projects.clone()
    } else {
        Vec::new()
    };

    let dbt_packages_lock = if let Some(ref dbt_packages) = package_def {
        if let Some(dbt_packages_lock) =
            try_load_valid_dbt_packages_lock(io, packages_install_path, dbt_packages)?
        {
            show_progress!(io, fsinfo!(LOADING.into(), package_yml_name.to_string()));
            dbt_packages_lock
        } else {
            show_progress!(io, fsinfo!(FETCHING.into(), package_yml_name.to_string()));
            compute_package_lock(
                io,
                package_render_scope.jinja_env,
                &mut hub_registry,
                dbt_packages,
            )
            .await?
        }
    } else {
        DbtPackagesLock::default()
    };

    if install_deps && !dbt_packages_lock.packages.is_empty() {
        // Write out the lock file
        show_progress!(io, fsinfo!(INSTALLING.into(), "packages".to_string()));
        // check if the packages install path exists
        if !packages_install_path.exists() {
            // Create the directory
            stdfs::create_dir_all(packages_install_path).unwrap();
        }
        install_packages(
            io,
            &mut hub_registry,
            package_render_scope.jinja_env,
            &dbt_packages_lock,
            packages_install_path,
        )
        .await?;
    }

    let mut missing_packages = Vec::new();
    for package in dbt_packages_lock.packages.iter() {
        if !packages_install_path.join(package.package_name()).exists() {
            missing_packages.push(package.package_name());
        }
    }
    let mut missing_packages_after_auto_install = Vec::new();

    // Auto install missing packages if not installing deps
    if !missing_packages.is_empty() {
        if !install_deps {
            // check if the packages install path exists
            if !packages_install_path.exists() {
                // Create the directory
                stdfs::create_dir_all(packages_install_path).unwrap();
            }
            // try to install the missing packages
            show_progress!(io, fsinfo!(INSTALLING.into(), "packages".to_string()));
            install_packages(
                io,
                &mut hub_registry,
                package_render_scope.jinja_env,
                &dbt_packages_lock,
                packages_install_path,
            )
            .await?;
            for package in dbt_packages_lock.packages.iter() {
                if !packages_install_path.join(package.package_name()).exists() {
                    missing_packages_after_auto_install.push(package.package_name());
                }
            }
            if !missing_packages_after_auto_install.is_empty() {
                return err!(
                    ErrorCode::InvalidConfig,
                    "The following packages are missing from the packages install path: {:?}. Check you package definition and run 'fs deps' to install the missing packages.",
                    missing_packages_after_auto_install.join(", ")
                );
            }
        } else {
            return err!(
                ErrorCode::InvalidConfig,
                "The following packages are missing from the packages install path: {:?}. Check you package definition and run 'fs deps' to install the missing packages.",
                missing_packages.join(", ")
            );
        }
    }

    Ok((dbt_packages_lock, projects))
}
