use dbt_common::io_args::IoArgs;
use dbt_common::{fs_err, fsinfo, show_progress, show_warning, ErrorCode, FsResult};
use dbt_schemas::schemas::project::ProjectDbtCloudConfig;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{
    policies::ExponentialBackoff as RetryExponentialBackoff, RetryTransientMiddleware,
};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::utils::load_raw_yml;

const DOWNLOAD_INTERVAL: u64 = 3600; // 1 hour
const MAX_CLIENT_RETRIES: u32 = 3;

/// Downloads manifest from dbt Cloud if available and not recently cached
#[allow(clippy::cognitive_complexity)]
pub async fn download_manifest_from_cloud(
    dbt_cloud_config: &Option<ProjectDbtCloudConfig>,
    io: &IoArgs,
) -> FsResult<Option<PathBuf>> {
    // Check if dbt cloud config exists and has project_id
    let project_id = match dbt_cloud_config {
        Some(config) => match &config.project_id {
            Some(id) => id,
            None => return Ok(None),
        },
        None => return Ok(None),
    };

    // Create directory for manifest
    let default_dir = io.out_dir.join("dbt_cloud_defer");
    std::fs::create_dir_all(&default_dir)?;

    let manifest_path = default_dir.join("manifest.json");
    let info_path = default_dir.join("manifest.info");

    // Check if manifest already exists and is recent (less than an hour old)
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to get system time: {}", e))?
        .as_secs();

    if manifest_path.exists() && info_path.exists() {
        // Read the timestamp from info file
        if let Ok(timestamp_str) = std::fs::read_to_string(&info_path) {
            if let Ok(last_download_time) = timestamp_str.trim().parse::<u64>() {
                // If less than an hour has passed, use existing manifest
                if now - last_download_time <= DOWNLOAD_INTERVAL {
                    return Ok(Some(default_dir));
                }
            }
        }
    }

    // Get home directory
    let home_dir = match dirs::home_dir() {
        Some(dir) => dir,
        None => {
            return Err(fs_err!(
                ErrorCode::IoError,
                "Could not determine home directory"
            ));
        }
    };

    // Check if dbt_cloud.yml exists
    let dbt_cloud_config_path = home_dir.join(".dbt").join("dbt_cloud.yml");
    if !dbt_cloud_config_path.exists() {
        return Ok(None);
    }

    // Load dbt cloud configuration
    let dbt_cloud_config: dbt_schemas::schemas::DbtCloudConfig =
        load_raw_yml(&dbt_cloud_config_path)?;
    // Check if defer_env_id is specified and show warning
    if let Some(defer_env_id) = &dbt_cloud_config.context.defer_env_id {
        show_progress!(
                io,
                fsinfo!("WARNING".into(), format!("defer_env_id '{}' is specified but not yet supported - using prod/staging environment", defer_env_id))
            );
    }

    let project = match dbt_cloud_config.get_project_by_id(project_id.to_string().as_str()) {
        Some(p) => p,
        None => return Ok(None), // Project not found in dbt_cloud.yml, skip download
    };

    let (account_id, account_host, token) = (
        project.account_id.clone(),
        project.account_host.clone(),
        project.token_value.clone(),
    );

    // Construct API URL to get presigned link
    let url = format!(
        "https://{account_host}/api/private/accounts/{account_id}/projects/{project_id}/manifest/latest/"
    );

    // Log download attempt
    show_progress!(
        io,
        fsinfo!("DOWNLOADING".into(), "deferral manifest".to_string())
    );

    // First request to get presigned URL
    let retry_policy =
        RetryExponentialBackoff::builder().build_with_max_retries(MAX_CLIENT_RETRIES);
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();
    let response = client
        .get(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to execute HTTP request: {}", e))?;

    if !response.status().is_success() {
        show_warning!(
            io,
            fs_err!(
                ErrorCode::Generic,
                "Failed to request deferral manifest from the dbt platform for project {}, continuing without deferral. HTTP status {}{}",
                project_id,
                response.status(),
                if let Ok(text) = response.text().await {
                    format!(" - {text}")
                } else {
                    "".to_string()
                }
            )
        );
        return Ok(None);
    }

    // Parse response to get presigned URL
    let response_json: serde_json::Value = response.json().await.map_err(|e| {
        fs_err!(
            ErrorCode::IoError,
            "Failed to parse response as JSON: {}",
            e
        )
    })?;

    let presigned_url = response_json["data"]["manifest_href"]
        .as_str()
        .ok_or_else(|| {
            fs_err!(
                ErrorCode::IoError,
                "Failed to extract manifest_href from response"
            )
        })?;

    // Download manifest from presigned URL
    let manifest_response = client
        .get(presigned_url)
        .send()
        .await
        .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to download manifest: {}", e))?;

    if !manifest_response.status().is_success() {
        show_warning!(
            io,
            fs_err!(
                ErrorCode::Generic,
                "Failed to download deferral manifest from the dbt platform for project {}, continuing without deferral. HTTP status {}{}",
                project_id,
                manifest_response.status(),
                if let Ok(text) = manifest_response.text().await {
                    format!(" - {text}")
                } else {
                    "".to_string()
                }
            )
        );
        return Ok(None);
    }

    // Save manifest to file
    let manifest_bytes = manifest_response
        .bytes()
        .await
        .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to read manifest body: {}", e))?;

    std::fs::write(&manifest_path, manifest_bytes).map_err(|e| {
        fs_err!(
            ErrorCode::IoError,
            "Failed to write manifest to file: {}",
            e
        )
    })?;

    // Write timestamp to info file
    std::fs::write(&info_path, now.to_string())
        .map_err(|e| fs_err!(ErrorCode::IoError, "Failed to write info file: {}", e))?;

    // Log successful download
    show_progress!(
        io,
        fsinfo!(
            "DOWNLOADED".into(),
            format!("deferral manifest to {}", manifest_path.display())
        )
    );

    Ok(Some(default_dir))
}
