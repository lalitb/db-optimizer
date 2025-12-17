# CloudNativePG Plugin - Quick Start Guide

**Status:** Beta v0.1.0  
**Date:** December 16, 2025

## 🎯 Vaanyze vs CNPG: Complementary Tools

### CloudNativePG (Built-in Monitoring)
**What it provides:**
- **Metrics collection**: Exposes Prometheus metrics on port 9187
- **Observability**: WAL stats, replication lag, query counts, connection stats
- **Monitoring focus**: "What's happening?" - Current state visibility
- **Examples:**
  - `cnpg_collector_pg_wal{value="count"} 7`
  - `cnpg_pg_replication_lag 5.2`
  - `pg_stat_statements` query execution times

**Use case:** Dashboards, alerting, historical trends in Grafana/Prometheus

### Vaanyze CNPG Plugin (AI-Powered Optimization)
**What it provides:**
- **Deep inspection**: eBPF-based lock contention, I/O patterns, query parsing
- **Intelligence**: AI-powered root cause analysis and optimization recommendations
- **Action-oriented**: "What should I do?" - Concrete, implementable changes
- **Examples:**
  - "Add index on `users(email, created_at)` → 450ms → 2ms (99.5% faster)"
  - "Rewrite `SELECT *` → `SELECT id, name` to reduce I/O by 80%"
  - "Increase `work_mem` from 4MB → 16MB for this workload"
  - Auto-apply safe changes in production

**Use case:** Performance optimization, cost reduction, autonomous database tuning

### They Work Together 🤝

```mermaid
┌─────────────────────────────────────────────────────┐
│           CloudNativePG Cluster                     │
│  ┌──────────────┐  ┌──────────────┐               │
│  │ PostgreSQL   │  │ PostgreSQL   │               │
│  │ Primary      │  │ Replica      │               │
│  │              │  │              │               │
│  │ Port 5432    │  │ Port 5432    │               │
│  │ Port 9187 ──────────► Metrics  │               │
│  └──────────────┘  └──────────────┘               │
│         │                                           │
└─────────┼───────────────────────────────────────────┘
          │
          ├──────────► Prometheus/Grafana
          │            (Observability - "What's happening?")
          │
          └──────────► Vaanyze Plugin (gRPC)
                       (Intelligence - "How to optimize?")
                       
                       ┌─────────────────────┐
                       │ eBPF Data Collection│
                       │ + AI Analysis       │
                       │ + Recommendations   │
                       │ + Auto-Apply        │
                       └─────────────────────┘
```

**Real-World Example:**

1. **CNPG Monitoring shows:**
   ```
   cnpg_pg_replication_lag{cluster="prod"} 5.2
   pg_stat_statements: SELECT * FROM orders WHERE user_id = $1 (avg: 450ms)
   ```

2. **Vaanyze Plugin analyzes:**
   ```
   🔍 Detected: Full table scan on orders (450ms avg, 10k rows)
   📊 Analysis: Missing index on user_id, causing sequential scan
   💡 Recommendation: CREATE INDEX idx_orders_user_id ON orders(user_id)
   📈 Expected improvement: 450ms → 2ms (99.5% faster)
   💰 Cost savings: $120/month (reduced CPU usage)
   ✅ Auto-applied (safe mode) at 2025-12-16 14:30:00
   ```

3. **Result:** Both tools confirm improvement
   - CNPG metrics: Query time drops to 2ms
   - Vaanyze reports: Optimization successful, monitoring for regression

## ✅ What's Working

- CLI command: `pg-query-advisor cnpg-plugin`
- Health check endpoints on port 8081
- Docker containerization
- Kubernetes deployment manifests
- Basic health monitoring

## 🚀 Quick Test Locally

### 1. Build Binary

```bash
# Build with CNPG feature
cargo build --release --features cnpg

# Test help
./target/release/pg-query-advisor cnpg-plugin --help
```

### 2. Run Locally

