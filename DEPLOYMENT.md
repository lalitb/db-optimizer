# 🚀 Deployment Quick Start Guide

This guide will help you deploy db-optimizer to production in under 5 minutes.

---

## Prerequisites

Choose your deployment method:

- **Docker Compose**: Docker + Docker Compose installed
- **Kubernetes**: kubectl + cluster access
- **Native**: Rust 1.75+ + PostgreSQL client libraries

---

## Option 1: Docker Compose (Recommended for Testing)

### Step 1: Clone and Configure

```bash
git clone https://github.com/YOUR_USERNAME/db-optimizer.git
cd db-optimizer

# Copy example config
cp config.example.yml config.yml

# Edit config.yml with your database credentials
vim config.yml

# If enabling the HTTP API, set an auth token for production.
# http_server:
#   enabled: true
#   bind_address: "127.0.0.1"  # or your pod IP
#   port: 9090
#   auth_token: "your-secret-token"
```

### Step 2: Deploy

```bash
# Start PostgreSQL (optional, if you don't have one)
docker-compose up -d postgres

# Run one-shot analysis
docker-compose --profile analyze up db-optimizer-analyze

# OR run as long-running agent
docker-compose --profile agent up -d db-optimizer-agent

# View logs
docker-compose logs -f db-optimizer-agent
```

### Step 3: Test Query Rewriter

```bash
# Create a test query file
echo "SELECT * FROM users WHERE id = 1" > test.sql

# Run rewriter
docker-compose run --rm db-optimizer-rewrite rewrite-sql --file /scripts/test.sql
```

---

## Option 2: Kubernetes (Production)

### Step 1: Build and Push Image

```bash
# Build image
docker build -t your-registry/db-optimizer:v1.0.0 .

# Push to registry
docker push your-registry/db-optimizer:v1.0.0
```

### Step 2: Update Manifests

Edit `k8s/deployment.yaml`:

```yaml
# Update image reference
image: your-registry/db-optimizer:v1.0.0

# Update ConfigMap with your database details
data:
  config.yml: |
    postgres:
      host: your-postgres-service
      port: 5432
      database: your-database
      user: your-user

# Update Secret with actual credentials
stringData:
  postgres-password: "your-secure-password"
```

### Step 3: Deploy

```bash
# Apply manifests
kubectl apply -f k8s/deployment.yaml

# Verify deployment
kubectl get pods -n db-optimizer
kubectl logs -n db-optimizer -l app=db-optimizer -f

# Check recommendations
kubectl exec -n db-optimizer deployment/db-optimizer-agent -- \
  db-optimizer analyze --config /config/config.yml
```

### Step 4: Access Logs and Data

```bash
# View agent logs
kubectl logs -n db-optimizer -l component=agent -f

# Access persistent data
kubectl exec -n db-optimizer deployment/db-optimizer-agent -- \
  ls -la /data

# If the HTTP API is enabled and auth is configured:
# curl -H 'Authorization: Bearer YOUR_TOKEN' http://<pod-ip>:9090/api/v1/health

# Run manual analysis
kubectl create job --from=cronjob/db-optimizer-analyze manual-run -n db-optimizer
```

---

## Option 3: Kubernetes Sidecar Pattern

Deploy db-optimizer alongside your existing Postgres StatefulSet:

### Step 1: Update Your StatefulSet

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: postgres
spec:
  template:
    spec:
      containers:
      # Your existing Postgres container
      - name: postgres
        image: postgres:16
        # ... existing config ...
      
      # Add db-optimizer sidecar
      - name: db-optimizer
        image: your-registry/db-optimizer:v1.0.0
        command:
          - db-optimizer
          - agent
          - --config
          - /config/config.yml
        env:
        - name: DATABASE_URL
          value: "postgresql://user:pass@localhost:5432/mydb"
        volumeMounts:
        - name: optimizer-config
          mountPath: /config
        - name: optimizer-data
          mountPath: /data
        resources:
          requests:
            memory: "128Mi"
            cpu: "50m"
          limits:
            memory: "256Mi"
            cpu: "200m"
      
      volumes:
      - name: optimizer-config
        configMap:
          name: db-optimizer-config
      - name: optimizer-data
        emptyDir: {}
```

---

## Option 4: Native Binary

### Step 1: Build

```bash
# Install dependencies (Ubuntu/Debian)
sudo apt-get install -y libssl-dev libsqlite3-dev postgresql-client

# Build
cargo build --release

# Install binary
sudo cp target/release/db-optimizer /usr/local/bin/
```

### Step 2: Create Config

```bash
# Create config directory
sudo mkdir -p /etc/db-optimizer

# Copy and edit config
sudo cp config.example.yml /etc/db-optimizer/config.yml
sudo vim /etc/db-optimizer/config.yml
```

### Step 3: Create Systemd Service

```bash
# Create service file
sudo tee /etc/systemd/system/db-optimizer.service <<EOF
[Unit]
Description=PostgreSQL Database Optimizer Agent
After=network.target

[Service]
Type=simple
User=postgres
Group=postgres
ExecStart=/usr/local/bin/db-optimizer agent --config /etc/db-optimizer/config.yml
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable db-optimizer
sudo systemctl start db-optimizer

