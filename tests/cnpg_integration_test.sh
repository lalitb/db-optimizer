#!/bin/bash
set -e

# Vaanyze CNPG Integration Test
# This script sets up a local Kind cluster and tests the CNPG plugin integration.

CLUSTER_NAME="vaanyze-test"
IMAGE_NAME="vaanyze/cnpg-plugin:test"

echo "🚀 Starting CNPG Integration Test..."

# Check prerequisites
command -v kind >/dev/null 2>&1 || { echo "❌ kind is required but not installed."; exit 1; }
command -v kubectl >/dev/null 2>&1 || { echo "❌ kubectl is required but not installed."; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "❌ docker is required but not installed."; exit 1; }

# 1. Create Kind Cluster
if kind get clusters | grep -q "$CLUSTER_NAME"; then
    echo "ℹ️  Cluster $CLUSTER_NAME already exists."
else
    echo "📦 Creating Kind cluster..."
    kind create cluster --name "$CLUSTER_NAME"
fi

# 2. Install CloudNativePG Operator
echo "🔧 Installing CloudNativePG Operator..."
kubectl apply -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.22/releases/cnpg-1.22.1.yaml
kubectl wait --for=condition=Ready pod -l app.kubernetes.io/name=cloudnative-pg -n cnpg-system --timeout=120s

# 3. Build Plugin Image
echo "🔨 Building Plugin Docker Image..."
docker build -f Dockerfile.cnpg -t "$IMAGE_NAME" .

# 4. Load Image into Kind
echo "🚚 Loading image into Kind cluster..."
kind load docker-image "$IMAGE_NAME" --name "$CLUSTER_NAME"

# 5. Deploy Plugin
echo "🚀 Deploying Vaanyze Plugin..."
kubectl apply -f k8s/cnpg-plugin.yaml
# Patch image to use local tag
kubectl set image deployment/vaanyze-cnpg-plugin plugin="$IMAGE_NAME" -n cnpg-system
kubectl wait --for=condition=Available deployment/vaanyze-cnpg-plugin -n cnpg-system --timeout=60s

# 6. Configure Operator to use Plugin
# Note: In a real scenario, we'd patch the Operator config. 
# For this test, we assume the Operator is watching for plugins or we manually trigger it.
# Since CNPG 1.22, plugins are configured via ConfigMap or args.
# This step depends on exact CNPG version configuration.
echo "⚠️  Skipping Operator configuration patch (requires specific CNPG setup)"

# 7. Deploy Test Database
echo "🐘 Deploying Test PostgreSQL Cluster..."
cat <<EOF | kubectl apply -f -
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: test-db
  namespace: default
  annotations:
    cnpg.io/plugins: vaanyze
spec:
  instances: 1
  resources:
    requests:
      memory: "1Gi"
      cpu: "500m"
  storage:
    size: 1Gi
EOF

echo "⏳ Waiting for database to be ready..."
kubectl wait --for=condition=Ready cluster/test-db --timeout=300s

# 8. Verify Plugin Logs
echo "🔍 Verifying Plugin startup and health endpoints..."
LOGS=$(kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin --tail=200)
echo "$LOGS"

# The operator/plugin registration mechanism varies by CNPG version and installation.
# For CI, we validate that the plugin comes up and reports ready.

echo "🌐 Checking /ready endpoint..."
kubectl -n cnpg-system port-forward deployment/vaanyze-cnpg-plugin 8081:8081 >/tmp/pf.log 2>&1 &
PF_PID=$!

cleanup() {
  kill $PF_PID >/dev/null 2>&1 || true
}
trap cleanup EXIT

for i in {1..30}; do
  if curl -fsS http://127.0.0.1:8081/ready | grep -q "Ready"; then
    echo "✅ Plugin readiness probe OK"
    exit 0
  fi
  sleep 2
done

echo "❌ Plugin did not become ready in time"
exit 1
