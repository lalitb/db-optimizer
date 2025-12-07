// src/ebpf/mock.rs
//
// Mock providers for rootless CI testing
// Reference: Enhanced eBPF Implementation Plan v2 - "CI-Friendly Testing" section
//
// Provides:
// - MockEventGenerator: Generates synthetic eBPF events
// - MockDaemonServer: Simulates the privileged daemon for client testing
// - ReplayProvider: Replays captured production events
//
// Usage in tests:
//   #[cfg(test)]
//   let collector = MockEventGenerator::new(MockProfile::HighLoad);
//   let events = collector.generate_batch(100);

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::events::{EventType, RawEvent};
use super::protocol::{Message, MessagePayload};

/// Workload profiles for generating realistic mock data
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MockProfile {
    /// Idle system with occasional queries
    Idle,
    /// Normal OLTP workload
    NormalOltp,
    /// Read-heavy analytical workload
    AnalyticalRead,
    /// Write-heavy batch workload
    BatchWrite,
    /// High concurrency with lock contention
    HighContention,
    /// Overloaded system with high latency
    HighLoad,
    /// Mixed workload
    Mixed,
}

impl MockProfile {
    /// Get typical events per second for this profile
    pub fn events_per_second(&self) -> u64 {
        match self {
            Self::Idle => 10,
            Self::NormalOltp => 500,
            Self::AnalyticalRead => 200,
            Self::BatchWrite => 300,
            Self::HighContention => 800,
            Self::HighLoad => 5000,
            Self::Mixed => 1000,
        }
    }

    /// Get typical query patterns for this profile
    #[allow(dead_code)] // Reserved for mock query generation
    fn query_patterns(&self) -> Vec<(&'static str, f64)> {
        match self {
            Self::Idle => vec![
                ("SELECT 1", 0.9),
                ("SELECT * FROM users WHERE id = $1", 0.1),
            ],
            Self::NormalOltp => vec![
                ("SELECT * FROM orders WHERE customer_id = $1", 0.3),
                ("INSERT INTO orders (customer_id, product_id, qty) VALUES ($1, $2, $3)", 0.2),
                ("UPDATE orders SET status = $1 WHERE id = $2", 0.2),
                ("SELECT p.*, c.name FROM products p JOIN categories c ON p.cat_id = c.id WHERE p.id = $1", 0.2),
                ("DELETE FROM cart WHERE session_id = $1 AND created_at < $2", 0.1),
            ],
            Self::AnalyticalRead => vec![
                ("SELECT date_trunc('day', created_at), SUM(amount) FROM orders GROUP BY 1 ORDER BY 1", 0.4),
                ("SELECT customer_id, COUNT(*), SUM(total) FROM orders WHERE created_at > $1 GROUP BY 1", 0.3),
                ("SELECT * FROM products p WHERE EXISTS (SELECT 1 FROM inventory i WHERE i.product_id = p.id AND i.qty < 10)", 0.2),
                ("WITH monthly AS (SELECT ...) SELECT * FROM monthly WHERE ...", 0.1),
            ],
            Self::BatchWrite => vec![
                ("INSERT INTO events (type, data, created_at) VALUES ($1, $2, $3)", 0.5),
                ("COPY events FROM STDIN", 0.2),
                ("UPDATE products SET price = price * $1 WHERE category = $2", 0.2),
                ("DELETE FROM logs WHERE created_at < $1", 0.1),
            ],
            Self::HighContention => vec![
                ("UPDATE accounts SET balance = balance - $1 WHERE id = $2", 0.4),
                ("SELECT * FROM accounts WHERE id = $1 FOR UPDATE", 0.3),
                ("INSERT INTO transactions (from_id, to_id, amount) VALUES ($1, $2, $3)", 0.2),
                ("SELECT * FROM inventory WHERE product_id = $1 FOR UPDATE NOWAIT", 0.1),
            ],
            Self::HighLoad | Self::Mixed => vec![
                ("SELECT * FROM users WHERE id = $1", 0.2),
                ("INSERT INTO logs (msg, level) VALUES ($1, $2)", 0.2),
                ("UPDATE sessions SET last_active = $1 WHERE id = $2", 0.2),
                ("SELECT COUNT(*) FROM orders WHERE status = $1", 0.2),
                ("DELETE FROM temp_data WHERE created_at < $1", 0.2),
            ],
        }
    }

