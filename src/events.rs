// src/events.rs - Database event tracking for deployment correlation

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Types of database events we track
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventType {
    /// Schema change (DDL)
    SchemaChange,
    /// Configuration change
    ConfigChange,
    /// Index created/dropped
    IndexChange,
    /// Manual marker (user-defined deployment)
    Deployment,
    /// Performance anomaly detected
    PerformanceAnomaly,
}

/// A database event that can be correlated with performance changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseEvent {
    pub id: String,
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub details: EventDetails,
    pub source: EventSource,
}

/// Detailed information about the event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventDetails {
    SchemaChange {
        table_name: String,
        change_type: String, // ALTER, CREATE, DROP
        sql: String,
    },
    IndexChange {
        index_name: String,
        table_name: String,
        action: String, // CREATE, DROP
        is_concurrent: bool,
    },
    ConfigChange {
        parameter: String,
        old_value: String,
        new_value: String,
    },
    Deployment {
        version: Option<String>,
        notes: String,
    },
    PerformanceAnomaly {
        metric: String,
        baseline: f64,
        current: f64,
        deviation_pct: f64,
    },
}

/// Source of the event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventSource {
    /// Detected automatically by monitoring pg_stat_user_tables
    AutoDetected,
    /// Applied by the auto-apply orchestrator
    AutoApplied,
    /// Manually recorded via CLI
    Manual,
    /// Detected from external monitoring
    External,
}

/// Event store for tracking database events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStore {
    pub events: Vec<DatabaseEvent>,
}

impl EventStore {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Load event store from file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let store: Self = serde_json::from_str(&content)?;
            tracing::info!("Loaded {} events from store", store.events.len());
            Ok(store)
        } else {
            tracing::info!("No existing event store found, creating new");
            Ok(Self::new())
        }
    }

    /// Save event store to file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;

        tracing::debug!("Saved {} events to {}", self.events.len(), path.display());
        Ok(())
    }

    /// Add a new event
    pub fn add_event(&mut self, event: DatabaseEvent) {
        tracing::info!(
            "Recording event: {} - {}",
            event.event_type_str(),
            event.description
        );
        self.events.push(event);
    }

    /// Get events within a time range
    pub fn get_events_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<&DatabaseEvent> {
        self.events
            .iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .collect()
    }

    /// Get recent events (last N)
    pub fn get_recent_events(&self, count: usize) -> Vec<&DatabaseEvent> {
        let mut events: Vec<_> = self.events.iter().collect();
        events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        events.into_iter().take(count).collect()
    }

    /// Get events by type
    pub fn get_events_by_type(&self, event_type: EventType) -> Vec<&DatabaseEvent> {
        self.events
            .iter()
            .filter(|e| e.event_type == event_type)
            .collect()
    }

    /// Prune old events (keep last N days)
    pub fn prune_old_events(&mut self, days: i64) {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let before_count = self.events.len();
        self.events.retain(|e| e.timestamp >= cutoff);
        let pruned = before_count - self.events.len();
        if pruned > 0 {
            tracing::info!("Pruned {} old events (older than {} days)", pruned, days);
        }
    }
}

impl Default for EventStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DatabaseEvent {
    /// Create a new event with auto-generated ID
    pub fn new(
        event_type: EventType,
        description: String,
        details: EventDetails,
        source: EventSource,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            timestamp: Utc::now(),
            description,
            details,
            source,
        }
    }

    /// Get event type as string
    pub fn event_type_str(&self) -> &str {
        match self.event_type {
            EventType::SchemaChange => "Schema Change",
            EventType::ConfigChange => "Config Change",
            EventType::IndexChange => "Index Change",
            EventType::Deployment => "Deployment",
            EventType::PerformanceAnomaly => "Performance Anomaly",
        }
    }

    /// Get a short summary of the event
    pub fn summary(&self) -> String {
        match &self.details {
            EventDetails::SchemaChange {
                table_name,
                change_type,
                ..
            } => {
                format!("{} on {}", change_type, table_name)
            }
            EventDetails::IndexChange {
                index_name, action, ..
            } => {
                format!("{} index {}", action, index_name)
            }
            EventDetails::ConfigChange {
                parameter,
                new_value,
                ..
            } => {
                format!("{} = {}", parameter, new_value)
            }
            EventDetails::Deployment { version, .. } => {
                version.clone().unwrap_or_else(|| "deployment".to_string())
            }
            EventDetails::PerformanceAnomaly {
                metric,
                deviation_pct,
                ..
            } => {
                format!("{} changed by {:.1}%", metric, deviation_pct)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_event_store_new() {
        let store = EventStore::new();
        assert_eq!(store.events.len(), 0);
    }

    #[test]
    fn test_add_event() {
        let mut store = EventStore::new();
        let event = DatabaseEvent::new(
            EventType::IndexChange,
            "Created index on users".to_string(),
            EventDetails::IndexChange {
                index_name: "idx_users_email".to_string(),
                table_name: "users".to_string(),
                action: "CREATE".to_string(),
                is_concurrent: true,
            },
            EventSource::AutoApplied,
        );

        store.add_event(event);
        assert_eq!(store.events.len(), 1);
    }

    #[test]
    fn test_save_load() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let path = temp_file.path();

        let mut store = EventStore::new();
        store.add_event(DatabaseEvent::new(
            EventType::Deployment,
            "v1.2.3 deployed".to_string(),
            EventDetails::Deployment {
                version: Some("v1.2.3".to_string()),
                notes: "Bug fixes".to_string(),
            },
            EventSource::Manual,
        ));

        store.save(path)?;

        let loaded = EventStore::load(path)?;
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].description, "v1.2.3 deployed");

        Ok(())
    }

    #[test]
    fn test_get_recent_events() {
        let mut store = EventStore::new();

        for i in 0..5 {
            store.add_event(DatabaseEvent::new(
                EventType::SchemaChange,
                format!("Change {}", i),
                EventDetails::SchemaChange {
                    table_name: "test".to_string(),
                    change_type: "ALTER".to_string(),
                    sql: "ALTER TABLE test ADD COLUMN x INT".to_string(),
                },
                EventSource::AutoDetected,
            ));
        }

        let recent = store.get_recent_events(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_prune_old_events() {
        let mut store = EventStore::new();

        // Add an old event (manually set timestamp)
        let mut old_event = DatabaseEvent::new(
            EventType::SchemaChange,
            "Old change".to_string(),
            EventDetails::SchemaChange {
                table_name: "test".to_string(),
                change_type: "ALTER".to_string(),
                sql: "ALTER TABLE test ADD COLUMN x INT".to_string(),
            },
            EventSource::AutoDetected,
        );
        old_event.timestamp = Utc::now() - chrono::Duration::days(100);
        store.add_event(old_event);

        // Add a recent event
        store.add_event(DatabaseEvent::new(
            EventType::SchemaChange,
            "Recent change".to_string(),
            EventDetails::SchemaChange {
                table_name: "test".to_string(),
                change_type: "ALTER".to_string(),
                sql: "ALTER TABLE test ADD COLUMN y INT".to_string(),
            },
            EventSource::AutoDetected,
        ));

        assert_eq!(store.events.len(), 2);

        store.prune_old_events(30);

        assert_eq!(store.events.len(), 1);
        assert_eq!(store.events[0].description, "Recent change");
    }
}
