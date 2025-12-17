# Systemd Deployment Guide

This directory contains files for deploying db-optimizer as a systemd service on Linux systems.

## Files

- `db-optimizer.service` - Systemd service unit file
- `install.sh` - Installation script
- `README.md` - This file

## Quick Start

```bash
# 1. Build the release binary
cargo build --release

# 2. Install binary
sudo cp target/release/db-optimizer /usr/local/bin/

# 3. Run installation script
cd deployment/systemd
sudo ./install.sh

# 4. Configure database connection
sudo nano /etc/db-optimizer/environment
# Add: DBO_DATABASE_URL=postgresql://user:pass@localhost/db

# 5. Start service
sudo systemctl enable db-optimizer
sudo systemctl start db-optimizer

# 6. Check status
sudo systemctl status db-optimizer
sudo journalctl -u db-optimizer -f
```

## Environment Variables

The service supports two ways to configure environment variables:

### 1. Environment File (Recommended)

Create `/etc/db-optimizer/environment`:

```bash
# Database connection (REQUIRED)
DBO_DATABASE_URL=postgresql://user:password@localhost:5432/mydb

# Operation mode
DBO_OPERATION_MODE=observe_only  # observe_only | safe_auto | aggressive

# Guardrails (System resource thresholds)
DBO_GUARDRAILS_ENABLED=true
DBO_GUARDRAILS_MEMORY_THRESHOLD=90
DBO_GUARDRAILS_CPU_THRESHOLD=85
DBO_GUARDRAILS_DISK_THRESHOLD=95
DBO_GUARDRAILS_MAX_CONNECTIONS=1000

# eBPF (Requires CAP_BPF or root)
DBO_EBPF_ENABLED=false

# HTTP API
DBO_HTTP_ENABLED=true
DBO_HTTP_PORT=9090

# Notifications
DBO_SLACK_WEBHOOK=https://hooks.slack.com/services/YOUR/WEBHOOK/URL
```

### 2. Service Override

Or use systemd drop-in files:

```bash
sudo systemctl edit db-optimizer
```

Add:
```ini
[Service]
Environment="DBO_DATABASE_URL=postgresql://..."
Environment="DBO_OPERATION_MODE=safe_auto"
```

## Available Environment Variables

### Core Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `DBO_DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `DBO_OPERATION_MODE` | `observe_only` | Operation mode: `observe_only`, `safe_auto`, `aggressive` |

### Guardrails (System Resource Limits)

| Variable | Default | Description |
|----------|---------|-------------|
| `DBO_GUARDRAILS_ENABLED` | `true` | Enable/disable guardrails |
| `DBO_GUARDRAILS_MEMORY_THRESHOLD` | `90` | Memory threshold (%) - stop auto-apply if exceeded |
| `DBO_GUARDRAILS_CPU_THRESHOLD` | `85` | CPU threshold (%) - stop auto-apply if exceeded |
| `DBO_GUARDRAILS_DISK_THRESHOLD` | `95` | Disk threshold (%) - stop auto-apply if exceeded |
| `DBO_GUARDRAILS_MAX_CONNECTIONS` | `1000` | Max connections - stop auto-apply if exceeded |

### eBPF Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `DBO_EBPF_ENABLED` | `false` | Enable eBPF metrics collection (requires privileges) |

### HTTP Server

| Variable | Default | Description |
|----------|---------|-------------|
| `DBO_HTTP_ENABLED` | `false` | Enable HTTP API server |
| `DBO_HTTP_PORT` | `9090` | HTTP server port |

### Notifications

| Variable | Default | Description |
|----------|---------|-------------|
| `DBO_SLACK_WEBHOOK` | _(none)_ | Slack webhook URL for notifications |

## Security Considerations

### File Permissions

The installation script sets up:
- User: `db-optimizer` (system user, no shell)
- Config dir: `/etc/db-optimizer` (755)
- Data dir: `/var/lib/db-optimizer` (750, owned by db-optimizer)
- Log dir: `/var/log/db-optimizer` (750, owned by db-optimizer)

### SELinux

If using SELinux, you may need to adjust policies:

```bash
# Allow network connections
setsebool -P httpd_can_network_connect 1

# If eBPF enabled
setsebool -P domain_can_mmap_files 1
```

### eBPF Privileges

For eBPF metrics, the service needs either:
- Run as root (not recommended)
- CAP_BPF capability (recommended)

To grant CAP_BPF:

```bash
sudo setcap cap_bpf,cap_perfmon=ep /usr/local/bin/db-optimizer
```

Update service file:
```ini
[Service]
AmbientCapabilities=CAP_BPF CAP_PERFMON
```

## Monitoring

### View Logs

```bash
# Follow logs in real-time
sudo journalctl -u db-optimizer -f

# View recent logs
sudo journalctl -u db-optimizer -n 100

# Filter by priority
sudo journalctl -u db-optimizer -p err

# Export logs
sudo journalctl -u db-optimizer --since today > db-optimizer-logs.txt
```