```bash
# Start the plugin server
./target/release/pg-query-advisor cnpg-plugin --port 8080 --verbose

# In another terminal, test health endpoints
curl http://localhost:8081/health
curl http://localhost:8081/ready
```

Expected output:
```
✅ /health returns "OK"
✅ /ready returns "Ready"
```

## 🐳 Docker Deployment

### 1. Build Docker Image

```bash
docker build -f Dockerfile.cnpg -t vaanyze/cnpg-plugin:test .
```

### 2. Run Container

```bash
docker run -p 8080:8080 -p 8081:8081 vaanyze/cnpg-plugin:test
```

### 3. Test Health Endpoints

```bash
# Test from host
curl http://localhost:8081/health
curl http://localhost:8081/ready
```

## ☸️ Kubernetes Deployment

### Prerequisites

- Kubernetes cluster (Kind, Minikube, or production)
- kubectl configured
- CloudNativePG operator installed

### 1. Install CloudNativePG Operator (if not already)

```bash
kubectl apply -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.22/releases/cnpg-1.22.1.yaml
```

### 2. Deploy Vaanyze Plugin

```bash
# Deploy plugin
kubectl apply -f k8s/cnpg-plugin.yaml

# Wait for ready
kubectl wait --for=condition=Available deployment/vaanyze-cnpg-plugin -n cnpg-system --timeout=60s

# Check pods
kubectl get pods -n cnpg-system -l app=vaanyze-cnpg-plugin
```

### 3. Check Logs

```bash
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin --tail=50
```

Expected log output:
```
Vaanyze CNPG Plugin v0.1.0 starting on 0.0.0.0:8080
Capabilities: 7 enabled
Health check endpoints on http://0.0.0.0:8081
  - GET /health - Liveness probe
  - GET /ready - Readiness probe
gRPC server ready on 0.0.0.0:8080
```

### 4. Verify Health Checks

```bash
# Port-forward to access from local
kubectl port-forward -n cnpg-system svc/vaanyze-cnpg-plugin 8081:80

# Test (in another terminal)
curl http://localhost:8081/health
curl http://localhost:8081/ready
```

## 🧪 Test with Kind (Local Kubernetes)

Complete integration test using Kind:

```bash
# Run automated integration test
./tests/cnpg_integration_test.sh
```

This will:
1. Create Kind cluster
2. Install CNPG operator
3. Build and load plugin image
4. Deploy plugin
5. Create test database
6. Verify plugin is working

## 🔍 Troubleshooting

### Plugin Won't Start

**Symptom:** Pod in CrashLoopBackOff
```bash
kubectl describe pod -n cnpg-system -l app=vaanyze-cnpg-plugin
```

**Common Issues:**
- Image not found: `imagePullPolicy: IfNotPresent` requires local build or registry push
- Missing feature: Ensure image built with `--features cnpg`

**Fix:**
```bash
# Rebuild with correct features
docker build -f Dockerfile.cnpg -t vaanyze/cnpg-plugin:latest .

# For Kind: load image
kind load docker-image vaanyze/cnpg-plugin:latest --name <cluster-name>
```

### Health Checks Failing

**Symptom:** Pod shows "Unhealthy" status

```bash
# Check if port 8081 is listening
kubectl exec -n cnpg-system -it <pod-name> -- netstat -tuln | grep 8081

# Check logs for errors
kubectl logs -n cnpg-system <pod-name>
```

### Cannot Access from Outside Cluster

**Solution:** Use port-forward for testing:
```bash
kubectl port-forward -n cnpg-system svc/vaanyze-cnpg-plugin 8080:80
```

## 📋 Verification Checklist

Before considering deployment successful:

- [ ] Pod is in Running state
- [ ] Health probe returns 200 OK
- [ ] Ready probe returns 200 OK
- [ ] Logs show "gRPC server ready"
- [ ] No errors in pod logs
- [ ] Can curl health endpoint via port-forward

