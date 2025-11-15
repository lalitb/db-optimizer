// Workload detector - analyzes pg_stat_statements to determine workload type

use super::models::WorkloadType;
use anyhow::Result;
use sqlx::{PgPool, Row};

/// Workload detector
pub struct WorkloadDetector {
    // Thresholds for workload classification
    oltp_max_avg_duration_ms: f64,
    oltp_min_connections: i32,
    dw_min_avg_duration_ms: f64,
    dw_max_write_ratio: f64,
    web_typical_connections_min: i32,
    web_typical_connections_max: i32,
}

impl WorkloadDetector {
    pub fn new() -> Self {
        Self {
            oltp_max_avg_duration_ms: 100.0,  // OLTP queries are fast
            oltp_min_connections: 150,         // OLTP has many connections
            dw_min_avg_duration_ms: 1000.0,    // DW queries are slow
            dw_max_write_ratio: 0.2,           // DW is mostly reads
            web_typical_connections_min: 50,   // Web apps have moderate connections
            web_typical_connections_max: 250,
        }
    }

    /// Detect workload type by analyzing database activity
    pub async fn detect(&self, pool: &PgPool) -> Result<WorkloadType> {
        // Get query statistics
        let stats = self.get_query_stats(pool).await?;

        // Get connection statistics
        let conn_stats = self.get_connection_stats(pool).await?;

        // Decision tree based on multiple signals
        let workload = self.classify_workload(&stats, &conn_stats);

        Ok(workload)
    }

    /// Get query statistics from pg_stat_statements
    async fn get_query_stats(&self, pool: &PgPool) -> Result<QueryStats> {
        // Check if pg_stat_statements is available
        let extension_row = sqlx::query(
            "SELECT COUNT(*) as count FROM pg_extension WHERE extname = 'pg_stat_statements'"
        )
        .fetch_one(pool)
        .await?;

        let ext_count: i64 = extension_row.try_get("count")?;
        
        if ext_count == 0 {
            // No pg_stat_statements, use conservative estimates
            return Ok(QueryStats::default());
        }

        let row = sqlx::query(
            r#"
            SELECT 
                COUNT(*) as query_count,
                COALESCE(AVG(mean_exec_time), 0)::float as avg_duration_ms,
                COALESCE(SUM(calls), 0)::bigint as total_calls,
                COALESCE(SUM(CASE WHEN query ILIKE 'SELECT%' THEN calls ELSE 0 END), 0)::bigint as read_calls,
                COALESCE(SUM(CASE WHEN query ILIKE 'INSERT%' OR query ILIKE 'UPDATE%' OR query ILIKE 'DELETE%' THEN calls ELSE 0 END), 0)::bigint as write_calls,
                COALESCE(AVG(CASE WHEN query ILIKE '%JOIN%' THEN 1 ELSE 0 END), 0)::float as join_ratio,
                COALESCE(MAX(mean_exec_time), 0)::float as max_avg_duration_ms
            FROM pg_stat_statements
            WHERE calls > 10
            "#
        )
        .fetch_one(pool)
        .await?;

        Ok(QueryStats {
            query_count: row.try_get::<i64, _>("query_count")? as i32,
            avg_duration_ms: row.try_get("avg_duration_ms")?,
            total_calls: row.try_get("total_calls")?,
            read_calls: row.try_get("read_calls")?,
            write_calls: row.try_get("write_calls")?,
            join_ratio: row.try_get("join_ratio")?,
            max_avg_duration_ms: row.try_get("max_avg_duration_ms")?,
        })
    }

    /// Get connection statistics
    async fn get_connection_stats(&self, pool: &PgPool) -> Result<ConnectionStats> {
        let row = sqlx::query(
            r#"
            SELECT 
                (SELECT setting::int FROM pg_settings WHERE name = 'max_connections') as max_connections,
                COUNT(*)::int as active_connections,
                COALESCE(AVG(EXTRACT(EPOCH FROM (now() - backend_start)))::int, 0) as avg_age_seconds
            FROM pg_stat_activity
            WHERE backend_type = 'client backend'
            "#
        )
        .fetch_one(pool)
        .await?;

        Ok(ConnectionStats {
            max_connections: row.try_get("max_connections")?,
            active_connections: row.try_get("active_connections")?,
            avg_age_seconds: row.try_get("avg_age_seconds")?,
        })
    }

