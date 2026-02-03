//! Validation module for EXPLAIN-based proof of index recommendations.
//!
//! This module provides:
//! - EXPLAIN plan execution and parsing
//! - Index validation proof generation
//! - Before/after plan comparison
//! - Deterministic diff summaries

pub mod explain;
pub mod models;
pub mod proof;

pub use explain::{ExplainExecutor, PlanParser};
pub use models::{
    ExplainPlan, PlanDiff, PlanNode, ProofMode, ProofStrength, ScanType, ValidationReasonCode,
    ValidationResult, ValidationStatus,
};
pub use proof::{DestructiveCheckResult, IndexProof, ProofGenerator};