### Check Status

```bash
# Service status
sudo systemctl status db-optimizer

# Resource usage
sudo systemctl show db-optimizer --property=CPUUsageNSec,MemoryCurrent

# Restart service
sudo systemctl restart db-optimizer

# Reload configuration
sudo systemctl reload db-optimizer
```

### Metrics

If HTTP server is enabled, metrics are available at:

```bash
curl http://localhost:9090/health
curl http://localhost:9090/stats/summary
```

## Troubleshooting

### Service Won't Start

Check logs:
```bash
sudo journalctl -u db-optimizer -xe
```

Common issues:
1. **Database connection failed**: Verify `DBO_DATABASE_URL` in `/etc/db-optimizer/environment`
2. **Permission denied**: Check file permissions on config/data/log directories
3. **Port already in use**: Change `DBO_HTTP_PORT` or disable conflicting service
4. **Binary not found**: Ensure `/usr/local/bin/db-optimizer` exists and is executable

### High Resource Usage

Check guardrails are enabled:
```bash
# Verify in config
sudo cat /etc/db-optimizer/config.yml | grep -A 10 guardrails

# Or set via environment
sudo nano /etc/db-optimizer/environment
# Add: DBO_GUARDRAILS_ENABLED=true
sudo systemctl restart db-optimizer
```

### Logs Not Appearing

Check journald configuration:
```bash
# Verify service is logging
sudo systemctl status db-optimizer

# Check journal size
sudo journalctl --disk-usage

# Increase retention
sudo nano /etc/systemd/journald.conf
# Set: SystemMaxUse=1G
sudo systemctl restart systemd-journald
```

## Updating

To update to a new version:

```bash
# 1. Build new version
cd /path/to/db-optimizer
git pull
cargo build --release

# 2. Stop service
sudo systemctl stop db-optimizer

# 3. Replace binary
sudo cp target/release/db-optimizer /usr/local/bin/

# 4. Restart service
sudo systemctl start db-optimizer

# 5. Verify
sudo systemctl status db-optimizer
```

## Uninstallation

```bash
# Stop and disable service
sudo systemctl stop db-optimizer
sudo systemctl disable db-optimizer

# Remove files
sudo rm /etc/systemd/system/db-optimizer.service
sudo rm -rf /etc/db-optimizer
sudo rm -rf /var/lib/db-optimizer
sudo rm -rf /var/log/db-optimizer
sudo rm /usr/local/bin/db-optimizer

# Remove user
sudo userdel db-optimizer

# Reload systemd
sudo systemctl daemon-reload
```

## Production Recommendations

For production deployments:

1. **Use config file over environment variables** for complex setups
2. **Enable guardrails** to prevent resource exhaustion
3. **Set up log rotation** if using file logging
4. **Monitor the service** with your observability platform
5. **Use `observe_only` mode initially** to validate behavior
6. **Configure maintenance windows** for auto-apply operations
7. **Set up Slack notifications** for important events
8. **Test in staging first** before production deployment
9. **Document your specific configuration** for your team
10. **Regular backups** of `/etc/db-optimizer/config.yml`

## Examples

### Development Setup

```bash
# Observe-only mode with HTTP API
sudo tee /etc/db-optimizer/environment <<EOF
DBO_DATABASE_URL=postgresql://dev:dev@localhost/devdb
DBO_OPERATION_MODE=observe_only
DBO_HTTP_ENABLED=true
DBO_HTTP_PORT=9090
DBO_GUARDRAILS_ENABLED=false
EOF

sudo systemctl restart db-optimizer
```

### Production Setup (Conservative)

```bash
# Safe auto-apply with strict guardrails
sudo tee /etc/db-optimizer/environment <<EOF
DBO_DATABASE_URL=postgresql://optimizer:PASSWORD@prod-db:5432/production
DBO_OPERATION_MODE=safe_auto
DBO_GUARDRAILS_ENABLED=true
DBO_GUARDRAILS_MEMORY_THRESHOLD=85
DBO_GUARDRAILS_CPU_THRESHOLD=80
DBO_GUARDRAILS_DISK_THRESHOLD=90
DBO_HTTP_ENABLED=true
DBO_SLACK_WEBHOOK=https://hooks.slack.com/services/YOUR/WEBHOOK
EOF

sudo systemctl restart db-optimizer
```

### Staging Setup (Aggressive Testing)

```bash
# Aggressive mode for testing
sudo tee /etc/db-optimizer/environment <<EOF
DBO_DATABASE_URL=postgresql://optimizer:PASSWORD@staging-db:5432/staging
DBO_OPERATION_MODE=aggressive
DBO_EBPF_ENABLED=true
DBO_HTTP_ENABLED=true
DBO_GUARDRAILS_MEMORY_THRESHOLD=95
EOF

sudo systemctl restart db-optimizer
```

## Support

For issues and questions:
- GitHub: https://github.com/lalitb/db-optimizer/issues
- Documentation: https://github.com/lalitb/db-optimizer/blob/main/README.md