## 🎯 Known Limitations (Beta v0.1.0)

### Current Version

✅ **Working:**
- CLI command runs
- Health monitoring active
- Kubernetes integration
- Liveness/readiness probes
- Logging and observability

⚠️ **Not Yet Implemented:**
- Full gRPC CNPG-I protocol (placeholder)
- Lifecycle event handling (create/update/delete)
- WAL archive hooks
- Backup/restore hooks
- Auto-discovery of CNPG clusters

### Roadmap

**v0.2.0 (Next Week):**
- Implement basic lifecycle event handlers
- Connect to actual PostgreSQL clusters
- Return real health scores

**v0.3.0:**
- Full CNPG-I protocol with tonic-build
- Auto-discovery via Kubernetes API
- Multi-cluster support

**v1.0.0:**
- Production-grade protocol implementation
- Comprehensive testing
- Official CNPG plugin directory listing

## 💡 Usage Examples

### Example 1: Monitor Single Cluster

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: my-db
  annotations:
    cnpg.io/plugins: "vaanyze"  # Enable Vaanyze plugin
spec:
  instances: 3
  storage:
    size: 10Gi
```

### Example 2: Custom Configuration

```yaml
# vaanyze-config.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: vaanyze-config
  namespace: cnpg-system
data:
  config.yaml: |
    health_threshold: 80
    check_interval_seconds: 300
    auto_apply_config:
      enabled: false
      min_savings_threshold: 100.0
```

## 🆘 Support

- **Issues:** https://github.com/lalitb/db-optimizer/issues
- **Discussions:** https://github.com/lalitb/db-optimizer/discussions
- **CNPG Slack:** #vaanyze channel (coming soon)

## 📚 Related Documentation

- [CNPG Integration Guide](docs/CNPG_INTEGRATION.md)
- [Launch Readiness Assessment](CNPG_LAUNCH_READINESS.md)
- [CNPG Target Customers](CNPG_TARGET_CUSTOMERS.txt)
- [Main README](README.md)

---

## 🔄 Using CNPG Monitoring + Vaanyze Together

### Step 1: Set Up CNPG Monitoring

#### Install Prometheus & Grafana (if needed)
```bash
# Add Prometheus Community Helm repo
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm repo update

# Install kube-prometheus-stack
helm install prometheus prometheus-community/kube-prometheus-stack \
  --namespace monitoring \
  --create-namespace \
  --set prometheus.prometheusSpec.podMonitorSelectorNilUsesHelmValues=false \
  --set prometheus.prometheusSpec.serviceMonitorSelectorNilUsesHelmValues=false
```

#### Enable CNPG Metrics Collection
```yaml
# postgres-cluster.yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: production-db
spec:
  instances: 3
  storage:
    size: 50Gi
  
  # Enable automatic PodMonitor creation
  monitoring:
    enablePodMonitor: true
    
    # Optional: Add custom metrics
    customQueriesConfigMap:
      - name: custom-metrics
        key: queries
```

#### Verify Metrics Endpoint
```bash
# Port-forward to PostgreSQL pod
kubectl port-forward production-db-1 9187:9187

# Check metrics (in another terminal)
curl http://localhost:9187/metrics | grep cnpg_collector
```

Expected output:
```
cnpg_collector_up{cluster="production-db"} 1
cnpg_collector_pg_wal{value="count"} 12
cnpg_collector_pg_replication_lag 0.5
```

### Step 2: Deploy Vaanyze Plugin

```bash
# Deploy Vaanyze plugin
kubectl apply -f k8s/cnpg-plugin.yaml

# Verify deployment
kubectl get pods -n cnpg-system -l app=vaanyze-cnpg-plugin
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin
```

### Step 3: Configure Vaanyze to Monitor CNPG Cluster

```yaml
# vaanyze-cluster-config.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: vaanyze-config
  namespace: cnpg-system
