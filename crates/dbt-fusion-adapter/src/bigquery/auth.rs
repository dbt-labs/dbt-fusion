use crate::auth::Auth;
use crate::config::AdapterConfig;
use crate::errors::AdapterResult;
use dbt_xdbc::{database, Backend};

pub(crate) struct BigQueryAuth;

impl Auth for BigQueryAuth {
    fn backend(&self) -> Backend {
        Backend::BigQuery
    }

    fn configure(&self, config: &AdapterConfig) -> AdapterResult<database::Builder> {
        let mut builder = database::Builder::new(self.backend());

        if let Ok(impersonate_principal) = config.get_str("impersonate_service_account") {
            builder.with_named_option(
                dbt_xdbc::bigquery::IMPERSONATE_TARGET_PRINCIPAL,
                impersonate_principal,
            )?;
        }
        Ok(builder)
    }
}
