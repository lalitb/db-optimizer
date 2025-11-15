// src/persistence/schema.rs - Database schema management

/// Get the SQL schema for creating tables
pub fn get_schema() -> &'static str {
    include_str!("schema.sql")
}

/// Schema version
pub const SCHEMA_VERSION: i32 = 1;
