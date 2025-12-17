use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;

/// Comprehensive column statistics for intelligent index type selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnStats {
    pub schema_name: String,
    pub table_name: String,
    pub column_name: String,
    pub data_type: PostgresType,
    pub total_rows: i64,

    // Cardinality and distribution
    pub distinct_values: i64,
    pub null_fraction: f64,
    pub cardinality_ratio: f64, // distinct_values / total_rows
    pub correlation: f64,       // Physical vs logical order (-1.0 to 1.0)

    // Value characteristics
    pub avg_width: i32,
    pub most_common_values: Vec<String>,
    pub most_common_freqs: Vec<f64>,

    // Query pattern flags (detected from pg_stat_statements)
    pub has_json_ops: bool,
    pub has_array_ops: bool,
    pub has_text_patterns: bool,
    pub has_geometric_ops: bool,
    pub has_range_queries: bool,
    pub is_sequential: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PostgresType {
    // Numeric types
    Integer,
    Bigint,
    Numeric,
    Real,
    DoublePrecision,

    // Text types
    Text,
    Varchar,

    // Date/Time types
    Date,
    Timestamp,
    TimestampTz,

    // Binary types
    Bytea,

    // JSON types
    Json,
    Jsonb,

    // Special types
    Uuid,
    Boolean,

    // Array types
    Array(Box<PostgresType>),

    // Geometric types (PostGIS)
    Geometry,
    Point,

    // Full-text search
    Tsvector,

    // Custom/Unknown
    Custom(String),
}

pub struct ColumnStatsCollector {
    pool: PgPool,
    cache: HashMap<(String, String, String), ColumnStats>,
}

