//! Managed database detection and environment detection.
//!
//! Detects RDS, Cloud SQL, Azure Database for PostgreSQL, and other
//! managed environments where certain operations may be restricted.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::info;

/// Detected database environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DatabaseEnvironment {
    /// Self-managed PostgreSQL (full access)
    SelfManaged,
    /// AWS RDS for PostgreSQL
    AwsRds,
    /// AWS Aurora PostgreSQL
    AwsAurora,
    /// Google Cloud SQL for PostgreSQL
    GoogleCloudSql,
    /// Azure Database for PostgreSQL
    AzurePostgres,
    /// Heroku Postgres
    Heroku,
    /// Supabase
    Supabase,
    /// Neon
    Neon,
    /// Unknown managed environment
    UnknownManaged,
}

impl std::fmt::Display for DatabaseEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SelfManaged => write!(f, "Self-managed PostgreSQL"),
            Self::AwsRds => write!(f, "AWS RDS for PostgreSQL"),
            Self::AwsAurora => write!(f, "AWS Aurora PostgreSQL"),
            Self::GoogleCloudSql => write!(f, "Google Cloud SQL"),
            Self::AzurePostgres => write!(f, "Azure Database for PostgreSQL"),
            Self::Heroku => write!(f, "Heroku Postgres"),
            Self::Supabase => write!(f, "Supabase"),
            Self::Neon => write!(f, "Neon"),
            Self::UnknownManaged => write!(f, "Unknown managed environment"),
        }
    }
}

/// Environment detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    /// Detected environment type
    pub environment: DatabaseEnvironment,
    /// PostgreSQL version
    pub pg_version: String,
    /// Whether pg_stat_statements is available
    pub has_pg_stat_statements: bool,
    /// Whether CREATE INDEX CONCURRENTLY is allowed
    pub can_create_index: bool,
    /// Whether DROP INDEX CONCURRENTLY is allowed
    pub can_drop_index: bool,
    /// Whether superuser access is available
    pub is_superuser: bool,
    /// Whether current user owns the database
    pub is_db_owner: bool,
    /// Warnings about the environment
    pub warnings: Vec<String>,
    /// Recommended operation mode
    pub recommended_mode: String,
}

/// Detect the database environment and capabilities
pub async fn detect_environment(pool: &PgPool) -> anyhow::Result<EnvironmentInfo> {
    let mut warnings = Vec::new();

    // Get PostgreSQL version
    let (pg_version,): (String,) = sqlx::query_as("SELECT version()").fetch_one(pool).await?;

    // Check for managed environment markers
    let environment = detect_managed_environment(pool, &pg_version).await;

    // Check pg_stat_statements
    let has_pg_stat_statements = check_pg_stat_statements(pool).await;
    if !has_pg_stat_statements {
        warnings.push(
            "pg_stat_statements extension not available - query analysis limited".to_string(),
        );
    }

    // Check permissions
    let (is_superuser, is_db_owner) = check_permissions(pool).await?;

    // Check if we can create indexes
    let can_create_index = check_index_permissions(pool).await;
    let can_drop_index = can_create_index; // Same permission typically

    if !can_create_index {
        warnings.push("No CREATE INDEX permission - operating in read-only mode".to_string());
    }

    // Determine recommended mode
    let recommended_mode = if !can_create_index {
        "observe_only".to_string()
    } else if matches!(
        environment,
        DatabaseEnvironment::AwsRds
            | DatabaseEnvironment::AwsAurora
            | DatabaseEnvironment::GoogleCloudSql
            | DatabaseEnvironment::AzurePostgres
            | DatabaseEnvironment::Heroku
            | DatabaseEnvironment::Supabase
            | DatabaseEnvironment::Neon
    ) {
        warnings.push(format!(
            "Detected {} - recommend starting in observe_only mode",
            environment
        ));
        "observe_only".to_string()
    } else {
        "safe_auto".to_string()
    };

    // Log detection results
    info!(
        environment = %environment,
        pg_version = %pg_version,
        has_pg_stat_statements = has_pg_stat_statements,
        can_create_index = can_create_index,
        recommended_mode = %recommended_mode,
        "Environment detected"
    );

    Ok(EnvironmentInfo {
        environment,
        pg_version,
        has_pg_stat_statements,
        can_create_index,
        can_drop_index,
        is_superuser,
        is_db_owner,
        warnings,
        recommended_mode,
    })
}

