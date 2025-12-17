// src/azure/mod.rs - Azure integration for db-optimizer

pub mod advisor;
pub mod metadata;
pub mod pricing;

pub use advisor::{CostJustification, CostOptimization, SkuRecommendation};
pub use metadata::{AzureMetadata, AzureMetadataCollector};
pub use pricing::{AzurePricing, AzureSku, PricingCalculator};