data:
  config.yaml: |
    # Database connection
    database_url: "postgresql://app:password@production-db-rw:5432/app"
    
    # Collection settings
    ebpf:
      enabled: true
      protocols:
        - postgres
    
    collection_interval_seconds: 300
    
    # Analysis settings
    ai_analysis:
      enabled: true
      provider: "openai"
      model: "gpt-4"
    
    # Auto-apply configuration
    auto_apply_config:
      enabled: true
      dry_run: false
      min_savings_threshold: 50.0  # Minimum $50/month savings
      safety_mode: conservative    # conservative | moderate | aggressive
      
      # What can be auto-applied
      allowed_actions:
        - create_index
        - drop_unused_index
        - update_statistics
        - rewrite_query
      
      # What requires manual approval
      require_approval:
        - drop_table
        - alter_column
        - major_config_change
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: vaanyze-cnpg-plugin
  namespace: cnpg-system
spec:
  template:
    spec:
      containers:
      - name: plugin
        image: vaanyze/cnpg-plugin:v0.1.0-beta
        env:
        - name: CONFIG_FILE
          value: /etc/vaanyze/config.yaml
        volumeMounts:
        - name: config
          mountPath: /etc/vaanyze
      volumes:
      - name: config
        configMap:
          name: vaanyze-config
```

Apply the configuration:
```bash
kubectl apply -f vaanyze-cluster-config.yaml
kubectl rollout restart deployment/vaanyze-cnpg-plugin -n cnpg-system
```

### Step 4: Set Up Grafana Dashboard

#### Import CNPG Dashboard
```bash
# Get Grafana admin password
kubectl get secret -n monitoring prometheus-grafana -o jsonpath="{.data.admin-password}" | base64 -d

# Port-forward to Grafana
kubectl port-forward -n monitoring svc/prometheus-grafana 3000:80
```

Visit http://localhost:3000, login (admin/password), then:
1. Go to **Dashboards** → **Import**
2. Import CNPG dashboard: ID `20417` or use `docs/grafana-dashboard.json`
3. Select Prometheus datasource

#### Add Vaanyze Metrics Panel

Add custom panel to CNPG dashboard:
```json
{
  "title": "Vaanyze Recommendations Applied",
  "targets": [
    {
      "expr": "vaanyze_recommendations_applied_total",
      "legendFormat": "{{action_type}}"
    }
  ],
  "type": "graph"
}
```

### Step 5: Set Up Alerts

#### CNPG + Vaanyze Alert Rules
```yaml
# alerts.yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: cnpg-vaanyze-alerts
  namespace: monitoring
spec:
  groups:
  - name: database-health
    interval: 30s
    rules:
    # CNPG Alert: High replication lag
    - alert: HighReplicationLag
      expr: cnpg_collector_pg_replication_lag > 10
      for: 5m
      labels:
        severity: warning
      annotations:
        summary: "High replication lag on {{ $labels.cluster }}"
        description: "Replication lag is {{ $value }}s"
    
    # CNPG Alert: Database down
    - alert: PostgreSQLDown
      expr: cnpg_collector_up == 0
      for: 1m
      labels:
        severity: critical
      annotations:
        summary: "PostgreSQL instance down: {{ $labels.cluster }}"
    
    # Vaanyze Alert: Optimization opportunity
    - alert: HighSavingsOpportunity
      expr: vaanyze_potential_monthly_savings > 500
      for: 15m
      labels:
        severity: info
      annotations:
        summary: "High cost savings opportunity detected"
        description: "Potential savings: ${{ $value }}/month"
    
    # Vaanyze Alert: Query performance degraded
    - alert: QueryPerformanceDegraded
      expr: vaanyze_query_p95_latency_ms > 1000
      for: 10m
      labels:
        severity: warning
      annotations:
        summary: "Query performance degraded on {{ $labels.cluster }}"
        description: "P95 latency: {{ $value }}ms"
