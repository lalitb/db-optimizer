# CNPG Plugin Launch Readiness Assessment

**Date:** December 16, 2025  
**Assessment:** ⚠️ **INCOMPLETE - 70% Ready**  
**ETA to Launch:** 3-5 days

---

## ✅ What's Working (Completed)

### 1. Core Plugin Implementation ✅
- **Files:**
  - `src/cnpg_plugin/mod.rs` - Entry point
  - `src/cnpg_plugin/server.rs` - gRPC service implementation
  - `src/cnpg_plugin/proto.rs` - Protocol definitions (manual traits)
  - `src/cnpg_plugin/config.rs` - Configuration loading

- **Capabilities Implemented:**
  - ✅ Health monitoring with 0-100 score
  - ✅ Performance analysis (using existing DatabaseCollector)
  - ✅ Index recommendations
  - ✅ Query rewriting capability
  - ✅ Auto-optimization toggle
  - ✅ Cost analysis
  - ✅ eBPF instrumentation (Linux only)

- **gRPC Services:**
  - ✅ `lifecycle()` - Handles create/update/delete/healthcheck
  - ✅ `get_capabilities()` - Returns plugin features
  - ⚠️ WAL, Backup, Reconciler services (defined but not implemented)

### 2. Kubernetes Deployment ✅
- **Files:**
  - `k8s/cnpg-plugin.yaml` - Complete deployment manifest
  - `Dockerfile.cnpg` - Multi-stage build optimized
  - `charts/db-optimizer/` - Helm chart structure exists

- **Features:**
  - ✅ Deployment with 1 replica
  - ✅ Service exposing port 80 (maps to 8080)
  - ✅ ServiceAccount with RBAC
  - ✅ Resource limits (512Mi mem, 500m CPU)
  - ✅ Health probes ready to add

### 3. Testing Infrastructure ✅
- **File:** `tests/cnpg_integration_test.sh`
- ✅ Kind cluster setup
- ✅ CNPG operator installation
- ✅ Plugin deployment
- ✅ Test database creation
- ✅ Log verification

### 4. Documentation ✅
- `docs/CNPG_INTEGRATION.md` - Installation guide
- `CNPG_TARGET_CUSTOMERS.txt` - GTM strategy
- `CNPG_ROADMAP.txt` - Development plan

---

## ❌ What's Missing (Critical Blockers)

### 1. ❌ **NO CLI COMMAND TO START PLUGIN**

**Problem:** The main.rs doesn't have a `cnpg-plugin` subcommand!

**Current State:**
```rust
enum Commands {
    Analyze(AnalyzeOptions),
    Agent(AgentOptions),
    RewriteSql(RewriteSqlOptions),
    Serve(ServeOptions),
    ConfigTune(ConfigTuneOptions),
    ConfigValidate(ConfigValidateOptions),
    Ask(AskOptions),
    // ❌ Missing: CnpgPlugin(CnpgPluginOptions)
}
```

**Dockerfile expects:**
```dockerfile
CMD ["cnpg-plugin"]  # ❌ This command doesn't exist!
```

**Fix Required:** Add CNPG plugin subcommand in `src/main.rs`

---

### 2. ❌ **PROTO FILES NOT COMPILED**

**Problem:** Using manual trait definitions instead of official protobuf compilation.

**Current:**
- `src/cnpg_plugin/proto.rs` - Manual trait definitions
- `src/cnpg_plugin/proto/*.proto` - Proto files exist but unused

**Issues:**
1. No `tonic-build` in `build.rs`
2. No actual gRPC server implementation
3. Server code has placeholder comment:
   ```rust
   // In production, this would start the actual tonic server:
   // tonic::transport::Server::builder()
   //     .add_service(OperatorServer::new(service))
   //     .serve(addr)
   //     .await?;
   ```

**Fix Required:**
1. Add proto compilation to `build.rs`
2. Implement actual tonic server
3. OR: Get official CNPG-I proto files (they may not be public yet)

---

### 3. ❌ **NO CI/CD FOR CNPG**

**Problem:** Current `.github/workflows/ci.yml` doesn't test CNPG feature.

**Missing:**
- No `cargo build --features cnpg` in CI
- No Docker build for `Dockerfile.cnpg`
- No integration test execution
- No image push to registry

**Fix Required:** Add CNPG-specific CI job

---

### 4. ❌ **NO ACTUAL CNPG OPERATOR INTEGRATION**

**Problem:** Plugin isn't registered with CNPG operator.

**What's needed:**
1. CNPG operator config to point to plugin endpoint
2. Plugin discovery mechanism
3. Authentication/authorization between operator and plugin
4. Connection string injection from operator

