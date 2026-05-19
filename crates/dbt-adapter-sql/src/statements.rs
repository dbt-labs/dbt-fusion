use dbt_adapter_core::AdapterType;

use crate::tokenizer::{Token, Tokenizer};

pub fn is_update_statement(sql: &str, adapter_type: AdapterType) -> bool {
    match adapter_type {
        AdapterType::ClickHouse => {
            first_sql_token(sql).is_some_and(|token| !is_clickhouse_read_statement_token(token))
        }
        AdapterType::Bigquery
        | AdapterType::Snowflake
        | AdapterType::Databricks
        | AdapterType::Redshift
        | AdapterType::Postgres
        | AdapterType::Salesforce
        | AdapterType::Spark
        | AdapterType::DuckDB
        | AdapterType::Fabric
        | AdapterType::Exasol
        | AdapterType::Starburst
        | AdapterType::Athena
        | AdapterType::Trino
        | AdapterType::Datafusion
        | AdapterType::Dremio
        | AdapterType::Oracle => false,
    }
}

fn first_sql_token(sql: &str) -> Option<&str> {
    let sql = trim_leading_sql_comments(sql);
    let mut tokenizer = Tokenizer::new(sql);
    match tokenizer.next() {
        Some(Token::Word(token)) => Some(token),
        _ => None,
    }
}

fn trim_leading_sql_comments(mut sql: &str) -> &str {
    loop {
        let trimmed = sql.trim_start_matches(char::is_whitespace);
        if let Some(rest) = trimmed.strip_prefix("--") {
            let Some((_, rest)) = rest.split_once('\n') else {
                return "";
            };
            sql = rest;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("/*") {
            let Some((_, rest)) = rest.split_once("*/") else {
                return "";
            };
            sql = rest;
            continue;
        }
        return trimmed;
    }
}

fn is_clickhouse_read_statement_token(token: &str) -> bool {
    token.eq_ignore_ascii_case("SELECT")
        || token.eq_ignore_ascii_case("WITH")
        || token.eq_ignore_ascii_case("SHOW")
        || token.eq_ignore_ascii_case("DESCRIBE")
        || token.eq_ignore_ascii_case("EXPLAIN")
        || token.eq_ignore_ascii_case("EXISTS")
        || token.eq_ignore_ascii_case("CHECK")
}

#[cfg(test)]
mod tests {
    use dbt_adapter_core::AdapterType;

    use super::is_update_statement;

    #[test]
    fn clickhouse_update_statement_classification_uses_sql_tokenizer() {
        assert!(is_update_statement(
            "/* dbt */\nCREATE TABLE foo (id Int32)",
            AdapterType::ClickHouse,
        ));
        assert!(is_update_statement(
            "-- dbt\nINSERT INTO foo VALUES (1)",
            AdapterType::ClickHouse,
        ));
        assert!(!is_update_statement(
            "/* dbt */\nSELECT 1",
            AdapterType::ClickHouse,
        ));
        assert!(!is_update_statement("SHOW TABLES", AdapterType::ClickHouse));
        assert!(!is_update_statement(
            "CREATE TABLE foo (id int)",
            AdapterType::DuckDB,
        ));
    }
}