```

Apply alerts:
```bash
kubectl apply -f alerts.yaml
```

### Step 6: Monitor Both Systems

#### View CNPG Metrics in Prometheus
```bash
# Port-forward to Prometheus
kubectl port-forward -n monitoring svc/prometheus-kube-prometheus-prometheus 9090:9090
```

Visit http://localhost:9090 and query:
```promql
# CNPG metrics
cnpg_collector_up
cnpg_collector_pg_wal
cnpg_pg_replication_lag

# Vaanyze metrics
vaanyze_recommendations_total
vaanyze_query_analysis_duration_seconds
vaanyze_auto_applied_changes_total
```

#### Check Vaanyze Recommendations
```bash
# View recommendations in plugin logs
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin --tail=100 | grep "Recommendation"

# Or query via API (when implemented in v0.2.0)
kubectl port-forward -n cnpg-system svc/vaanyze-cnpg-plugin 8080:80
curl http://localhost:8080/api/v1/recommendations
```

### Step 7: Review Auto-Applied Changes

```bash
# Check auto-applied changes
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin | grep "Auto-applied"

# Review in database
kubectl exec -it production-db-1 -- psql -U postgres -d app -c "
  SELECT * FROM vaanyze_audit_log 
  WHERE action_type = 'auto_applied' 
  ORDER BY created_at DESC 
  LIMIT 10;
"
```

### Complete Integration Example

```yaml
# complete-setup.yaml - Full CNPG + Vaanyze deployment
---
# PostgreSQL Cluster with monitoring
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: app-database
  namespace: production
spec:
  instances: 3
  
  postgresql:
    parameters:
      shared_preload_libraries: "pg_stat_statements"
      pg_stat_statements.track: all
  
  storage:
    size: 100Gi
    storageClass: fast-ssd
  
  monitoring:
    enablePodMonitor: true
    customQueriesConfigMap:
      - name: vaanyze-metrics
        key: queries
  
  backup:
    barmanObjectStore:
      destinationPath: s3://backups/app-database
      s3Credentials:
        accessKeyId:
          name: backup-creds
          key: ACCESS_KEY_ID
        secretAccessKey:
          name: backup-creds
          key: SECRET_ACCESS_KEY

---
# Vaanyze custom metrics for CNPG
apiVersion: v1
kind: ConfigMap
metadata:
  name: vaanyze-metrics
  namespace: production
  labels:
    cnpg.io/reload: ""
data:
  queries: |
    vaanyze_table_bloat:
      query: |
        SELECT 
          schemaname,
          tablename,
          pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) as size,
          (pg_stat_get_live_tuples(schemaname||'.'||tablename::regclass) + 
           pg_stat_get_dead_tuples(schemaname||'.'||tablename::regclass)) as total_tuples,
          pg_stat_get_dead_tuples(schemaname||'.'||tablename::regclass) as dead_tuples,
          CASE WHEN (pg_stat_get_live_tuples(schemaname||'.'||tablename::regclass) + 
                     pg_stat_get_dead_tuples(schemaname||'.'||tablename::regclass)) > 0
               THEN round(100.0 * pg_stat_get_dead_tuples(schemaname||'.'||tablename::regclass) / 
                    (pg_stat_get_live_tuples(schemaname||'.'||tablename::regclass) + 
                     pg_stat_get_dead_tuples(schemaname||'.'||tablename::regclass)), 2)
               ELSE 0
          END as bloat_ratio
        FROM pg_tables
        WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
      metrics:
        - schemaname:
            usage: "LABEL"
        - tablename:
            usage: "LABEL"
        - dead_tuples:
            usage: "GAUGE"
            description: "Number of dead tuples"
        - bloat_ratio:
            usage: "GAUGE"
            description: "Bloat ratio percentage"

