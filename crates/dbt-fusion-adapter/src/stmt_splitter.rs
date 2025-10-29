use dbt_frontend_common::dialect::Dialect;
use std::fmt::Debug;

/// Trait for SQL statement splitting functionality
pub trait StmtSplitter: Send + Sync + Debug {
    /// Split a SQL string into individual statements
    ///
    /// The implementation should:
    /// - Split the SQL into individual statements based on delimiters
    /// - Handle dialect-specific syntax correctly
    fn split(&self, sql: &str, dialect: Dialect) -> Vec<String>;

    /// Determine if a SQL string is either empty or only contains a comment
    fn is_empty(&self, sql: &str, dialect: Dialect) -> bool;
}

/// Naive implementation of StmtSplitter
///
/// Used as a placeholder until a more robust solution is made available
/// to this crate.
#[derive(Debug)]
pub struct NaiveStmtSplitter;

impl StmtSplitter for NaiveStmtSplitter {
    fn split(&self, sql: &str, _dialect: Dialect) -> Vec<String> {
        // Improved SQL splitter that properly handles strings and comments
        let mut statements = Vec::new();
        let mut current = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let mut chars = sql.chars().peekable();
        
        while let Some(ch) = chars.next() {
            match ch {
                // Handle single quotes (string literals)
                '\'' if !in_double_quote && !in_line_comment && !in_block_comment => {
                    in_single_quote = !in_single_quote;
                    current.push(ch);
                }
                // Handle double quotes (identifiers)
                '"' if !in_single_quote && !in_line_comment && !in_block_comment => {
                    in_double_quote = !in_double_quote;
                    current.push(ch);
                }
                // Handle line comments
                '-' if !in_single_quote && !in_double_quote && !in_block_comment => {
                    if chars.peek() == Some(&'-') {
                        chars.next(); // consume second '-'
                        in_line_comment = true;
                        current.push_str("--");
                    } else {
                        current.push(ch);
                    }
                }
                // Handle block comments start
                '/' if !in_single_quote && !in_double_quote && !in_line_comment => {
                    if chars.peek() == Some(&'*') {
                        chars.next(); // consume '*'
                        in_block_comment = true;
                        current.push_str("/*");
                    } else {
                        current.push(ch);
                    }
                }
                // Handle block comments end
                '*' if in_block_comment => {
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume '/'
                        in_block_comment = false;
                        current.push_str("*/");
                    } else {
                        current.push(ch);
                    }
                }
                // Handle line endings (end line comments)
                '\n' | '\r' => {
                    if in_line_comment {
                        in_line_comment = false;
                    }
                    current.push(ch);
                }
                // Handle semicolons (statement separators)
                ';' if !in_single_quote && !in_double_quote && !in_line_comment && !in_block_comment => {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        statements.push(trimmed.to_string());
                    }
                    current.clear();
                }
                // Handle all other characters
                _ => {
                    current.push(ch);
                }
            }
        }
        
        // Add remaining content as final statement
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            statements.push(trimmed.to_string());
        }
        
        statements
    }

    fn is_empty(&self, sql: &str, _dialect: Dialect) -> bool {
        // Improved empty check that ignores comments
        let mut chars = sql.chars().peekable();
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        
        while let Some(ch) = chars.next() {
            match ch {
                // Handle line comments
                '-' if !in_block_comment => {
                    if chars.peek() == Some(&'-') {
                        chars.next(); // consume second '-'
                        in_line_comment = true;
                    } else if !ch.is_whitespace() && !in_line_comment {
                        return false; // Found non-whitespace, non-comment content
                    }
                }
                // Handle block comments start
                '/' if !in_line_comment => {
                    if chars.peek() == Some(&'*') {
                        chars.next(); // consume '*'
                        in_block_comment = true;
                    } else if !ch.is_whitespace() && !in_block_comment {
                        return false; // Found non-whitespace, non-comment content
                    }
                }
                // Handle block comments end
                '*' if in_block_comment => {
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume '/'
                        in_block_comment = false;
                    }
                }
                // Handle line endings (end line comments)
                '\n' | '\r' => {
                    if in_line_comment {
                        in_line_comment = false;
                    }
                }
                // Handle other characters
                _ => {
                    if !ch.is_whitespace() && !in_line_comment && !in_block_comment {
                        return false; // Found actual content
                    }
                }
            }
        }
        
        true // Only found whitespace and comments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_statement_splitting() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT * FROM table1; INSERT INTO table2 VALUES (1, 2)";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT * FROM table1");
        assert_eq!(result[1], "INSERT INTO table2 VALUES (1, 2)");
    }
    
    #[test]
    fn test_string_literals_with_semicolons() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT 'value;with;semicolons' FROM table; INSERT INTO logs VALUES ('error;message')";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT 'value;with;semicolons' FROM table");
        assert_eq!(result[1], "INSERT INTO logs VALUES ('error;message')");
    }
    
    #[test]
    fn test_double_quoted_identifiers() {
        let splitter = NaiveStmtSplitter;
        let sql = r#"SELECT "column;name" FROM "table;name"; CREATE TABLE test (id INT)"#;
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], r#"SELECT "column;name" FROM "table;name""#);
        assert_eq!(result[1], "CREATE TABLE test (id INT)");
    }
    
    #[test]
    fn test_line_comments_with_semicolons() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT * FROM table1; -- comment with ; semicolon\nINSERT INTO table2 VALUES (1)";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT * FROM table1");
        assert_eq!(result[1], "-- comment with ; semicolon\nINSERT INTO table2 VALUES (1)");
    }
    
    #[test]
    fn test_block_comments_with_semicolons() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT * FROM table1; /* block comment; with semicolon */ INSERT INTO table2 VALUES (1)";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT * FROM table1");
        assert_eq!(result[1], "/* block comment; with semicolon */ INSERT INTO table2 VALUES (1)");
    }
    
    #[test]
    fn test_mixed_quotes_and_comments() {
        let splitter = NaiveStmtSplitter;
        let sql = r#"
            SELECT 'text;with;semicolons', "identifier;name" FROM table1; 
            -- This is a comment; with semicolons
            INSERT INTO table2 VALUES ('data', "col;name"); 
            /* Multi-line comment
               with; semicolons; inside */
            UPDATE table3 SET col = 'value;test'
        "#;
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 3);
        assert!(result[0].contains("SELECT"));
        assert!(result[1].contains("INSERT"));
        assert!(result[2].contains("UPDATE"));
    }
    
    #[test]
    fn test_empty_statements() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT * FROM table1; ; ; INSERT INTO table2 VALUES (1)";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        // Empty statements should be filtered out
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT * FROM table1");
        assert_eq!(result[1], "INSERT INTO table2 VALUES (1)");
    }
    
    #[test]
    fn test_trailing_semicolon() {
        let splitter = NaiveStmtSplitter;
        let sql = "SELECT * FROM table1; INSERT INTO table2 VALUES (1);";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "SELECT * FROM table1");
        assert_eq!(result[1], "INSERT INTO table2 VALUES (1)");
    }
    
    #[test]
    fn test_is_empty_with_comments() {
        let splitter = NaiveStmtSplitter;
        
        // Empty string
        assert!(splitter.is_empty("", Dialect::Snowflake));
        
        // Only whitespace
        assert!(splitter.is_empty("   \n\t  ", Dialect::Snowflake));
        
        // Only line comment
        assert!(splitter.is_empty("-- This is just a comment", Dialect::Snowflake));
        
        // Only block comment
        assert!(splitter.is_empty("/* This is a block comment */", Dialect::Snowflake));
        
        // Whitespace with comments
        assert!(splitter.is_empty("  \n-- comment\n  /* block */ \t", Dialect::Snowflake));
        
        // Contains actual SQL
        assert!(!splitter.is_empty("SELECT 1", Dialect::Snowflake));
        assert!(!splitter.is_empty("-- comment\nSELECT 1", Dialect::Snowflake));
    }
    
    #[test]
    fn test_escaped_quotes() {
        let splitter = NaiveStmtSplitter;
        // Note: SQL standard escape is '' not \'
        let sql = "SELECT 'don''t split; this' FROM table; INSERT INTO test VALUES ('can''t break; this')";
        let result = splitter.split(sql, Dialect::Snowflake);
        
        assert_eq!(result.len(), 2);
        assert!(result[0].contains("don''t split; this"));
        assert!(result[1].contains("can''t break; this"));
    }

    #[cfg(test)]
    mod performance_tests {
        use super::*;
        use std::time::Instant;
        
        #[test]
        fn benchmark_splitting_performance() {
            let splitter = NaiveStmtSplitter;
            
            // Create a complex SQL with mixed content
            let complex_sql = r#"
                SELECT 'string;with;semicolons', "quoted;identifier" FROM table1; 
                -- Line comment with ; semicolon
                INSERT INTO table2 VALUES ('data;value', 123); 
                /* Block comment
                   with; multiple; semicolons; 
                   spanning lines */
                UPDATE table3 SET col = 'another;string;value' WHERE id > 0;
                DELETE FROM table4 WHERE name LIKE '%test;pattern%'
            "#;
            
            // Warm up
            for _ in 0..100 {
                let _ = splitter.split(complex_sql, Dialect::Snowflake);
            }
            
            // Benchmark current implementation
            let iterations = 10_000;
            let start = Instant::now();
            for _ in 0..iterations {
                let _ = splitter.split(complex_sql, Dialect::Snowflake);
            }
            let duration = start.elapsed();
            
            println!("Improved splitter: {} iterations in {:?}", iterations, duration);
            println!("Average per iteration: {:?}", duration / iterations);
            
            // Verify correctness
            let result = splitter.split(complex_sql, Dialect::Snowflake);
            assert_eq!(result.len(), 4); // Should find 4 statements
            
            // Verify no semicolons from strings/comments caused incorrect splits
            assert!(result.iter().all(|stmt| {
                !stmt.trim().is_empty() && 
                (stmt.contains("SELECT") || stmt.contains("INSERT") || 
                 stmt.contains("UPDATE") || stmt.contains("DELETE"))
            }));
        }
        
        #[test]
        fn benchmark_naive_vs_improved() {
            // Simple naive implementation for comparison
            fn naive_split(sql: &str) -> Vec<String> {
                sql.split(';').map(|s| s.trim().to_string()).collect()
            }
            
            let sql = "SELECT * FROM table1; INSERT INTO table2 VALUES (1, 2); UPDATE table3 SET col = 1";
            let iterations = 50_000;
            
            // Benchmark naive approach
            let start = Instant::now();
            for _ in 0..iterations {
                let _ = naive_split(sql);
            }
            let naive_duration = start.elapsed();
            
            // Benchmark improved approach
            let splitter = NaiveStmtSplitter;
            let start = Instant::now();
            for _ in 0..iterations {
                let _ = splitter.split(sql, Dialect::Snowflake);
            }
            let improved_duration = start.elapsed();
            
            println!("Naive approach: {} iterations in {:?}", iterations, naive_duration);
            println!("Improved approach: {} iterations in {:?}", iterations, improved_duration);
            println!("Overhead ratio: {:.2}x", improved_duration.as_nanos() as f64 / naive_duration.as_nanos() as f64);
            
            // The improved version should be only 2-3x slower for simple cases
            // but provide correct results for complex cases
            assert!(improved_duration.as_nanos() < naive_duration.as_nanos() * 5);
        }
    }
}
