use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Agent state tracking what has been applied
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// IDs of recommendations that have been successfully applied
    pub applied_recommendations: HashSet<String>,

    /// Last time the agent ran successfully
    pub last_run: DateTime<Utc>,

    /// Total number of optimizations applied
    pub total_optimizations: usize,

    /// Agent start time
    pub started_at: DateTime<Utc>,
}

impl AgentState {
    /// Create new empty state
    pub fn new() -> Self {
        Self {
            applied_recommendations: HashSet::new(),
            last_run: Utc::now(),
            total_optimizations: 0,
            started_at: Utc::now(),
        }
    }

    /// Load state from file, or create new if doesn't exist
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let state: Self = serde_json::from_str(&content)?;
            tracing::info!(
                "Loaded agent state: {} optimizations applied",
                state.total_optimizations
            );
            Ok(state)
        } else {
            tracing::info!("No existing state found, creating new state");
            Ok(Self::new())
        }
    }

    /// Save state to file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;

        tracing::debug!("Saved agent state to {}", path.display());
        Ok(())
    }

    /// Check if a recommendation has already been applied
    pub fn is_applied(&self, id: &str) -> bool {
        self.applied_recommendations.contains(id)
    }

    /// Mark a recommendation as applied
    pub fn mark_applied(&mut self, id: String) {
        if self.applied_recommendations.insert(id) {
            self.total_optimizations += 1;
        }
        self.last_run = Utc::now();
    }

    /// Update last run time
    pub fn update_last_run(&mut self) {
        self.last_run = Utc::now();
    }

    /// Get runtime duration
    pub fn uptime_hours(&self) -> f64 {
        let duration = Utc::now() - self.started_at;
        duration.num_seconds() as f64 / 3600.0
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

/// Results from a single analysis cycle
#[derive(Debug, Default)]
pub struct AnalysisResults {
    pub new_recommendations: usize,
    pub applied_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub total_savings_usd: f64,
    pub health_score: Option<u8>,
}

impl AnalysisResults {
    pub fn is_empty(&self) -> bool {
        self.new_recommendations == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_new_state() {
        let state = AgentState::new();
        assert_eq!(state.total_optimizations, 0);
        assert!(state.applied_recommendations.is_empty());
    }

    #[test]
    fn test_mark_applied() {
        let mut state = AgentState::new();
        state.mark_applied("rec1".to_string());

        assert_eq!(state.total_optimizations, 1);
        assert!(state.is_applied("rec1"));
        assert!(!state.is_applied("rec2"));

        // Marking same rec again doesn't increment count
        state.mark_applied("rec1".to_string());
        assert_eq!(state.total_optimizations, 1);
    }

    #[test]
    fn test_save_load() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path();

        let mut state = AgentState::new();
        state.mark_applied("rec1".to_string());
        state.mark_applied("rec2".to_string());

        state.save(path)?;

        let loaded = AgentState::load(path)?;
        assert_eq!(loaded.total_optimizations, 2);
        assert!(loaded.is_applied("rec1"));
        assert!(loaded.is_applied("rec2"));

        Ok(())
    }

    #[test]
    fn test_load_nonexistent() -> Result<()> {
        let state = AgentState::load("/tmp/nonexistent-file-12345.json")?;
        assert_eq!(state.total_optimizations, 0);
        Ok(())
    }
}