    /// Classify workload based on collected statistics
    fn classify_workload(&self, query_stats: &QueryStats, conn_stats: &ConnectionStats) -> WorkloadType {
        // Calculate key metrics
        let read_ratio = if query_stats.write_calls > 0 {
            query_stats.read_calls as f64 / query_stats.write_calls as f64
        } else {
            10.0 // Assume read-heavy if no writes
        };

        let connection_utilization = 
            conn_stats.active_connections as f64 / conn_stats.max_connections as f64;

        // Decision tree
        
        // Desktop: very few connections, local development
        if conn_stats.max_connections <= 30 && conn_stats.active_connections < 5 {
            return WorkloadType::Desktop;
        }

        // Data Warehouse: slow queries, read-heavy, complex joins
        if query_stats.avg_duration_ms > self.dw_min_avg_duration_ms
            && read_ratio > 5.0
            && query_stats.join_ratio > 0.3
        {
            return WorkloadType::DataWarehouse;
        }

        // OLTP: fast queries, many connections, balanced read/write
        if query_stats.avg_duration_ms < self.oltp_max_avg_duration_ms
            && conn_stats.active_connections > self.oltp_min_connections
            && read_ratio < 10.0  // Not too read-heavy
            && read_ratio > 0.5   // Not too write-heavy
        {
            return WorkloadType::OLTP;
        }

        // Web: moderate connections, fast-ish queries, more reads than writes
        if conn_stats.active_connections >= self.web_typical_connections_min
            && conn_stats.active_connections <= self.web_typical_connections_max
            && query_stats.avg_duration_ms < 500.0
            && read_ratio > 2.0
        {
            return WorkloadType::Web;
        }

        // High connection churn suggests web/OLTP
        if conn_stats.avg_age_seconds < 600 && conn_stats.active_connections > 20 {
            if query_stats.avg_duration_ms < self.oltp_max_avg_duration_ms {
                return WorkloadType::OLTP;
            } else {
                return WorkloadType::Web;
            }
        }

        // Default to Mixed for ambiguous cases
        WorkloadType::Mixed
    }

    /// Get confidence level for the detection (0.0 to 1.0)
    pub fn get_confidence(&self, workload: &WorkloadType, query_stats: &QueryStats, conn_stats: &ConnectionStats) -> f64 {
        let read_ratio = if query_stats.write_calls > 0 {
            query_stats.read_calls as f64 / query_stats.write_calls as f64
        } else {
            10.0
        };

        match workload {
            WorkloadType::Desktop => {
                if conn_stats.max_connections <= 20 { 0.95 } else { 0.70 }
            }
            WorkloadType::OLTP => {
                let mut confidence = 0.5;
                if query_stats.avg_duration_ms < self.oltp_max_avg_duration_ms {
                    confidence += 0.2;
                }
                if conn_stats.active_connections > self.oltp_min_connections {
                    confidence += 0.2;
                }
                if read_ratio > 0.5 && read_ratio < 10.0 {
                    confidence += 0.1;
                }
                confidence
            }
            WorkloadType::DataWarehouse => {
                let mut confidence = 0.5;
                if query_stats.avg_duration_ms > self.dw_min_avg_duration_ms {
                    confidence += 0.2;
                }
                if read_ratio > 5.0 {
                    confidence += 0.15;
                }
                if query_stats.join_ratio > 0.3 {
                    confidence += 0.15;
                }
                confidence
            }
            WorkloadType::Web => {
                let mut confidence = 0.5;
                if conn_stats.active_connections >= self.web_typical_connections_min
                    && conn_stats.active_connections <= self.web_typical_connections_max
                {
                    confidence += 0.2;
                }
                if query_stats.avg_duration_ms < 500.0 {
                    confidence += 0.15;
                }
                if read_ratio > 2.0 {
                    confidence += 0.15;
                }
                confidence
            }
            WorkloadType::Mixed => 0.60, // Lower confidence for mixed
        }
    }
}

impl Default for WorkloadDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Query statistics from pg_stat_statements
#[derive(Debug, Clone)]
pub struct QueryStats {
    pub query_count: i32,
    pub avg_duration_ms: f64,
    pub total_calls: i64,
    pub read_calls: i64,
    pub write_calls: i64,
    pub join_ratio: f64,
    pub max_avg_duration_ms: f64,
}

impl Default for QueryStats {
    fn default() -> Self {
        Self {
            query_count: 0,
            avg_duration_ms: 100.0, // Assume moderate
            total_calls: 0,
            read_calls: 0,
            write_calls: 0,
            join_ratio: 0.2,
            max_avg_duration_ms: 1000.0,
        }
    }
}