**Current status:** Documentation mentions it but no implementation.

---

### 5. ⚠️ **INCOMPLETE SERVER IMPLEMENTATION**

**Issues in `src/cnpg_plugin/server.rs`:**

```rust
pub async fn start_grpc_server(...) -> anyhow::Result<()> {
    // ...
    let service = VaanyzeOperatorService::new(config, state);
    
    // ❌ Server is created but never actually started!
    let _ = service;  // This just discards it!
    
    // Just waits for Ctrl+C
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
```

**Missing:**
- Actual `tonic::transport::Server::builder()` call
- gRPC route registration
- Health check endpoint
- Graceful shutdown handling

---

### 6. ⚠️ **CONNECTION STRING HANDLING**

**Problem:** Plugin expects connection string from operator, but:
- No validation
- No secret management
- No connection pooling per cluster
- No multi-cluster support

---

### 7. ⚠️ **NO OBSERVABILITY**

**Missing:**
- Prometheus metrics endpoint
- Structured logging
- Trace IDs for debugging
- Health/readiness probes in K8s

---

## 🔧 Quick Fixes Needed (Ranked by Priority)

### Priority 1: Make It Runnable (2-3 hours)

#### Fix 1: Add CLI Subcommand
**File:** `src/main.rs`

Add to `Commands` enum:
```rust
/// Run as CNPG plugin server
CnpgPlugin(CnpgPluginOptions),
```

Add options struct:
```rust
#[derive(Parser)]
struct CnpgPluginOptions {
    /// gRPC port
    #[arg(short, long, default_value = "8080")]
    port: u16,
    
    /// Config file path
    #[arg(short, long)]
    config: Option<PathBuf>,
}
```

Add match arm in main:
```rust
#[cfg(feature = "cnpg")]
Commands::CnpgPlugin(opts) => {
    pg_query_advisor::cnpg_plugin::run_cnpg_plugin(opts.port, opts.config).await?;
}
```

#### Fix 2: Implement Actual gRPC Server
**File:** `src/cnpg_plugin/server.rs`

Replace placeholder with:
```rust
use tonic::transport::Server;
use crate::cnpg_plugin::proto::OperatorServer;

pub async fn start_grpc_server(...) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let service = VaanyzeOperatorService::new(config, state);

    info!("Starting gRPC server on {}", addr);
    
    Server::builder()
        .add_service(OperatorServer::new(service))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutting down gRPC server");
        })
        .await?;
    
    Ok(())
}
```

But wait - `OperatorServer` needs to be a proper tonic generated struct...

---

### Priority 2: Fix Proto Compilation (4-6 hours)

**Option A: Use Official CNPG-I Protos** (Preferred)
1. Clone https://github.com/cloudnative-pg/cnpg-i
2. Add proto files to `proto/cnpg-i/`
3. Update `build.rs`:
   ```rust
   fn main() {
       #[cfg(feature = "cnpg")]
       {
           tonic_build::configure()
               .build_server(true)
               .build_client(false)
               .compile(
                   &["proto/cnpg-i/operator.proto"],
                   &["proto/cnpg-i"],
               )
               .unwrap();
       }
   }
   ```
4. Replace manual traits with generated code

**Option B: Keep Manual Implementation** (Faster for MVP)
1. Implement proper tonic server manually
2. Add `#[tonic::async_trait]` to trait impls
3. Manually register routes
4. Accept this as "CNPG-inspired" not "CNPG-I compliant"

**Recommendation:** Option B for first beta, Option A for v1.0

---

### Priority 3: Add CI for CNPG (1-2 hours)

**File:** `.github/workflows/cnpg-ci.yml`

```yaml
name: CNPG Plugin CI

on:
  push:
    branches: [ main, develop ]
  pull_request:
    branches: [ main, develop ]

jobs:
  build-cnpg:
    name: Build CNPG Plugin
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        profile: minimal
        toolchain: stable
        override: true
    
    - name: Install protobuf compiler
      run: sudo apt-get update && sudo apt-get install -y protobuf-compiler
    
    - name: Build CNPG feature
      run: cargo build --release --features cnpg
    
    - name: Test CNPG feature
      run: cargo test --features cnpg
    
    - name: Build Docker image
      run: docker build -f Dockerfile.cnpg -t vaanyze/cnpg-plugin:test .
    
    - name: Test Docker image
      run: |
        docker run --rm vaanyze/cnpg-plugin:test cnpg-plugin --help

  integration-test:
    name: CNPG Integration Test
    runs-on: ubuntu-latest
    needs: build-cnpg
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Kind
      run: |
        curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.20.0/kind-linux-amd64
        chmod +x ./kind
        sudo mv ./kind /usr/local/bin/kind
    
    - name: Run integration test
      run: ./tests/cnpg_integration_test.sh
```

