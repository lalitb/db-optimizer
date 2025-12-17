# Azure Integration Guide

## Overview

db-optimizer provides deep Azure integration for PostgreSQL cost optimization, combining eBPF-powered performance insights with Azure-specific features.

## Features

### 1. Azure Instance Metadata Service (IMDS) Integration

Automatically detects Azure environment and collects metadata:

```rust
use pg_query_advisor::azure::AzureMetadataCollector;

let collector = AzureMetadataCollector::new();

if collector.is_azure().await {
    let metadata = collector.collect().await?;
    println!("Region: {}", metadata.region);
    println!("VM Size: {}", metadata.vm_size);
    println!("Resource Group: {}", metadata.resource_group);
}
```

**Collected Information**:
- Subscription ID
- Resource Group
- VM ID and Size
- Azure Region
- PostgreSQL Flexible Server details (if applicable)

### 2. Azure SKU Pricing Integration

Real-time pricing for cost optimization:

```rust
use pg_query_advisor::azure::pricing::{AzurePricing, AzureSku, PricingCalculator};

let pricing = AzurePricing::new_with_defaults();
let calculator = PricingCalculator::new(pricing);

let current_sku = AzureSku::from_name("GP_Gen5_8").unwrap();
let recommended_sku = AzureSku::new("GeneralPurpose", "Gen5", 4);

let savings = calculator.calculate_savings(&current_sku, &recommended_sku)?;
println!("Monthly Savings: ${:.2}", savings);  // $277.40
```

**Supported SKUs**:
- General Purpose (GP): 2, 4, 8, 16, 32, 64 vCores
- Memory Optimized (MO): 2, 4, 8, 16, 32 vCores
- Burstable (B): B1ms (1 vCore), B2s (2 vCores)

### 3. eBPF-Powered Cost Justification

Recommendations backed by kernel-level metrics:

```rust
use pg_query_advisor::azure::advisor::{AzureAdvisor, CostJustification};

// eBPF metrics collected over time
let justification = CostJustification {
    cpu_utilization: 23.0,           // From eBPF CPU tracking
    memory_utilization: 41.0,        // From page fault analysis
    io_utilization: 12.0,            // From disk I/O tracking
    
    peak_cpu_during_business_hours: 45.0,
    queries_per_second_p95: 150.0,
    avg_query_latency_ms: 8.5,
    
    // eBPF-specific insights
    io_wait_percentage: 10.0,        // From I/O latency attribution
    disk_seeks_per_query: 5.0,       // From block layer tracing
    page_cache_hit_rate: 0.95,       // From page fault tracking
};

let advisor = AzureAdvisor::new(calculator);
let optimization = advisor.generate_recommendations(&current_sku, justification);

if let Some(rec) = optimization.sku_recommendation {
    println!("Current: {}", rec.current_sku.to_name());
    println!("Recommended: {}", rec.recommended_sku.to_name());
    println!("Monthly Savings: ${:.2}", rec.monthly_savings);
    println!("Confidence: {:.0}%", rec.confidence * 100.0);
    println!("Impact: {}", rec.performance_impact);
}
```

## Cost Optimization Scenarios

### Scenario 1: Over-Provisioned Database

**Current State**:
- SKU: GP_Gen5_8 ($554.80/month)
- CPU: 23% average, 45% peak
- Memory: 41%
- I/O: 12%

**eBPF Analysis**:
```
Page cache hit rate: 95%  ← Most data served from memory
Disk seeks per query: 5    ← Efficient I/O patterns
No lock contention detected
```

**Recommendation**:
```
Downgrade to: GP_Gen5_4
Monthly Savings: $277.40
Performance Impact: None (eBPF-verified low utilization)
Confidence: 92%
```

### Scenario 2: Memory-Constrained Workload

**Current State**:
- SKU: GP_Gen5_8
- CPU: 65%
- Memory: 88%
- I/O: 45%

**eBPF Analysis**:
```
Page cache hit rate: 68%    ← Low (should be >85%)
Page faults: 2,847/sec      ← Memory pressure
I/O wait percentage: 42%    ← High due to memory pressure
```

**Recommendation**:
```
Change to: MO_Gen5_8 (Memory Optimized)
Monthly Cost: +$146/month
Performance Impact: 30-50% latency improvement expected
Confidence: 88%

Justification:
- Low page cache hit rate indicates insufficient memory
- High I/O wait correlates with memory pressure
- Memory Optimized tier provides 2x memory per vCore
```

### Scenario 3: CPU-Bound Workload

**Current State**:
- SKU: MO_Gen5_8 (Memory Optimized)
- CPU: 85%
- Memory: 52%
- I/O: 15%

**eBPF Analysis**:
```
CPU time: 78% of query duration  ← CPU-bound
I/O wait: 12%                    ← Not I/O bound
Page cache hit rate: 92%         ← Memory is adequate
```

**Recommendation**:
```
Change to: GP_Gen5_8 (General Purpose)
Monthly Savings: $146/month
Performance Impact: None (sufficient memory, need CPU not RAM)
Confidence: 90%

Additional Recommendations:
- Review query optimization opportunities (CPU-bound queries)
- Consider parallel query settings
```