    /// Get latency distribution parameters (mean, stddev) in microseconds
    fn latency_params(&self) -> (u64, u64) {
        match self {
            Self::Idle => (100, 50),                  // 0.1ms avg
            Self::NormalOltp => (1_000, 500),         // 1ms avg
            Self::AnalyticalRead => (50_000, 30_000), // 50ms avg
            Self::BatchWrite => (5_000, 3_000),       // 5ms avg
            Self::HighContention => (10_000, 8_000),  // 10ms avg with high variance
            Self::HighLoad => (100_000, 80_000),      // 100ms avg (overloaded)
            Self::Mixed => (5_000, 10_000),           // 5ms avg with high variance
        }
    }

    /// Get I/O characteristics (read_bytes_mean, write_bytes_mean)
    fn io_params(&self) -> (u64, u64) {
        match self {
            Self::Idle => (1024, 0),
            Self::NormalOltp => (8192, 4096),
            Self::AnalyticalRead => (1_000_000, 1024),
            Self::BatchWrite => (1024, 500_000),
            Self::HighContention => (4096, 4096),
            Self::HighLoad => (100_000, 50_000),
            Self::Mixed => (50_000, 25_000),
        }
    }
}

/// Generator for synthetic eBPF events
pub struct MockEventGenerator {
    profile: MockProfile,
    rng: StdRng,
    #[allow(dead_code)] // Reserved for dynamic PID generation
    next_pid: u32,
    pids: Vec<u32>,
    start_time: Instant,
    event_count: u64,
}

impl MockEventGenerator {
    /// Create a new generator with the given profile
    pub fn new(profile: MockProfile) -> Self {
        Self::with_seed(profile, 42)
    }

    /// Create a generator with a specific seed for reproducibility
    pub fn with_seed(profile: MockProfile, seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);

        // Generate some PIDs to use
        let num_pids = match profile {
            MockProfile::Idle => 2,
            MockProfile::HighContention | MockProfile::HighLoad => 50,
            _ => 10,
        };

        let pids: Vec<u32> = (0..num_pids).map(|_| rng.gen_range(1000..65000)).collect();