---

### Priority 4: Production Readiness (3-4 hours)

#### Add Health Checks
**File:** `k8s/cnpg-plugin.yaml`

Add to deployment:
```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 8080
  initialDelaySeconds: 10
  periodSeconds: 30

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 10
```

Implement in server:
```rust
// Add HTTP health endpoint alongside gRPC
let health_server = axum::Router::new()
    .route("/health", get(|| async { "OK" }))
    .route("/ready", get(health_check));

tokio::spawn(async move {
    axum::Server::bind(&"0.0.0.0:8081".parse().unwrap())
        .serve(health_server.into_make_service())
        .await
});
```

---

## 📊 Launch Checklist

### Must Have (Blocking Launch)
- [ ] CLI `cnpg-plugin` subcommand
- [ ] Actual gRPC server running
- [ ] Docker image builds successfully
- [ ] Docker image starts without crashing
- [ ] Health endpoint responds
- [ ] Basic end-to-end test passes

### Should Have (Beta Quality)
- [ ] CI builds CNPG feature
- [ ] CI runs integration tests
- [ ] Documentation for manual operator integration
- [ ] Error handling for DB connection failures
- [ ] Logging shows cluster events
- [ ] Dockerfile optimized (currently 2-stage but can improve)

### Nice to Have (v1.0)
- [ ] Official CNPG-I proto compilation
- [ ] Registered in CNPG plugin directory
- [ ] Helm chart published
- [ ] Multi-cluster support
- [ ] Prometheus metrics
- [ ] Distributed tracing
- [ ] Auto-discovery of CNPG clusters

---

## 🚀 Recommended Launch Plan

### Day 1-2: Core Functionality (8-12 hours)
1. ✅ Add `cnpg-plugin` CLI command (1 hour)
2. ✅ Implement real gRPC server with manual proto (2-3 hours)
3. ✅ Add health endpoints (1 hour)
4. ✅ Test Docker build end-to-end (2 hours)
5. ✅ Manual test with local Kind cluster (2-3 hours)

### Day 3: CI/CD (4-6 hours)
1. ✅ Add CNPG CI workflow (1 hour)
2. ✅ Fix any CI failures (2-3 hours)
3. ✅ Set up Docker Hub auto-build (1 hour)
4. ✅ Tag v0.1.0-beta1 (30 min)

### Day 4: Documentation & Testing (4-6 hours)
1. ✅ Update CNPG_INTEGRATION.md with actual steps (1 hour)
2. ✅ Record demo video (2 hours)
3. ✅ Write beta announcement (1 hour)
4. ✅ Create GitHub release (30 min)
5. ✅ Post in CNPG Slack (30 min)

### Day 5: Beta Launch
1. ✅ Onboard first 3 beta users
2. ✅ Monitor logs and fix critical bugs
3. ✅ Iterate on feedback

---

## 💡 Critical Decisions Needed

### 1. Proto Strategy
**Question:** Manual traits (fast) or official CNPG-I protos (correct)?

**Recommendation:** 
- **Beta:** Manual traits (ship in 3 days)
- **v1.0:** Migrate to official protos (ship in 2-3 weeks)

### 2. Operator Integration
**Question:** How do users register the plugin?

**Options:**
A. Manual ConfigMap edit (what we have docs for)
B. Automatic discovery via K8s API
C. Operator helm value override

**Recommendation:** Start with A, add B in v0.2

### 3. Multi-Cluster
**Question:** Support multiple clusters from single plugin instance?

**Answer:** Yes, but v0.2 feature. v0.1 = 1:1 mapping.

---

## 🎯 Success Criteria for Beta Launch

### Technical:
- [ ] Docker image runs without crashing
- [ ] Plugin responds to health checks
- [ ] Logs show connection to at least 1 database
- [ ] No panics in first 24 hours

### Business:
- [ ] 3 beta signups in first week
- [ ] 1 successful deployment in production
- [ ] 0 critical bugs
- [ ] Positive feedback in CNPG Slack

---

## ⚡ Next Actions (Right Now)

1. **Add CLI subcommand** (src/main.rs) - 30 min
2. **Implement gRPC server** (src/cnpg_plugin/server.rs) - 2 hours
3. **Test Docker build** - 30 min
4. **Fix any compile errors** - 1 hour
5. **Manual K8s test** - 1 hour

**Total: 5 hours to working beta**

Then add CI tomorrow and launch by end of week.

Ready to start? Let's fix #1 first! 🚀