## Integration with Azure Portal

### Future: Azure Portal Widget

Planned embeddable React component for Azure Portal:

```typescript
<DbOptimizerWidget
  subscriptionId={subscriptionId}
  resourceGroup={resourceGroup}
  serverName={serverName}
/>
```

**Features**:
- Top 5 slow queries with eBPF breakdowns
- Recommended indexes with one-click apply
- SKU optimization with cost savings
- Before/after comparison

### Azure Monitor Integration

Export metrics to Azure Monitor:

```yaml
# config.yml
azure:
  export_to_monitor: true
  workspace_id: "your-workspace-id"
  metrics:
    - query_latency_p95
    - io_wait_percentage
    - lock_contention_events
    - page_fault_rate
```

## Deployment on Azure

### Azure VM

```bash
# Install on Azure Ubuntu VM
curl -sSL https://github.com/yourusername/db-optimizer/releases/download/v0.1.0/install.sh | bash

# Automatically detects Azure environment
db-optimizer agent --config config.yml
```

### Azure Container Instances

```yaml
apiVersion: 2019-12-01
location: eastus
name: db-optimizer
properties:
  containers:
  - name: db-optimizer
    properties:
      image: your-registry/db-optimizer:latest
      resources:
        requests:
          cpu: 1.0
          memoryInGB: 1.0
      environmentVariables:
      - name: POSTGRES_HOST
        value: your-postgres-server.postgres.database.azure.com
      - name: POSTGRES_USER
        secureValue: your-admin-user
      - name: POSTGRES_PASSWORD
        secureValue: your-password
```

### Azure Kubernetes Service (AKS)

See [k8s/deployment.yaml](../k8s/deployment.yaml) for Kubernetes deployment.

## Azure Postgres Flexible Server

### Supported Configurations

- **Compute**: All General Purpose, Memory Optimized, and Burstable SKUs
- **Storage**: Premium SSD, Standard SSD
- **High Availability**: Zone-redundant, Same-zone
- **Versions**: PostgreSQL 11, 12, 13, 14, 15

### Connection String

```bash
# Using Azure AD authentication (recommended)
db-optimizer agent \
  --host your-server.postgres.database.azure.com \
  --user your-azure-ad-user@yourdomain.com \
  --sslmode require

# Using password authentication
db-optimizer agent \
  --host your-server.postgres.database.azure.com \
  --user admin_user \
  --password your-password \
  --sslmode require
```

## Cost Optimization Best Practices

### 1. Collect Sufficient Data

Recommended monitoring period: **7 days minimum**, **30 days ideal**

```bash
# Run for at least 7 days to capture weekly patterns
db-optimizer agent --duration 7d
```

### 2. Consider Business Cycles

eBPF tracks peak vs average utilization:
- Peak during business hours
- Weekend patterns
- Month-end spikes

### 3. Test Before Applying

```bash
# Generate recommendations only (dry-run)
db-optimizer analyze --azure-optimization

# Review confidence scores
# Apply only recommendations with >85% confidence
```

### 4. Monitor After Changes

eBPF continues monitoring post-change:
```
Before SKU change:
- P95 latency: 45ms
- CPU: 85%

After SKU change (GP_Gen5_8 → MO_Gen5_8):
- P95 latency: 32ms (-29%)
- CPU: 62%
- Page faults: -78%
✓ Change validated by eBPF
```

## Pricing Accuracy

Pricing data is approximate and should be verified:

1. **Azure Pricing Calculator**: https://azure.microsoft.com/pricing/calculator/
2. **Azure Cost Management**: Real-time costs in Azure Portal
3. **Update Frequency**: Refresh pricing monthly via `AzurePricing::update_from_api()`

## Security Considerations

### IMDS Access

Azure IMDS is accessible from within Azure VMs without authentication:
```rust
// Safe - only accessible from Azure VM
let metadata = collector.collect().await?;
```

### Managed Identity

Recommended for production:

```yaml
azure:
  use_managed_identity: true
  # No credentials needed - uses Azure AD
```

### Least Privilege

Required permissions:
- `Microsoft.DBforPostgreSQL/flexibleServers/read`
- `Microsoft.DBforPostgreSQL/flexibleServers/configurations/read`
- (Optional) `Microsoft.DBforPostgreSQL/flexibleServers/configurations/write` for auto-apply

## Troubleshooting

### Not Detecting Azure Environment

```bash
# Verify IMDS access
curl -H "Metadata: true" "http://169.254.169.254/metadata/instance?api-version=2021-02-01"

# Should return JSON with compute info
```

### Pricing Data Outdated

```bash
# Manual pricing update
db-optimizer update-pricing --source azure-api
```

### SKU Recommendation Unexpected

Review eBPF confidence metrics:
```bash
db-optimizer analyze --explain-recommendation --query-id 12345
```

## References

- [Azure PostgreSQL Flexible Server Pricing](https://azure.microsoft.com/pricing/details/postgresql/flexible-server/)
- [Azure IMDS Documentation](https://docs.microsoft.com/azure/virtual-machines/instance-metadata-service)
- [Azure Monitor Integration](https://docs.microsoft.com/azure/azure-monitor/)