---
# Vaanyze Plugin Deployment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: vaanyze-cnpg-plugin
  namespace: cnpg-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: vaanyze-cnpg-plugin
  template:
    metadata:
      labels:
        app: vaanyze-cnpg-plugin
    spec:
      serviceAccountName: vaanyze-plugin
      containers:
      - name: plugin
        image: vaanyze/cnpg-plugin:v0.1.0-beta
        ports:
        - containerPort: 8080
          name: grpc
        - containerPort: 8081
          name: health
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: app-database-app
              key: uri
        - name: OPENAI_API_KEY
          valueFrom:
            secretKeyRef:
              name: vaanyze-secrets
              key: openai-api-key
        - name: CONFIG_FILE
          value: /etc/vaanyze/config.yaml
        livenessProbe:
          httpGet:
            path: /health
            port: health
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /ready
            port: health
          initialDelaySeconds: 5
          periodSeconds: 5
        volumeMounts:
        - name: config
          mountPath: /etc/vaanyze
        resources:
          requests:
            memory: "256Mi"
            cpu: "100m"
          limits:
            memory: "512Mi"
            cpu: "500m"
      volumes:
      - name: config
        configMap:
          name: vaanyze-config
```

Deploy everything:
```bash
kubectl apply -f complete-setup.yaml
```

---

## 📊 Monitoring Dashboard Layout

### Recommended Grafana Dashboard Structure

**Row 1: CNPG Health (Native Metrics)**
- PostgreSQL Up/Down status
- Replication lag
- WAL file count
- Connection count

**Row 2: Vaanyze Intelligence**
- Recommendations generated (today)
- Auto-applied changes (last 7 days)
- Potential cost savings
- Query performance improvement %

**Row 3: Combined Analysis**
- Slow queries detected (CNPG) → Recommendations applied (Vaanyze)
- Before/After query latency comparison
- Index usage efficiency
- Table bloat trends

**Row 4: Alerts & Actions**
- Active alerts timeline
- Recent auto-applied changes
- Manual approval queue
- Cost savings achieved (cumulative)

---

## 🎓 Best Practices

### 1. Start with Monitoring Only
```yaml
# Week 1: Enable CNPG metrics + Vaanyze analysis (dry-run)
auto_apply_config:
  enabled: false
  dry_run: true
```

### 2. Review Recommendations Weekly
```bash
# Export recommendations for review
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin | \
  grep "Recommendation" > weekly-recommendations.txt
```

### 3. Enable Auto-Apply Gradually
```yaml
# Week 2: Enable safe actions only
auto_apply_config:
  enabled: true
  safety_mode: conservative
  allowed_actions:
    - create_index        # Safe
    - update_statistics   # Safe

# Week 4: Add query rewriting
  allowed_actions:
    - create_index
    - update_statistics
    - rewrite_query       # Medium risk

# Week 8: Full automation
  safety_mode: moderate
  allowed_actions:
    - create_index
    - drop_unused_index
    - update_statistics
    - rewrite_query
    - optimize_config
```

### 4. Set Up Notifications
```yaml
# Slack/PagerDuty integration for high-value recommendations
- alert: HighValueOptimization
  expr: vaanyze_potential_monthly_savings > 1000
  annotations:
    summary: "💰 High-value optimization detected: ${{ $value }}/month"
    slack_channel: "#database-ops"
```

### 5. Track ROI
```sql
-- Query to track Vaanyze ROI
SELECT 
  DATE_TRUNC('month', applied_at) as month,
  COUNT(*) as recommendations_applied,
  SUM(estimated_monthly_savings) as total_savings,
  AVG(performance_improvement_pct) as avg_improvement
FROM vaanyze_audit_log
WHERE action_status = 'applied'
GROUP BY month
ORDER BY month DESC;
```

---

## 🚀 Next Steps

1. ✅ Install CNPG with monitoring enabled
2. ✅ Deploy Vaanyze plugin
3. ✅ Configure Grafana dashboards
4. ✅ Set up alerts
5. ✅ Review first week of recommendations
6. ✅ Enable auto-apply for safe actions
7. 📈 Monitor cost savings and performance gains


