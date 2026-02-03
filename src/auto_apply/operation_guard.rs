//! Centralized operation mode enforcement.
//!
//! This module provides a single source of truth for determining whether
//! destructive operations (CREATE INDEX, DROP INDEX) are allowed.
//!
//! All destructive code paths MUST use this module to check permissions.

use crate::analyzer::environment::{detect_environment, DatabaseEnvironment, EnvironmentInfo};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Reasons why an operation might be blocked
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    /// Kill switch is active - emergency stop
    KillSwitchActive,
    /// System is in explicit read-only mode
    ReadOnlyMode,
    /// Managed database environment detected (RDS, Cloud SQL, etc.)
    ManagedEnvironment,
    /// Missing CREATE INDEX permission
    MissingPermission,
    /// Environment not yet detected
    EnvironmentUnknown,
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KillSwitchActive => write!(f, "Kill switch is active"),
            Self::ReadOnlyMode => write!(f, "System is in read-only mode"),
            Self::ManagedEnvironment => write!(f, "Managed database environment detected"),
            Self::MissingPermission => write!(f, "Missing CREATE/DROP INDEX permission"),
            Self::EnvironmentUnknown => write!(f, "Environment not yet detected"),
        }
    }
}

/// Result of an operation permission check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationCheck {
    /// Whether the operation is allowed
    pub allowed: bool,
    /// If not allowed, why
    pub block_reason: Option<BlockReason>,
    /// Current effective mode
    pub effective_mode: EffectiveMode,
    /// Whether the mode is forced (cannot be overridden)
    pub is_forced: bool,
}

impl OperationCheck {
    pub fn allowed(mode: EffectiveMode) -> Self {
        Self {
            allowed: true,
            block_reason: None,
            effective_mode: mode,
            is_forced: false,
        }
    }

    pub fn blocked(reason: BlockReason, mode: EffectiveMode, forced: bool) -> Self {
        Self {
            allowed: false,
            block_reason: Some(reason),
            effective_mode: mode,
            is_forced: forced,
        }
    }
}

/// Effective operation mode after all overrides are applied
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveMode {
    /// No DDL operations allowed
    ReadOnly,
    /// Safe operations allowed (CREATE INDEX CONCURRENTLY)
    SafeApply,
    /// More aggressive operations allowed
    Aggressive,
}

impl std::fmt::Display for EffectiveMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "read_only"),
            Self::SafeApply => write!(f, "safe_apply"),
            Self::Aggressive => write!(f, "aggressive"),
        }
    }
}

/// Centralized operation guard that enforces mode restrictions.
///
/// This is the ONLY place that should determine if destructive operations are allowed.
/// All HTTP handlers and internal code paths must use this.
#[derive(Debug)]
pub struct OperationGuard {
    /// Global kill switch - when true, ALL destructive operations blocked
    kill_switch: Arc<AtomicBool>,

    /// User-configured operation mode
    configured_mode: Arc<RwLock<String>>,

    /// Cached environment info (set after detection)
    environment: Arc<RwLock<Option<EnvironmentInfo>>>,

    /// Whether environment forces read-only (cached)
    environment_forces_read_only: Arc<AtomicBool>,
}

impl OperationGuard {
    /// Create a new operation guard with default (safe) settings
    pub fn new() -> Self {
        Self {
            kill_switch: Arc::new(AtomicBool::new(false)),
            configured_mode: Arc::new(RwLock::new("observe_only".to_string())),
            environment: Arc::new(RwLock::new(None)),
            environment_forces_read_only: Arc::new(AtomicBool::new(true)), // Safe default
        }
    }