/// Connection statistics
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub max_connections: i32,
    pub active_connections: i32,
    pub avg_age_seconds: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_workload_detection() {
        let detector = WorkloadDetector::new();
        
        let query_stats = QueryStats {
            query_count: 50,
            avg_duration_ms: 50.0,
            total_calls: 1000,
            read_calls: 800,
            write_calls: 200,
            join_ratio: 0.1,
            max_avg_duration_ms: 200.0,
        };

        let conn_stats = ConnectionStats {
            max_connections: 20,
            active_connections: 3,
            avg_age_seconds: 3600,
        };

        let workload = detector.classify_workload(&query_stats, &conn_stats);
        assert_eq!(workload, WorkloadType::Desktop);

        let confidence = detector.get_confidence(&workload, &query_stats, &conn_stats);
        assert!(confidence > 0.90);
    }

    #[test]
    fn test_oltp_workload_detection() {
        let detector = WorkloadDetector::new();
        
        let query_stats = QueryStats {
            query_count: 500,
            avg_duration_ms: 50.0,  // Fast queries
            total_calls: 100000,
            read_calls: 60000,
            write_calls: 40000,      // Balanced
            join_ratio: 0.2,
            max_avg_duration_ms: 200.0,
        };

        let conn_stats = ConnectionStats {
            max_connections: 300,
            active_connections: 200,  // High connection count
            avg_age_seconds: 300,
        };

        let workload = detector.classify_workload(&query_stats, &conn_stats);
        assert_eq!(workload, WorkloadType::OLTP);

        let confidence = detector.get_confidence(&workload, &query_stats, &conn_stats);
        assert!(confidence > 0.80);
    }

    #[test]
    fn test_data_warehouse_detection() {
        let detector = WorkloadDetector::new();
        
        let query_stats = QueryStats {
            query_count: 100,
            avg_duration_ms: 2000.0,  // Slow queries
            total_calls: 5000,
            read_calls: 4800,
            write_calls: 200,          // Read-heavy
            join_ratio: 0.6,           // Complex joins
            max_avg_duration_ms: 10000.0,
        };

        let conn_stats = ConnectionStats {
            max_connections: 50,
            active_connections: 20,
            avg_age_seconds: 7200,
        };

        let workload = detector.classify_workload(&query_stats, &conn_stats);
        assert_eq!(workload, WorkloadType::DataWarehouse);

        let confidence = detector.get_confidence(&workload, &query_stats, &conn_stats);
        assert!(confidence > 0.75);
    }

    #[test]
    fn test_web_workload_detection() {
        let detector = WorkloadDetector::new();
        
        let query_stats = QueryStats {
            query_count: 300,
            avg_duration_ms: 150.0,   // Moderate speed
            total_calls: 50000,
            read_calls: 40000,
            write_calls: 10000,        // Read-heavy but some writes
            join_ratio: 0.3,
            max_avg_duration_ms: 1000.0,
        };

        let conn_stats = ConnectionStats {
            max_connections: 200,
            active_connections: 120,  // Moderate connections
            avg_age_seconds: 800,
        };

        let workload = detector.classify_workload(&query_stats, &conn_stats);
        assert_eq!(workload, WorkloadType::Web);

        let confidence = detector.get_confidence(&workload, &query_stats, &conn_stats);
        assert!(confidence > 0.70);
    }

    #[test]
    fn test_mixed_workload_default() {
        let detector = WorkloadDetector::new();
        
        // Ambiguous stats that don't clearly match any pattern
        let query_stats = QueryStats {
            query_count: 200,
            avg_duration_ms: 300.0,
            total_calls: 20000,
            read_calls: 15000,
            write_calls: 5000,
            join_ratio: 0.25,
            max_avg_duration_ms: 2000.0,
        };

        let conn_stats = ConnectionStats {
            max_connections: 100,
            active_connections: 40,
            avg_age_seconds: 1200,
        };

        let workload = detector.classify_workload(&query_stats, &conn_stats);
        assert_eq!(workload, WorkloadType::Mixed);
    }

    #[test]
    fn test_confidence_levels() {
        let detector = WorkloadDetector::new();
        let query_stats = QueryStats::default();
        let conn_stats = ConnectionStats {
            max_connections: 100,
            active_connections: 50,
            avg_age_seconds: 600,
        };

        // Desktop should have high confidence with clear signals
        let desktop_confidence = detector.get_confidence(
            &WorkloadType::Desktop,
            &query_stats,
            &ConnectionStats {
                max_connections: 20,
                active_connections: 3,
                avg_age_seconds: 3600,
            },
        );
        assert!(desktop_confidence > 0.90);

        // Mixed should have moderate confidence
        let mixed_confidence = detector.get_confidence(&WorkloadType::Mixed, &query_stats, &conn_stats);
        assert!(mixed_confidence >= 0.60 && mixed_confidence < 0.70);
    }
}
