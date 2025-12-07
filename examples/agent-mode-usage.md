# Agent Mode Usage Examples

## Quick Start

### 1. Test Agent Mode (Dry-Run)

```bash
# Run agent mode with auto-apply in dry-run (safe - no changes made)
pg-query-advisor \
  --database-url postgresql://user:pass@localhost/mydb \
  --agent \
  --auto-apply \
  --dry-run=true \
  --interval-minutes 5

# Output:
🤖 PostgreSQL Query Health Advisor - Agent Mode
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
   Monitoring interval: 5 minutes
   Database: localhost/mydb
   Auto-apply: disabled
   Dry-run: yes
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📊 Loaded state: 0 optimizations applied previously

✅ Agent started successfully
   Press Ctrl+C to stop

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🔍 Running analysis cycle... (2025-11-02 09:52:00)
   📊 Found 3 new optimization opportunities
   ⏭️  Skipped: 3 (dry-run mode)
   💰 Potential savings: $450/month

# Wait 5 minutes...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🔍 Running analysis cycle... (2025-11-02 09:57:00)
   ✅ No new optimization opportunities found

# Press Ctrl+C to stop

🛑 Shutdown signal received
   Saving state and exiting...

Agent Statistics:
   Total optimizations applied: 0
   Uptime: 0.1 hours

✅ Agent stopped gracefully
```

### 2. Enable Auto-Apply (Production)

```bash
# Run with actual changes (disable dry-run)
pg-query-advisor \
  --database-url $DATABASE_URL \
  --agent \
  --auto-apply \
  --dry-run=false \
  --interval-minutes 15 \
  --min-savings 100 \
  --max-risk low \
  --slack-webhook $SLACK_WEBHOOK

# Output:
🤖 PostgreSQL Query Health Advisor - Agent Mode
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
   Monitoring interval: 15 minutes
   Database: prod.example.com/mydb
   Auto-apply: enabled
   Dry-run: no
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🔍 Running analysis cycle... (2025-11-02 10:00:00)
   📊 Found 2 new optimization opportunities
   ✅ Applied: 2
   💰 Potential savings: $280/month

# Slack notification sent:
# ✅ Optimization Successful
# Recommendation: Add index on users(email)
# Improvement: 75.3%
# Est. Savings: $180/month
```

## Deployment Options

### Option 1: Systemd Service (Linux)

```bash
# Create service file
sudo nano /etc/systemd/system/pg-advisor-agent.service
```

```ini
[Unit]
Description=PostgreSQL Query Advisor Agent
After=network.target postgresql.service
Documentation=https://github.com/yourorg/pg-query-advisor

[Service]
Type=simple
User=postgres
WorkingDirectory=/opt/pg-advisor
Environment="DATABASE_URL=postgresql://user:pass@localhost/mydb"
Environment="SLACK_WEBHOOK=https://hooks.slack.com/services/YOUR/WEBHOOK"
ExecStart=/usr/local/bin/pg-query-advisor \
  --agent \
  --auto-apply \
  --dry-run=false \
  --interval-minutes 15 \
  --min-savings 50 \
  --max-risk low \