    /// Create with explicit initial settings
    pub fn with_settings(mode: &str, kill_switch: bool) -> Self {
        Self {
            kill_switch: Arc::new(AtomicBool::new(kill_switch)),
            configured_mode: Arc::new(RwLock::new(mode.to_string())),
            environment: Arc::new(RwLock::new(None)),
            environment_forces_read_only: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Update environment info after detection
    pub async fn set_environment(&self, env: EnvironmentInfo) {
        let forced = should_force_read_only(&env);
        self.environment_forces_read_only.store(forced, Ordering::SeqCst);
        *self.environment.write().await = Some(env);
    }

    /// Get the cached environment info
    pub async fn get_environment(&self) -> Option<EnvironmentInfo> {
        self.environment.read().await.clone()
    }

    /// Set the kill switch state
    pub fn set_kill_switch(&self, enabled: bool) {
        self.kill_switch.store(enabled, Ordering::SeqCst);
    }

    /// Get current kill switch state
    pub fn is_kill_switch_active(&self) -> bool {
        self.kill_switch.load(Ordering::SeqCst)
    }

    /// Set the configured operation mode
    pub async fn set_mode(&self, mode: &str) {
        *self.configured_mode.write().await = mode.to_string();
    }

    /// Get the configured mode (not effective mode)
    pub async fn get_configured_mode(&self) -> String {
        self.configured_mode.read().await.clone()
    }

    /// Get the EFFECTIVE operation mode after all overrides.
    ///
    /// Priority (highest to lowest):
    /// 1. Kill switch active -> ReadOnly
    /// 2. Environment forces read-only -> ReadOnly
    /// 3. User configured mode
    pub async fn effective_mode(&self) -> EffectiveMode {
        // Kill switch overrides everything
        if self.kill_switch.load(Ordering::SeqCst) {
            return EffectiveMode::ReadOnly;
        }

        // Environment forces read-only
        if self.environment_forces_read_only.load(Ordering::SeqCst) {
            return EffectiveMode::ReadOnly;
        }

        // Check configured mode
        let mode = self.configured_mode.read().await;
        match mode.as_str() {
            "safe_auto" | "safe" => EffectiveMode::SafeApply,
            "aggressive" | "aggro" => EffectiveMode::Aggressive,
            _ => EffectiveMode::ReadOnly,
        }
    }

    /// Check if a destructive operation (CREATE/DROP INDEX) is allowed.
    ///
    /// This is the main enforcement point. Call this before ANY destructive operation.
    pub async fn check_destructive_operation(&self) -> OperationCheck {
        // Check kill switch first (highest priority)
        if self.kill_switch.load(Ordering::SeqCst) {
            return OperationCheck::blocked(
                BlockReason::KillSwitchActive,
                EffectiveMode::ReadOnly,
                true,
            );
        }

        // Check environment
        let env_forces = self.environment_forces_read_only.load(Ordering::SeqCst);
        let env = self.environment.read().await;

        if env.is_none() {
            // Environment not detected yet - be conservative
            return OperationCheck::blocked(
                BlockReason::EnvironmentUnknown,
                EffectiveMode::ReadOnly,
                true,
            );
        }

        if env_forces {
            let env_ref = env.as_ref().unwrap();
            let reason = if !env_ref.can_create_index {
                BlockReason::MissingPermission
            } else {
                BlockReason::ManagedEnvironment
            };
            return OperationCheck::blocked(reason, EffectiveMode::ReadOnly, true);
        }

        // Check configured mode
        let mode = self.configured_mode.read().await;
        let effective = match mode.as_str() {
            "safe_auto" | "safe" => EffectiveMode::SafeApply,
            "aggressive" | "aggro" => EffectiveMode::Aggressive,
            _ => {
                return OperationCheck::blocked(
                    BlockReason::ReadOnlyMode,
                    EffectiveMode::ReadOnly,
                    false,
                );
            }
        };

        OperationCheck::allowed(effective)
    }

    /// Check if operation is allowed - convenience method that returns bool
    pub async fn is_destructive_allowed(&self) -> bool {
        self.check_destructive_operation().await.allowed
    }

    /// Get full status for health/status endpoints
    pub async fn get_status(&self) -> OperationGuardStatus {
        let check = self.check_destructive_operation().await;
        let env = self.environment.read().await.clone();
        let configured = self.configured_mode.read().await.clone();
        
        // Determine force reason
        let (is_forced, force_reason) = if self.kill_switch.load(Ordering::SeqCst) {
            (true, Some("Kill switch active".to_string()))
        } else if self.environment_forces_read_only.load(Ordering::SeqCst) {
            let reason = env.as_ref().map(|e| {
                if !e.can_create_index {
                    "Missing CREATE INDEX permission".to_string()
                } else {
                    format!("Managed environment: {:?}", e.environment)
                }
            }).unwrap_or_else(|| "Environment requires read-only".to_string());
            (true, Some(reason))
        } else {
            (false, None)
        };

        OperationGuardStatus {
            effective_mode: check.effective_mode,
            configured_mode: configured,
            kill_switch_active: self.kill_switch.load(Ordering::SeqCst),
            environment_forces_read_only: self.environment_forces_read_only.load(Ordering::SeqCst),
            destructive_operations_allowed: check.allowed,
            block_reason: check.block_reason,
            environment: env.map(|e| e.environment),
            is_forced,
            force_reason,
        }
    }

    /// Detect environment and update guard state
    pub async fn detect_and_update_environment(&self, pool: &PgPool) -> anyhow::Result<EnvironmentInfo> {
        let env = detect_environment(pool).await?;
        self.set_environment(env.clone()).await;
        Ok(env)
    }

    /// Force read-only mode with a reason (for error conditions)
    pub fn force_read_only(&self, _reason: String) {
        self.environment_forces_read_only.store(true, Ordering::SeqCst);
    }

    // Shared state accessors for integrating with existing code
    pub fn kill_switch_handle(&self) -> Arc<AtomicBool> {
        self.kill_switch.clone()
    }

    pub fn configured_mode_handle(&self) -> Arc<RwLock<String>> {
        self.configured_mode.clone()
    }
}

impl Default for OperationGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for OperationGuard {
    fn clone(&self) -> Self {
        Self {
            kill_switch: self.kill_switch.clone(),
            configured_mode: self.configured_mode.clone(),
            environment: self.environment.clone(),
            environment_forces_read_only: self.environment_forces_read_only.clone(),
        }
    }
}

/// Status information for health endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationGuardStatus {
    pub effective_mode: EffectiveMode,
    pub configured_mode: String,
    pub kill_switch_active: bool,
    pub environment_forces_read_only: bool,
    pub destructive_operations_allowed: bool,
    pub block_reason: Option<BlockReason>,
    pub environment: Option<DatabaseEnvironment>,
    /// Whether the mode was forced (not user configured)
    pub is_forced: bool,
    /// Reason why mode was forced (if applicable)
    pub force_reason: Option<String>,
}

/// Determine if an environment should force read-only mode.
///
/// This is called when environment is detected to cache the result.
fn should_force_read_only(env: &EnvironmentInfo) -> bool {
    !env.can_create_index
        || matches!(
            env.environment,
            DatabaseEnvironment::AwsRds
                | DatabaseEnvironment::AwsAurora
                | DatabaseEnvironment::GoogleCloudSql
                | DatabaseEnvironment::AzurePostgres
                | DatabaseEnvironment::Heroku
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kill_switch_blocks_everything() {
        let guard = OperationGuard::new();

        // Set up a permissive environment
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("safe_auto").await;

        // Should be allowed
        assert!(guard.is_destructive_allowed().await);

        // Enable kill switch
        guard.set_kill_switch(true);

        // Should be blocked
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::KillSwitchActive));
        assert!(check.is_forced);
    }

