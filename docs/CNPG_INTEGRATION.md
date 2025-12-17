# Vaanyze CloudNativePG Integration Guide

This guide details how to deploy and configure the Vaanyze plugin for CloudNativePG (CNPG).

## Overview

The Vaanyze CNPG plugin integrates with the CloudNativePG operator using the **CNPG-I gRPC plugin interface**.

Current MVP scope: the plugin safely injects **PostgreSQL configuration tuning** into a CNPG `Cluster` by returning a **JSONPatch** from the CNPG-I `MutateCluster` RPC (opt-in, non-overwriting).

### Architecture

1. **CNPG Operator**: Manages PostgreSQL clusters.
2. **Vaanyze Plugin**: Runs as a separate deployment (gRPC server on port 8080, health HTTP on 8081).
3. **Communication**: The Operator calls the Plugin via CNPG-I gRPC (`GetCapabilities`, `MutateCluster`, `SetStatusInCluster`).
4. **Optimization**: The Plugin reads the CNPG `Cluster` definition and returns a JSONPatch that adds tuned keys under `.spec.postgresql.parameters`.

## Prerequisites

- Kubernetes cluster (v1.25+)
- CloudNativePG Operator installed (v1.22+)
- `kubectl` configured

## Installation

### 1. Build the Docker Image

```bash
docker build -f Dockerfile.cnpg -t vaanyze/cnpg-plugin:latest .
# Push to your registry
# docker push vaanyze/cnpg-plugin:latest
```

### 2. Deploy the Plugin

Apply the Kubernetes manifests to deploy the plugin and configure RBAC permissions:

```bash
kubectl apply -f k8s/cnpg-plugin.yaml
```

This creates:
- `Deployment`: The plugin application
- `Service`: Exposes the gRPC interface
- `ServiceAccount` & `RBAC`: Permissions to read CNPG Cluster resources (and optionally Secrets for future extensions)

Note: the Service exposes **gRPC on port 80** (targeting container port 8080). Health endpoints are served on container port **8081** and are used by Kubernetes probes.

### 3. Configure CloudNativePG to Use the Plugin

Edit your CloudNativePG Operator configuration (usually a ConfigMap or Operator arguments) to register the plugin.

*Note: As of CNPG v1.22, plugins are configured in the Operator's deployment arguments or via a specific ConfigMap. Consult the official CNPG documentation for the exact mechanism for your version.*

Typically, you define the plugin endpoint:

```yaml
# Example Operator Configuration
plugins:
  - name: vaanyze
    endpoint: vaanyze-cnpg-plugin.cnpg-system.svc:80
```

### 4. Enable Plugin on a Cluster

Enable the plugin on a specific CNPG `Cluster` via annotations.

Minimum:
- `cnpg.io/plugins: "vaanyze"` (tells CNPG to call the plugin)
- `vaanyze.io/enable-tuning: "true"` (opt-in gate; plugin does nothing without it)

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: my-cluster
  annotations:
    cnpg.io/plugins: "vaanyze"
    vaanyze.io/enable-tuning: "true"
spec:
  # ... rest of spec

For a complete example, see [examples/cnpg/cluster-vaanyze-tuning.yaml](../examples/cnpg/cluster-vaanyze-tuning.yaml).
```

## How It Works

1. **Capabilities**: CNPG calls `GetCapabilities` to determine supported operations.
2. **Mutation**: On reconcile, CNPG calls `MutateCluster` with the Cluster JSON definition.
3. **Opt-in gate**: If `vaanyze.io/enable-tuning` is not `"true"`, the plugin returns no patch.
4. **Patch generation**: If opted in, the plugin generates a JSONPatch that:
  - Adds `.spec.postgresql.parameters` if missing
  - Adds tuned parameters **only when the key is absent** (never overwrites user-provided values)
5. **Status**: CNPG may call `SetStatusInCluster`, and the plugin returns a status blob (e.g., version + whether tuning was applied).

Important: In the current MVP, the plugin does **not** require DB credentials and does **not** connect to PostgreSQL during `MutateCluster`. It relies on the Cluster definition and conservative defaults.

## Troubleshooting

**Check Plugin Logs:**
```bash
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin
```

**Check Health Endpoints (port-forward):**
```bash
kubectl port-forward -n cnpg-system deployment/vaanyze-cnpg-plugin 8081:8081
curl -sSf http://localhost:8081/health
curl -sSf http://localhost:8081/ready
```

**Common Issues:**
- **Plugin not invoked**: Ensure the CNPG operator is configured with the plugin endpoint, and the Cluster has `cnpg.io/plugins: "vaanyze"`.
- **No changes applied**: Ensure `vaanyze.io/enable-tuning: "true"` is set on the Cluster.
- **gRPC errors**: Ensure the operator can reach `vaanyze-cnpg-plugin.cnpg-system.svc:80`.
