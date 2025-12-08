# Quick Start: Enhanced AI Dashboard

## 🚀 Get Started in 5 Minutes

### Step 1: Set Up AI Provider (Choose One)

**Option A: OpenAI (Easiest, Best Quality)**
```bash
export OPENAI_API_KEY="sk-proj-..." # Get from https://platform.openai.com/api-keys
export DBO_AI_ENABLED=true
```

**Option B: Ollama (Free, Local)**
```bash
# Install Ollama: https://ollama.ai
ollama pull llama3.2
export DBO_AI_PROVIDER="ollama"
export DBO_AI_ENABLED=true
```

### Step 2: Start the Server
```bash
# Build (first time only)
cargo build --release

# Start HTTP server
./target/release/pg-query-advisor \
  --http-server \
  --http-port 8080 \
  --database-url "postgresql://user:pass@localhost/yourdb"
```

### Step 3: Open Dashboard
```
http://localhost:8080/dashboard-enhanced.html
```

**That's it! You now have:**
- ✅ AI-powered fleet dashboard
- ✅ Query optimizer
- ✅ Config tuning wizard
- ✅ Lock analysis
- ✅ AI chat assistant

---

## 🎯 Try These Features First

### 1. Ask AI a Question
1. Click "🤖 AI Assistant" button (top-right)
2. Type: "What issues do you see in my databases?"
3. Wait 2-5 seconds
4. Get expert analysis!

### 2. Optimize a Slow Query
1. Go to "🚀 Query Optimizer" tab
2. Paste a slow query
3. Click "Optimize with AI"
4. See 50-90% improvement suggestions
5. Copy optimized SQL

### 3. Auto-Tune Configuration
1. Go to "⚙️ Config Tuning" tab
2. Click "Generate Optimal Configuration"
3. Download `postgresql.conf`
4. Apply to your database

---

## 📊 Dashboard Overview

**5 Main Sections:**

1. **🏢 Fleet Tab** (Default)
   - See all databases at a glance
   - Click any card for AI recommendations
   - Track potential cost savings

2. **🚀 Query Optimizer**
   - Paste slow query
   - Get instant optimizations
   - See improvement estimates

3. **⚙️ Config Tuning**
   - Auto-detect workload type
   - Generate optimal postgres config
   - One-click download

4. **🔒 Lock Analysis**
   - Real-time lock monitoring
   - See blocking queries
   - Terminate problematic locks

5. **🤖 AI Assistant**
   - Always available (bottom-right)
   - Ask anything in plain English
   - Get expert guidance

---

## 💡 Example Questions for AI

**Diagnostics:**
- "Why is my database slow?"
- "Which database needs attention first?"
- "What's causing high CPU usage?"

**Recommendations:**
- "What indexes should I create?"
- "How can I improve query performance?"
- "What's the most expensive query?"

**Explanations:**
- "Explain this health score"
- "Why is this recommendation important?"
- "What does lock contention mean?"

---

## 🔧 Troubleshooting

**Dashboard not loading?**
```bash
# Check server is running
curl http://localhost:8080/api/v1/health

# Should return: {"status":"ok"}
```

**AI not working?**
```bash
# Verify environment variables
echo $OPENAI_API_KEY
echo $DBO_AI_ENABLED

# Check API key is valid
curl https://api.openai.com/v1/models \
  -H "Authorization: Bearer $OPENAI_API_KEY"
```

**No data showing?**
```bash
# Register a database first
curl -X POST http://localhost:8080/api/v1/fleet \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Database",
    "connection_string": "postgresql://user:pass@host:5432/db",
    "environment": "production"
  }'
```

---

## 💰 Cost Estimates

**Using OpenAI (gpt-4o-mini):**
- Per chat message: ~$0.005
- Per query optimization: ~$0.01
- 100 queries/day: ~$30/month

**Using Ollama (local):**
- Cost: $0 (free!)
- Trade-off: Slower, less accurate

---

## 📚 Learn More

**Full Documentation:**
- Complete guide: `docs/ENHANCED_DASHBOARD_GUIDE.md`
- Feature summary: `DASHBOARD_ENHANCEMENTS_SUMMARY.md`
- Original README: `README.md`

**API Documentation:**
- When server running: `http://localhost:8080/api/v1`

---

## 🎉 What's Next?

1. **Explore the dashboard** - Click around, try features
2. **Add more databases** - Monitor your entire fleet
3. **Set up automation** - Use API for CI/CD integration
4. **Configure alerts** - Get notified of issues
5. **Share with team** - Multiply the impact

**Need help?** See troubleshooting section in `docs/ENHANCED_DASHBOARD_GUIDE.md`

---

## 🌟 Pro Tips

1. **Use keyboard shortcuts**
   - `Ctrl/Cmd + K`: Focus AI chat
   - `Esc`: Close modals

2. **Bookmark quick prompts**
   - Create saved questions in AI chat
   - Reuse common optimization patterns

3. **Monitor trends**
   - Check dashboard daily
   - Track health score improvements
   - Measure cost savings

4. **Start small**
   - Test on staging first
   - Validate AI recommendations
   - Roll out gradually

---

**Happy optimizing! 🚀**