    #[tokio::test]
    async fn test_managed_environment_forces_read_only() {
        let guard = OperationGuard::new();

        // Set up RDS environment
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::AwsRds,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true, // Even with permissions!
            can_drop_index: true,
            is_superuser: false,
            is_db_owner: false,
            warnings: vec![],
            recommended_mode: "observe_only".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("safe_auto").await; // Try to enable

        // Should still be blocked
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::ManagedEnvironment));
        assert!(check.is_forced);
    }

    #[tokio::test]
    async fn test_missing_permission_blocks() {
        let guard = OperationGuard::new();

        // Self-managed but no permissions
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: false, // No permission
            can_drop_index: false,
            is_superuser: false,
            is_db_owner: false,
            warnings: vec![],
            recommended_mode: "observe_only".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("safe_auto").await;

        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::MissingPermission));
    }

    #[tokio::test]
    async fn test_read_only_mode_blocks() {
        let guard = OperationGuard::new();

        // Permissive environment
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("observe_only").await; // Explicit read-only

        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::ReadOnlyMode));
        assert!(!check.is_forced); // User chose this, not forced
    }

    #[tokio::test]
    async fn test_safe_auto_allows_operations() {
        let guard = OperationGuard::new();

        // Permissive environment + safe_auto mode
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("safe_auto").await;

        let check = guard.check_destructive_operation().await;
        assert!(check.allowed);
        assert_eq!(check.effective_mode, EffectiveMode::SafeApply);
    }

    #[tokio::test]
    async fn test_environment_unknown_blocks() {
        let guard = OperationGuard::new();
        guard.set_mode("safe_auto").await;

        // Don't set environment - should be blocked
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::EnvironmentUnknown));
    }

    #[tokio::test]
    async fn test_effective_mode_priority() {
        let guard = OperationGuard::new();

        // Set permissive environment
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };
        guard.set_environment(env).await;
        guard.set_mode("aggressive").await;

        // Aggressive mode should work
        assert_eq!(guard.effective_mode().await, EffectiveMode::Aggressive);

        // Kill switch should override
        guard.set_kill_switch(true);
        assert_eq!(guard.effective_mode().await, EffectiveMode::ReadOnly);
    }

    #[test]
    fn test_block_reason_display() {
        assert_eq!(
            BlockReason::KillSwitchActive.to_string(),
            "Kill switch is active"
        );
        assert_eq!(
            BlockReason::ManagedEnvironment.to_string(),
            "Managed database environment detected"
        );
    }
}
