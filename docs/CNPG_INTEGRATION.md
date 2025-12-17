# Vaanyze CloudNativePG Integration Guide

This guide details how to deploy and configure the Vaanyze plugin for CloudNativePG (CNPG).

## Overview

The Vaanyze CNPG plugin integrates directly with the CloudNativePG operator to provide autonomous database optimization for PostgreSQL clusters running in Kubernetes.

### Architecture

1. **CNPG Operator**: Manages PostgreSQL clusters.
2. **Vaanyze Plugin**: Runs as a separate deployment (gRPC server).
3. **Communication**: The Operator calls the Plugin via gRPC for lifecycle events (e.g., `Evaluate`).
4. **Optimization**: The Plugin connects to the database, analyzes performance, and reports health scores.

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
- `ServiceAccount` & `RBAC`: Permissions to read Secrets (for DB access)

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

Add the plugin annotation to your PostgreSQL Cluster manifest:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: my-cluster
  annotations:
    cnpg.io/plugins: vaanyze
spec:
  # ... rest of spec
```

## How It Works

1. **Lifecycle Hook**: When the Operator reconciles the Cluster, it calls the `LifecycleHook` method of the Vaanyze plugin.
2. **Connection**: The plugin reads the `Cluster` definition, extracts the name and namespace.
3. **Authentication**: It fetches the `<cluster-name>-superuser` Secret to get the `postgres` password.
4. **Analysis**: It connects to the primary instance (`<cluster-name>-rw`) and runs the Vaanyze analysis engine.
5. **Reporting**: It logs the health score and potential optimizations.

## Troubleshooting

**Check Plugin Logs:**
```bash
kubectl logs -n cnpg-system -l app=vaanyze-cnpg-plugin
```

**Common Issues:**
- **Permission Denied**: Ensure the ServiceAccount has `get` access to `secrets` in the target namespace.
- **Connection Failed**: Verify the plugin can resolve the `<cluster-name>-rw` service DNS.
- **gRPC Errors**: Ensure the Operator can reach the plugin Service on port 80.
