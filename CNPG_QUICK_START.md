# CloudNativePG Plugin - Quick Start Guide

**Status:** Beta v0.1.0  
**Date:** December 16, 2025

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
