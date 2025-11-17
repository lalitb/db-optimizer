// src/azure/mod.rs - Azure integration for db-optimizer

pub mod metadata;
pub mod pricing;
pub mod advisor;

pub use metadata::{AzureMetadata, AzureMetadataCollector};
pub use pricing::{AzurePricing, AzureSku, PricingCalculator};
pub use advisor::{CostOptimization, CostJustification, SkuRecommendation};