async fn detect_managed_environment(pool: &PgPool, version: &str) -> DatabaseEnvironment {
    let version_lower = version.to_lowercase();

    // Check version string for cloud markers
    if version_lower.contains("rds") || version_lower.contains("rdsadmin") {
        if version_lower.contains("aurora") {
            return DatabaseEnvironment::AwsAurora;
        }
        return DatabaseEnvironment::AwsRds;
    }

    if version_lower.contains("cloudsql") || version_lower.contains("gcp") {
        return DatabaseEnvironment::GoogleCloudSql;
    }

    // Check for cloud-specific roles/users
    let cloud_roles: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT rolname FROM pg_roles 
        WHERE rolname IN ('rds_superuser', 'rdsadmin', 'cloudsqlsuperuser', 'azure_pg_admin', 'postgres_exporter')
        LIMIT 5
        "#,
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for (role,) in &cloud_roles {
        match role.as_str() {
            "rds_superuser" | "rdsadmin" => return DatabaseEnvironment::AwsRds,
            "cloudsqlsuperuser" => return DatabaseEnvironment::GoogleCloudSql,
            "azure_pg_admin" => return DatabaseEnvironment::AzurePostgres,
            _ => {}
        }
    }

    // Check for Heroku markers
    let heroku_check: Option<(String,)> =
        sqlx::query_as("SELECT current_database() WHERE current_database() ~ '^[a-z]{14}$'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    if heroku_check.is_some() {
        // Heroku databases have 14-character random names
        let setting: Option<(String,)> = sqlx::query_as(
            "SELECT setting FROM pg_settings WHERE name = 'log_line_prefix' AND setting LIKE '%heroku%'",
        )
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if setting.is_some() {
            return DatabaseEnvironment::Heroku;
        }
    }

    // Check for Supabase
    let supabase_roles: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM pg_roles WHERE rolname IN ('supabase_admin', 'supabase_auth_admin') LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    if supabase_roles.is_some() {
        return DatabaseEnvironment::Supabase;
    }

    // Check for Neon
    let neon_check: Option<(String,)> =
        sqlx::query_as("SELECT setting FROM pg_settings WHERE name = 'neon.timeline_id'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    if neon_check.is_some() {
        return DatabaseEnvironment::Neon;
    }

    // Check if we have local file system access (self-managed indicator)
    let data_dir: Option<(String,)> =
        sqlx::query_as("SELECT setting FROM pg_settings WHERE name = 'data_directory'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    if data_dir
        .map(|(d,)| d.starts_with("/var/lib/postgresql") || d.starts_with("/data"))
        .unwrap_or(false)
    {
        return DatabaseEnvironment::SelfManaged;
    }

    // If we can't determine, assume self-managed
    DatabaseEnvironment::SelfManaged
}

async fn check_pg_stat_statements(pool: &PgPool) -> bool {
    let result: Option<(String,)> =
        sqlx::query_as("SELECT extname FROM pg_extension WHERE extname = 'pg_stat_statements'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    result.is_some()
}

async fn check_permissions(pool: &PgPool) -> anyhow::Result<(bool, bool)> {
    let (is_superuser,): (bool,) =
        sqlx::query_as("SELECT usesuper FROM pg_user WHERE usename = current_user")
            .fetch_one(pool)
            .await?;

    let is_db_owner: bool = sqlx::query_as::<_, (bool,)>(
        "SELECT pg_catalog.pg_get_userbyid(datdba) = current_user FROM pg_database WHERE datname = current_database()",
    )
    .fetch_one(pool)
    .await
    .map(|(b,)| b)
    .unwrap_or(false);

    Ok((is_superuser, is_db_owner))
}

async fn check_index_permissions(pool: &PgPool) -> bool {
    // Try to check if we have index creation privileges on any table
    let result: Option<(bool,)> = sqlx::query_as(
        r#"
        SELECT bool_or(has_table_privilege(c.oid, 'INSERT'))
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind = 'r' 
        AND n.nspname NOT IN ('pg_catalog', 'information_schema')
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    result.map(|(b,)| b).unwrap_or(false)
}

/// Force read-only mode check for managed environments
pub fn should_force_read_only(env: &EnvironmentInfo) -> bool {
    !env.can_create_index
        || matches!(
            env.environment,
            DatabaseEnvironment::AwsRds
                | DatabaseEnvironment::AwsAurora
                | DatabaseEnvironment::GoogleCloudSql
                | DatabaseEnvironment::AzurePostgres
                | DatabaseEnvironment::Heroku
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_display() {
        assert_eq!(
            DatabaseEnvironment::AwsRds.to_string(),
            "AWS RDS for PostgreSQL"
        );
        assert_eq!(
            DatabaseEnvironment::SelfManaged.to_string(),
            "Self-managed PostgreSQL"
        );
    }

    #[test]
    fn test_should_force_read_only() {
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::AwsRds,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: false,
            is_db_owner: false,
            warnings: vec![],
            recommended_mode: "observe_only".to_string(),
        };

        // RDS should force read-only even with permissions
        assert!(should_force_read_only(&env));

        let self_managed = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };

        assert!(!should_force_read_only(&self_managed));
    }
}