impl ColumnStatsCollector {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: HashMap::new(),
        }
    }

    /// Collect comprehensive statistics for a column
    pub async fn collect_column_stats(
        &mut self,
        schema: &str,
        table: &str,
        column: &str,
    ) -> Result<ColumnStats, Box<dyn std::error::Error>> {
        // Check cache first
        let cache_key = (schema.to_string(), table.to_string(), column.to_string());
        if let Some(stats) = self.cache.get(&cache_key) {
            return Ok(stats.clone());
        }

        // Get data type
        let data_type = self.get_column_type(schema, table, column).await?;

        // Get pg_stats statistics
        let pg_stats = self.get_pg_stats(schema, table, column).await?;

        // Get row count
        let total_rows = self.get_row_count(schema, table).await?;

        // Analyze query patterns
        let query_patterns = self.analyze_query_patterns(table, column).await?;

        // Calculate derived values
        let cardinality_ratio = if total_rows > 0 {
            pg_stats.n_distinct as f64 / total_rows as f64
        } else {
            0.0
        };

        let is_sequential = pg_stats.correlation.map(|c| c.abs() > 0.9).unwrap_or(false);

        // Build comprehensive stats
        let stats = ColumnStats {
            schema_name: schema.to_string(),
            table_name: table.to_string(),
            column_name: column.to_string(),
            data_type,
            total_rows,
            distinct_values: pg_stats.n_distinct,
            null_fraction: pg_stats.null_frac,
            cardinality_ratio,
            correlation: pg_stats.correlation.unwrap_or(0.0),
            avg_width: pg_stats.avg_width,
            most_common_values: pg_stats.most_common_vals,
            most_common_freqs: pg_stats.most_common_freqs,
            is_sequential,
            has_json_ops: query_patterns.has_json_ops,
            has_array_ops: query_patterns.has_array_ops,
            has_text_patterns: query_patterns.has_text_patterns,
            has_geometric_ops: query_patterns.has_geometric_ops,
            has_range_queries: query_patterns.has_range_queries,
        };

        // Cache the result
        self.cache.insert(cache_key, stats.clone());

        Ok(stats)
    }

    async fn get_column_type(
        &self,
        schema: &str,
        table: &str,
        column: &str,
    ) -> Result<PostgresType, Box<dyn std::error::Error>> {
        let query = r#"
            SELECT data_type, udt_name
            FROM information_schema.columns
            WHERE table_schema = $1
              AND table_name = $2
              AND column_name = $3
        "#;

        let row = sqlx::query(query)
            .bind(schema)
            .bind(table)
            .bind(column)
            .fetch_one(&self.pool)
            .await?;

        let data_type: String = row.get("data_type");
        Ok(self.parse_postgres_type(&data_type))
    }

    async fn get_pg_stats(
        &self,
        schema: &str,
        table: &str,
        column: &str,
    ) -> Result<PgStats, Box<dyn std::error::Error>> {
        let query = r#"
            SELECT 
                COALESCE(n_distinct, 0) as n_distinct,
                COALESCE(null_frac, 0.0) as null_frac,
                COALESCE(avg_width, 0) as avg_width,
                correlation
            FROM pg_stats
            WHERE schemaname = $1
              AND tablename = $2
              AND attname = $3
        "#;

        let row = sqlx::query(query)
            .bind(schema)
            .bind(table)
            .bind(column)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            Ok(PgStats {
                n_distinct: row.get::<f64, _>("n_distinct") as i64,
                null_frac: row.get("null_frac"),
                avg_width: row.get("avg_width"),
                correlation: row.get("correlation"),
                most_common_vals: Vec::new(), // Simplified for now
                most_common_freqs: Vec::new(),
            })
        } else {
            // No stats available - return defaults
            Ok(PgStats {
                n_distinct: 0,
                null_frac: 0.0,
                avg_width: 0,
                correlation: Some(0.0),
                most_common_vals: Vec::new(),
                most_common_freqs: Vec::new(),
            })
        }
    }

    async fn get_row_count(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let query = format!("SELECT COUNT(*) as count FROM {}.{}", schema, table);

        let row = sqlx::query(&query).fetch_one(&self.pool).await?;

        Ok(row.get::<i64, _>("count"))
    }

    async fn analyze_query_patterns(
        &self,
        table: &str,
        column: &str,
    ) -> Result<QueryPatterns, Box<dyn std::error::Error>> {
        // Check if pg_stat_statements is available
        let check_extension = r#"
            SELECT COUNT(*) as count
            FROM pg_extension
            WHERE extname = 'pg_stat_statements'
        "#;

        let row = sqlx::query(check_extension).fetch_one(&self.pool).await?;

        let has_extension: i64 = row.get("count");

        if has_extension == 0 {
            // Extension not available, return defaults
            return Ok(QueryPatterns::default());
        }

        // Analyze query patterns from pg_stat_statements
        let query = r#"
            SELECT query
            FROM pg_stat_statements
            WHERE query LIKE '%' || $1 || '%'
            LIMIT 100
        "#;

        let rows = sqlx::query(query).bind(table).fetch_all(&self.pool).await?;

        let mut patterns = QueryPatterns::default();

        for row in rows {
            let query_text: String = row.get("query");

            // Check for JSON operations
            if query_text.contains(&format!("{}->>", column))
                || query_text.contains(&format!("{}->", column))
                || query_text.contains("jsonb_")
            {
                patterns.has_json_ops = true;
            }

            // Check for array operations
            if query_text.contains("ANY(") || query_text.contains("@>") || query_text.contains("<@")
            {
                patterns.has_array_ops = true;
            }

            // Check for text pattern matching
            if query_text.contains(&format!("{} LIKE", column))
                || query_text.contains(&format!("{} ILIKE", column))
            {
                patterns.has_text_patterns = true;
            }

            // Check for geometric operations
            if query_text.contains("ST_") || query_text.contains("&&") {
                patterns.has_geometric_ops = true;
            }

            // Check for range queries
            if query_text.contains(&format!("{} >", column))
                || query_text.contains(&format!("{} <", column))
                || query_text.contains("BETWEEN")
            {
                patterns.has_range_queries = true;
            }
        }

        Ok(patterns)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn parse_postgres_type(&self, type_str: &str) -> PostgresType {
        match type_str.to_lowercase().as_str() {
            "integer" | "int" | "int4" => PostgresType::Integer,
            "bigint" | "int8" => PostgresType::Bigint,
            "numeric" | "decimal" => PostgresType::Numeric,
            "real" | "float4" => PostgresType::Real,
            "double precision" | "float8" => PostgresType::DoublePrecision,
            "text" => PostgresType::Text,
            "character varying" | "varchar" => PostgresType::Varchar,
            "timestamp without time zone" | "timestamp" => PostgresType::Timestamp,
            "timestamp with time zone" | "timestamptz" => PostgresType::TimestampTz,
            "date" => PostgresType::Date,
            "bytea" => PostgresType::Bytea,
            "json" => PostgresType::Json,
            "jsonb" => PostgresType::Jsonb,
            "uuid" => PostgresType::Uuid,
            "boolean" | "bool" => PostgresType::Boolean,
            "geometry" => PostgresType::Geometry,
            "point" => PostgresType::Point,
            "tsvector" => PostgresType::Tsvector,
            s if s.contains("[]") => {
                let base_type = s.replace("[]", "");
                PostgresType::Array(Box::new(self.parse_postgres_type(&base_type)))
            }
            other => PostgresType::Custom(other.to_string()),
        }
    }
}

#[derive(Default)]
struct QueryPatterns {
    has_json_ops: bool,
    has_array_ops: bool,
    has_text_patterns: bool,
    has_geometric_ops: bool,
    has_range_queries: bool,
}

struct PgStats {
    n_distinct: i64,
    null_frac: f64,
    avg_width: i32,
    correlation: Option<f64>,
    most_common_vals: Vec<String>,
    most_common_freqs: Vec<f64>,
}
