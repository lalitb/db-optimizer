// HTTP request handlers

pub mod config_tuning;
pub mod fleet;
pub mod locks;
pub mod queries;
pub mod recommendations;
pub mod stats;

use crate::http::{ApiResponse, HealthStatus};
use axum::{response::Html, Json};

/// Root endpoint - API information
pub async fn root() -> Html<&'static str> {
    Html(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>DB Optimizer API</title>
    <style>
        body { font-family: system-ui; max-width: 800px; margin: 40px auto; padding: 20px; }
        h1 { color: #2563eb; }
        .endpoint { background: #f1f5f9; padding: 15px; margin: 10px 0; border-radius: 5px; }
        .method { color: #059669; font-weight: bold; }
        code { background: #e2e8f0; padding: 2px 6px; border-radius: 3px; }
    </style>
</head>
<body>
    <h1>DB Optimizer API</h1>
    <p>PostgreSQL Query Optimizer with eBPF Integration</p>
    
    <h2>Available Endpoints</h2>
    
    <h3>Health & Stats</h3>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/health</code> - Health check
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/stats/summary</code> - System statistics
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/stats/metrics</code> - Prometheus metrics
    </div>
    
    <h3>Lock Contention</h3>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/locks/contention</code> - High contention tables
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/locks/summary</code> - Lock statistics summary
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/locks/events</code> - Recent lock events
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/locks/table/:oid</code> - Table-specific locks
    </div>
    
    <h3>Queries</h3>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/queries/slow</code> - Slow queries
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/queries/:query_id</code> - Query details
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/queries/analyze</code> - Analyze query
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/queries/rewrite</code> - Rewrite query
    </div>
    
    <h3>Recommendations</h3>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/recommendations</code> - List all
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/recommendations/:id</code> - Get by ID
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/recommendations/:id/approve</code> - Approve
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/recommendations/:id/reject</code> - Reject
    </div>
    
    <h3>Configuration Tuning</h3>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/config/tune</code> - Generate optimal config
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/config/validate</code> - Validate current config
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/config/detect-workload</code> - Auto-detect workload type
    </div>

    <h3>Fleet Management</h3>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/fleet</code> - List instances
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/fleet/summary</code> - Fleet summary
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/fleet</code> - Register instance
    </div>
    <div class="endpoint">
        <span class="method">GET</span> <code>/api/v1/fleet/:id</code> - Instance details
    </div>
    <div class="endpoint">
        <span class="method">POST</span> <code>/api/v1/fleet/:id/check</code> - Trigger health check
    </div>
</body>
</html>
    "#,
    )
}