        Self {
            profile,
            rng,
            next_pid: 1000,
            pids,
            start_time: Instant::now(),
            event_count: 0,
        }
    }

    /// Generate a single event
    pub fn generate_event(&mut self) -> RawEvent {
        self.event_count += 1;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let pid = *self
            .pids
            .get(self.rng.gen_range(0..self.pids.len()))
            .unwrap_or(&1000);

        // Choose event type based on profile
        let event_type = self.choose_event_type();

        // Generate latency
        let (mean, stddev) = self.profile.latency_params();
        let duration = self.generate_latency(mean, stddev);

        // Generate I/O
        let (read_mean, write_mean) = self.profile.io_params();
        let bytes = if matches!(event_type, EventType::Read) {
            self.generate_size(read_mean)
        } else if matches!(event_type, EventType::Write) {
            self.generate_size(write_mean)
        } else {
            0
        };

        RawEvent {
            timestamp_ns: timestamp,
            pid,
            tid: pid, // Simplified: tid = pid
            event_type_raw: event_type as u8,
            _pad: [0; 3],
            value1: duration, // Store duration in value1
            value2: bytes,    // Store bytes in value2
        }
    }

    /// Generate a batch of events
    pub fn generate_batch(&mut self, count: usize) -> Vec<RawEvent> {
        (0..count).map(|_| self.generate_event()).collect()
    }

    /// Generate events at the profile's natural rate for a duration
    pub fn generate_for_duration(&mut self, duration: Duration) -> Vec<RawEvent> {
        let eps = self.profile.events_per_second();
        let total_events = (duration.as_secs_f64() * eps as f64) as usize;
        self.generate_batch(total_events)
    }

    fn choose_event_type(&mut self) -> EventType {
        let r: f64 = self.rng.gen();
        match self.profile {
            MockProfile::AnalyticalRead => {
                if r < 0.7 {
                    EventType::Read
                } else if r < 0.9 {
                    EventType::SchedSwitch
                } else {
                    EventType::PageFault
                }
            }
            MockProfile::BatchWrite => {
                if r < 0.5 {
                    EventType::Write
                } else if r < 0.7 {
                    EventType::Read
                } else if r < 0.9 {
                    EventType::Fsync
                } else {
                    EventType::SchedSwitch
                }
            }
            MockProfile::HighContention => {
                if r < 0.4 {
                    EventType::LockWait
                } else if r < 0.6 {
                    EventType::LockWait
                } else if r < 0.8 {
                    EventType::SchedSwitch
                } else {
                    EventType::Read
                }
            }
            _ => {
                if r < 0.3 {
                    EventType::Read
                } else if r < 0.5 {
                    EventType::Write
                } else if r < 0.7 {
                    EventType::SchedSwitch
                } else if r < 0.85 {
                    EventType::PageFault
                } else {
                    EventType::Fsync
                }
            }
        }
    }

    fn generate_latency(&mut self, mean: u64, stddev: u64) -> u64 {
        // Simple approximation of normal distribution using uniform
        let u1: f64 = self.rng.gen();
        let u2: f64 = self.rng.gen();
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        let value = mean as f64 + z * stddev as f64;
        value.max(1.0) as u64
    }

    fn generate_size(&mut self, mean: u64) -> u64 {
        // Exponential distribution for sizes
        let u: f64 = self.rng.gen();
        let value = -(mean as f64) * (1.0 - u).ln();
        value.max(1.0) as u64
    }

    /// Get statistics about generated events
    pub fn stats(&self) -> MockGeneratorStats {
        MockGeneratorStats {
            events_generated: self.event_count,
            elapsed: self.start_time.elapsed(),
            profile: self.profile,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockGeneratorStats {
    pub events_generated: u64,
    pub elapsed: Duration,
    pub profile: MockProfile,
}

impl MockGeneratorStats {
    pub fn events_per_second(&self) -> f64 {
        if self.elapsed.as_secs_f64() == 0.0 {
            return 0.0;
        }
        self.events_generated as f64 / self.elapsed.as_secs_f64()
    }
}

/// Mock daemon server for testing client connections
pub struct MockDaemonServer {
    generator: Arc<Mutex<MockEventGenerator>>,
    connected_clients: Arc<Mutex<u32>>,
    running: Arc<Mutex<bool>>,
}

impl MockDaemonServer {
    pub fn new(profile: MockProfile) -> Self {
        Self {
            generator: Arc::new(Mutex::new(MockEventGenerator::new(profile))),
            connected_clients: Arc::new(Mutex::new(0)),
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Handle a mock request and return a response
    pub fn handle_request(&self, request: &Message) -> Message {
        match &request.payload {
            MessagePayload::GetMetrics { pid } => {
                // Generate mock metrics for the requested PID
                let mock_metrics = super::protocol::MetricsData {
                    pid: *pid,
                    timestamp_ns: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64,
                    total_time_us: 1000,
                    cpu_time_us: 800,
                    io_wait_time_us: 150,
                    lock_wait_time_us: 50,
                    bytes_read: 8192,
                    bytes_written: 4096,
                    read_syscalls: 10,
                    write_syscalls: 5,
                    fsync_count: 1,
                    page_faults: 2,
                    context_switches: 3,
                    events_processed: 100,
                    events_dropped: 0,
                    query_hash: None,
                };

                Message::new(MessagePayload::MetricsResponse(mock_metrics))
            }
            MessagePayload::GetStatus => {
                let status = super::protocol::DaemonStatus {
                    version: "1.0.0-mock".to_string(),
                    uptime_secs: 3600,
                    is_running: true,
                    programs_loaded: 2,
                    programs_attached: 2,
                    tracked_pids: vec![1000, 1001],
                    total_events_processed: self.generator.lock().unwrap().event_count,
                    total_events_dropped: 0,
                    confidence_score: 0.95,
                    cpu_usage_percent: 0.5,
                    memory_usage_bytes: 1024 * 1024,
                };
                Message::new(MessagePayload::StatusResponse(status))
            }
            MessagePayload::SetSamplingRate {
                event_type: _,
                rate: _,
            } => Message::new(MessagePayload::Ok),
            MessagePayload::Shutdown => {
                *self.running.lock().unwrap() = false;
                Message::new(MessagePayload::Ok)
            }
            _ => Message::new(MessagePayload::Error {
                code: 1,
                message: "Unknown or unhandled request".to_string(),
            }),
        }
    }

    /// Simulate a client connection
    pub fn connect(&self) {
        let mut clients = self.connected_clients.lock().unwrap();
        *clients += 1;
    }

    /// Simulate a client disconnection
    pub fn disconnect(&self) {
        let mut clients = self.connected_clients.lock().unwrap();
        if *clients > 0 {
            *clients -= 1;
        }
    }
}

/// Replay provider for captured production events
pub struct ReplayProvider {
    events: Vec<RawEvent>,
    index: usize,
    loop_forever: bool,
}

impl ReplayProvider {
    /// Create from a vector of events
    pub fn from_events(events: Vec<RawEvent>) -> Self {
        Self {
            events,
            index: 0,
            loop_forever: false,
        }
    }

    /// Create from serialized event data (e.g., captured from production)
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        // Each RawEvent is 64 bytes
        let event_size = std::mem::size_of::<RawEvent>();
        if !data.len().is_multiple_of(event_size) {
            return Err("Invalid data length".to_string());
        }

        let num_events = data.len() / event_size;
        let mut events = Vec::with_capacity(num_events);

        for i in 0..num_events {
            let start = i * event_size;
            let end = start + event_size;
            let event_data = &data[start..end];

            match RawEvent::parse(event_data) {
                Ok(event) => events.push(event),
                Err(e) => return Err(format!("Failed to parse event {}: {:?}", i, e)),
            }
        }

        Ok(Self::from_events(events))
    }

    /// Enable looping forever through the events
    pub fn loop_forever(mut self) -> Self {
        self.loop_forever = true;
        self
    }

    /// Get the next event
    pub fn next(&mut self) -> Option<RawEvent> {
        if self.index >= self.events.len() {
            if self.loop_forever {
                self.index = 0;
            } else {
                return None;
            }
        }

        if self.events.is_empty() {
            return None;
        }

        let event = self.events[self.index];
        self.index += 1;
        Some(event)
    }

    /// Get a batch of events
    pub fn next_batch(&mut self, count: usize) -> Vec<RawEvent> {
        (0..count).filter_map(|_| self.next()).collect()
    }

    /// Reset to the beginning
    pub fn reset(&mut self) {
        self.index = 0;
    }

    /// Get total number of events
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Builder for creating custom mock scenarios
pub struct MockScenarioBuilder {
    events: Vec<RawEvent>,
    pids: HashMap<u32, String>, // PID -> query mapping for correlation testing
}

impl MockScenarioBuilder {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            pids: HashMap::new(),
        }
    }

    /// Add a query execution scenario
    pub fn with_query(mut self, pid: u32, query: &str, duration_us: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        // Add Read event (query parsing)
        self.events.push(RawEvent {
            timestamp_ns: timestamp,
            pid,
            tid: pid,
            event_type_raw: EventType::Read as u8,
            _pad: [0; 3],
            value1: duration_us / 10,
            value2: query.len() as u64,
        });

        // Add SchedSwitch event (execution)
        self.events.push(RawEvent {
            timestamp_ns: timestamp + duration_us * 1000 / 2,
            pid,
            tid: pid,
            event_type_raw: EventType::SchedSwitch as u8,
            _pad: [0; 3],
            value1: duration_us / 2,
            value2: 0,
        });

        self.pids.insert(pid, query.to_string());
        self
    }

    /// Add lock contention scenario
    pub fn with_lock_contention(mut self, pid: u32, wait_us: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        self.events.push(RawEvent {
            timestamp_ns: timestamp,
            pid,
            tid: pid,
            event_type_raw: EventType::LockWait as u8,
            _pad: [0; 3],
            value1: wait_us,
            value2: 0,
        });

        self
    }

    /// Add I/O scenario
    pub fn with_io(mut self, pid: u32, read_bytes: u64, write_bytes: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        if read_bytes > 0 {
            self.events.push(RawEvent {
                timestamp_ns: timestamp,
                pid,
                tid: pid,
                event_type_raw: EventType::Read as u8,
                _pad: [0; 3],
                value1: read_bytes / 100, // ~100 bytes/us
                value2: read_bytes,
            });
        }

        if write_bytes > 0 {
            self.events.push(RawEvent {
                timestamp_ns: timestamp + 1000,
                pid,
                tid: pid,
                event_type_raw: EventType::Write as u8,
                _pad: [0; 3],
                value1: write_bytes / 50, // ~50 bytes/us
                value2: write_bytes,
            });
        }

        self
    }

    /// Build the replay provider
    pub fn build(self) -> ReplayProvider {
        ReplayProvider::from_events(self.events)
    }

    /// Get the PID-to-query mapping for correlation testing
    pub fn pid_mapping(&self) -> &HashMap<u32, String> {
        &self.pids
    }
}

impl Default for MockScenarioBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_generator_deterministic() {
        let mut gen1 = MockEventGenerator::with_seed(MockProfile::NormalOltp, 42);
        let mut gen2 = MockEventGenerator::with_seed(MockProfile::NormalOltp, 42);

        let batch1 = gen1.generate_batch(10);
        let batch2 = gen2.generate_batch(10);

        for (e1, e2) in batch1.iter().zip(batch2.iter()) {
            // Copy fields to avoid references to packed struct
            let pid1 = e1.pid;
            let pid2 = e2.pid;
            let et1 = e1.event_type_raw;
            let et2 = e2.event_type_raw;
            let v1 = e1.value1;
            let v2 = e2.value1;
            assert_eq!(pid1, pid2);
            assert_eq!(et1, et2);
            assert_eq!(v1, v2);
        }
    }

    #[test]
    fn test_mock_generator_profiles() {
        for profile in [
            MockProfile::Idle,
            MockProfile::NormalOltp,
            MockProfile::HighLoad,
        ] {
            let mut gen = MockEventGenerator::new(profile);
            let events = gen.generate_batch(100);
            assert_eq!(events.len(), 100);
        }
    }

    #[test]
    fn test_scenario_builder() {
        let scenario = MockScenarioBuilder::new()
            .with_query(1000, "SELECT * FROM users", 5000)
            .with_lock_contention(1000, 1000)
            .with_io(1000, 8192, 4096);

        let provider = scenario.build();
        // with_query: 2 events (Read + SchedSwitch)
        // with_lock_contention: 1 event (LockWait)
        // with_io: 2 events (Read + Write)
        assert_eq!(provider.len(), 5);
    }

    #[test]
    fn test_replay_provider_loop() {
        let events = vec![RawEvent {
            timestamp_ns: 1,
            pid: 1000,
            tid: 1000,
            event_type_raw: EventType::Read as u8,
            _pad: [0; 3],
            value1: 100,
            value2: 1024,
        }];

        let mut provider = ReplayProvider::from_events(events).loop_forever();

        for _ in 0..10 {
            assert!(provider.next().is_some());
        }
    }

    #[test]
    fn test_mock_daemon_get_status() {
        let server = MockDaemonServer::new(MockProfile::Idle);

        let request = Message {
            version: 1,
            payload: MessagePayload::GetStatus,
        };

        let response = server.handle_request(&request);
        assert!(matches!(
            response.payload,
            MessagePayload::StatusResponse(_)
        ));
    }
}