# Check status
sudo systemctl status db-optimizer
sudo journalctl -u db-optimizer -f
```

---

## Configuration Examples

### Observe-Only Mode (Safe for Production)

```yaml
operation_mode: observe_only

collection:
  interval_secs: 300  # Every 5 minutes
  pg_stat_statements: true
  table_stats: true
  index_stats: true

ebpf:
  enabled: false  # Disable if no privileged access
```

### Safe Auto-Apply Mode

```yaml
operation_mode: safe_auto

safety:
  max_index_size_mb: 1000.0
  max_ddl_lock_time_secs: 30
  protected_schemas:
    - pg_catalog
    - information_schema
    - auth  # Your critical schemas
  protected_tables:
    - users  # Critical tables

maintenance_window:
  allowed_hours_utc: [2, 6]  # 2 AM - 6 AM UTC
  allowed_days:
    - saturday
    - sunday

notifications:
  slack_webhook_url: ${SLACK_WEBHOOK_URL}
```

### Aggressive Mode (Staging Only)

```yaml
operation_mode: aggressive

# Use with extreme caution!
# Only for staging/development environments
```

---

## Monitoring and Maintenance

### View Recommendations

```bash
# Docker Compose
docker-compose exec db-optimizer-agent db-optimizer analyze --config /config/config.yml

# Kubernetes
kubectl exec -n db-optimizer deployment/db-optimizer-agent -- \
  db-optimizer analyze --config /config/config.yml

# Native
db-optimizer analyze --config /etc/db-optimizer/config.yml
```

### Check Historical Data

```bash
# The SQLite database contains all historical data
sqlite3 /data/db-optimizer.db

sqlite> SELECT * FROM recommendations ORDER BY created_at DESC LIMIT 10;
sqlite> SELECT * FROM query_stats_snapshots;
sqlite> .quit
```

### Backup and Restore

```bash
# Backup persistent data
tar -czf db-optimizer-backup-$(date +%Y%m%d).tar.gz /data/db-optimizer.db

# Restore
tar -xzf db-optimizer-backup-20241115.tar.gz -C /data/
```

---

## Troubleshooting

### Issue: Cannot connect to PostgreSQL

```bash
# Check connection from container/pod
psql "postgresql://user:pass@host:5432/dbname"

# Verify pg_stat_statements is installed
psql -c "SELECT * FROM pg_stat_statements LIMIT 1;"

# If not, install it
psql -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"
```

### Issue: eBPF not working

```bash
# eBPF requires privileged mode or CAP_BPF capability
# For Kubernetes, add:
securityContext:
  privileged: true  # Or use specific capabilities

# For Docker:
docker run --privileged ...

# Or disable eBPF in config:
ebpf:
  enabled: false
```

### Issue: Out of memory

```bash
# Increase memory limits
# Kubernetes:
resources:
  limits:
    memory: "512Mi"  # Increase from 256Mi

# Docker Compose:
deploy:
  resources:
    limits:
      memory: 512M
```

### Issue: Slow query analysis

```bash
# Reduce collection frequency
collection:
  interval_secs: 600  # Every 10 minutes instead of 1 minute

# Or limit analyzed queries
# Edit config to only analyze slowest N queries
```

---

## Production Checklist

- [ ] Configure database credentials securely (use secrets/env vars)
- [ ] Set appropriate operation mode (`observe_only` for first deployment)
- [ ] Configure protected schemas and tables
- [ ] Set up maintenance windows
- [ ] Configure Slack/webhook notifications
- [ ] Set resource limits (CPU, memory)
- [ ] Enable persistence (SQLite or Postgres backend)
- [ ] Test backup and restore procedures
- [ ] Monitor logs and recommendations
- [ ] Schedule periodic reviews of recommendations

---

## Performance Tuning

### For Large Databases (>1TB)

```yaml
collection:
  interval_secs: 600  # Less frequent collection
  sample_rate: 0.1    # Sample 10% of queries

persistence:
  backend: postgres  # Use Postgres instead of SQLite
  retention_days: 30  # Keep data for 30 days
```

### For High-Frequency OLTP

```yaml
collection:
  interval_secs: 60  # Frequent monitoring
  pg_stat_statements: true
  
ebpf:
  enabled: true  # For system-level insights
  sampling_interval_ms: 1000  # Less frequent eBPF sampling
```

### For Batch/Analytics Workloads

```yaml
collection:
  interval_secs: 300
  focus_on_slow_queries: true
  slow_query_threshold_ms: 1000
```

---

## Getting Help

- **Documentation**: See `docs/` directory
- **Issues**: https://github.com/YOUR_USERNAME/db-optimizer/issues
- **Discussions**: https://github.com/YOUR_USERNAME/db-optimizer/discussions

---

## Next Steps

1. ✅ Deploy in observe-only mode
2. ✅ Monitor for 24-48 hours
3. ✅ Review recommendations
4. ✅ Apply safe recommendations manually
5. ✅ Consider enabling safe_auto mode for non-critical systems
6. ✅ Set up alerting and dashboards
7. ✅ Schedule regular reviews

**Happy Optimizing!** 🚀
