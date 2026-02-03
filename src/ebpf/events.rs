// src/ebpf/events.rs - Memory-safe event structures for eBPF
//
// These structures are designed to be safely parsed from raw bytes
// coming from the kernel perf ring buffer. Uses bytemuck for zero-copy
// parsing with full validation.

use bytemuck::{Pod, Zeroable};
use chrono::{DateTime, TimeZone, Utc};
use std::fmt;

/// Event types matching bpf/events.bpf.c
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventType {
    Read = 1,
    Write = 2,
    SchedSwitch = 3,
    PageFault = 4,
    QueryStart = 5,
    QueryEnd = 6,
    Fsync = 7,
    LockWait = 8,
    Unknown = 255,
}

impl From<u8> for EventType {
    fn from(val: u8) -> Self {
        match val {
            1 => EventType::Read,
            2 => EventType::Write,
            3 => EventType::SchedSwitch,
            4 => EventType::PageFault,
            5 => EventType::QueryStart,
            6 => EventType::QueryEnd,
            7 => EventType::Fsync,
            8 => EventType::LockWait,
            _ => EventType::Unknown,
        }
    }
}

/// Lock contention event emitted by bpf/lock_contention.bpf.c
///
/// This must match `struct lock_contention_event` exactly.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LockContentionEventRaw {
    pub timestamp_ns: u64,
    pub blocking_pid: u32,
    pub waiting_pid: u32,
    pub blocking_query_hash: u64,
    pub waiting_query_hash: u64,
    pub wait_duration_ns: u64,
    pub lock_type: u8,
    pub lock_mode: u8,
    pub _pad: [u8; 2],
}

impl LockContentionEventRaw {
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < std::mem::size_of::<Self>() {
            return Err(ParseError::InsufficientData {
                expected: std::mem::size_of::<Self>(),
                actual: data.len(),
            });
        }

        let event = bytemuck::try_pod_read_unaligned::<Self>(&data[..std::mem::size_of::<Self>()])
            .map_err(|e| ParseError::InvalidData(e.to_string()))?;

        if event.timestamp_ns == 0 {
            return Err(ParseError::InvalidTimestamp(event.timestamp_ns));
        }

        Ok(event)
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        // Saturate to prevent panic on invalid timestamps
        let secs = (self.timestamp_ns / 1_000_000_000) as i64;
        let nsecs = (self.timestamp_ns % 1_000_000_000) as u32;
        Utc.timestamp_opt(secs, nsecs)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_else(Utc::now))
    }
}

/// LWLock event (skeleton; lock_id/mode depend on probe wiring)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LwLockEventRaw {
    pub timestamp_ns: u64,
    pub pid: u32,
    pub lock_id: u32,
    pub mode: u32,
    pub wait_duration_ns: u64,
}

impl LwLockEventRaw {
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < std::mem::size_of::<Self>() {
            return Err(ParseError::InsufficientData {
                expected: std::mem::size_of::<Self>(),
                actual: data.len(),
            });
        }

        let event = bytemuck::try_pod_read_unaligned::<Self>(&data[..std::mem::size_of::<Self>()])
            .map_err(|e| ParseError::InvalidData(e.to_string()))?;

        if event.timestamp_ns == 0 {
            return Err(ParseError::InvalidTimestamp(event.timestamp_ns));
        }

        Ok(event)
    }
}

/// Row lock event (skeleton; relfilenode/tuple_offset depend on probe wiring)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RowLockEventRaw {
    pub timestamp_ns: u64,
    pub pid: u32,
    pub relfilenode: u64,
    pub tuple_offset: u64,
    pub lock_mode: u32,
    pub wait_duration_ns: u64,
}

impl RowLockEventRaw {
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < std::mem::size_of::<Self>() {
            return Err(ParseError::InsufficientData {
                expected: std::mem::size_of::<Self>(),
                actual: data.len(),
            });
        }

        let event = bytemuck::try_pod_read_unaligned::<Self>(&data[..std::mem::size_of::<Self>()])
            .map_err(|e| ParseError::InvalidData(e.to_string()))?;

        if event.timestamp_ns == 0 {
            return Err(ParseError::InvalidTimestamp(event.timestamp_ns));
        }

        Ok(event)
    }
}

/// Raw event structure from kernel
///
/// This must match the `struct event` in bpf/events.bpf.c exactly.
/// Using Pod/Zeroable for safe transmutation from raw bytes.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RawEvent {
    pub timestamp_ns: u64,
    pub pid: u32,
    pub tid: u32,
    pub event_type_raw: u8,
    pub _pad: [u8; 3],
    pub value1: u64,
    pub value2: u64,
}

impl RawEvent {
    /// Get typed event type
    pub fn event_type(&self) -> EventType {
        EventType::from(self.event_type_raw)
    }

    /// Parse from raw bytes with validation
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        // Validate size
        if data.len() < std::mem::size_of::<Self>() {
            return Err(ParseError::InsufficientData {
                expected: std::mem::size_of::<Self>(),
                actual: data.len(),
            });
        }

        // Safe transmute with bytemuck
        let event = bytemuck::try_pod_read_unaligned::<Self>(&data[..std::mem::size_of::<Self>()])
            .map_err(|e| ParseError::InvalidData(e.to_string()))?;

        // Validate timestamp (sanity check - should be reasonable)
        if event.timestamp_ns == 0 {
            return Err(ParseError::InvalidTimestamp(event.timestamp_ns));
        }

        // Validate PID (0 is kernel, should typically be > 0 for userspace)
        // We allow 0 for kernel threads though

        Ok(event)
    }

    /// Size of the event structure
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl fmt::Display for RawEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Copy fields to avoid references to packed struct
        let pid = self.pid;
        let tid = self.tid;
        let ts = self.timestamp_ns;
        let v1 = self.value1;
        let v2 = self.value2;
        write!(
            f,
            "Event {{ type: {:?}, pid: {}, tid: {}, ts: {}, v1: {}, v2: {} }}",
            self.event_type(),
            pid,
            tid,
            ts,
            v1,
            v2
        )
    }
}

/// PostgreSQL-specific event from uprobes
///
/// This matches `struct pg_event` in bpf/postgres.bpf.c
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PostgresEvent {
    pub timestamp_ns: u64,
    pub pid: u32,
    pub event_type: u8,
    pub _pad: [u8; 3],
    pub query_hash: u64,
    pub value: u64,
}

/// PostgreSQL event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PgEventType {
    QueryStart = 100,
    QueryEnd = 101,
    ParseStart = 102,
    Bind = 103,
    Execute = 104,
    Unknown = 255,
}

impl From<u8> for PgEventType {
    fn from(val: u8) -> Self {
        match val {
            100 => PgEventType::QueryStart,
            101 => PgEventType::QueryEnd,
            102 => PgEventType::ParseStart,
            103 => PgEventType::Bind,
            104 => PgEventType::Execute,
            _ => PgEventType::Unknown,
        }
    }
}

impl PostgresEvent {
    /// Get typed event type
    pub fn pg_event_type(&self) -> PgEventType {
        PgEventType::from(self.event_type)
    }

    /// Parse from raw bytes
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < std::mem::size_of::<Self>() {
            return Err(ParseError::InsufficientData {
                expected: std::mem::size_of::<Self>(),
                actual: data.len(),
            });
        }

        let event = bytemuck::try_pod_read_unaligned::<Self>(&data[..std::mem::size_of::<Self>()])
            .map_err(|e| ParseError::InvalidData(e.to_string()))?;

        Ok(event)
    }
}

/// Parse errors for event data
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Insufficient data: expected {expected} bytes, got {actual}")]
    InsufficientData { expected: usize, actual: usize },

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Invalid event type: {0}")]
    InvalidEventType(u8),

    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_event_size() {
        // Ensure structure is properly packed
        // Layout: timestamp_ns(8) + pid(4) + tid(4) + event_type_raw(1) + _pad(3) + value1(8) + value2(8) = 36
        assert_eq!(RawEvent::size(), 36);
    }

    #[test]
    fn test_event_type_conversion() {
        assert_eq!(EventType::from(1), EventType::Read);
        assert_eq!(EventType::from(2), EventType::Write);
        assert_eq!(EventType::from(99), EventType::Unknown);
    }

    #[test]
    fn test_raw_event_parse() {
        let mut data = [0u8; 36];
        // timestamp_ns = 1000000
        data[0..8].copy_from_slice(&1000000u64.to_ne_bytes());
        // pid = 123
        data[8..12].copy_from_slice(&123u32.to_ne_bytes());
        // tid = 123
        data[12..16].copy_from_slice(&123u32.to_ne_bytes());
        // event_type = 1 (Read)
        data[16] = 1;
        // _pad = data[17..20] (leave as zeros)
        // value1 = 4096 (offset 20)
        data[20..28].copy_from_slice(&4096u64.to_ne_bytes());
        // value2 = 0 (offset 28..36, leave as zeros)

        let event = RawEvent::parse(&data).unwrap();
        // Copy values to avoid references to packed struct fields
        let pid = event.pid;
        let value1 = event.value1;
        assert_eq!(pid, 123);
        assert_eq!(event.event_type(), EventType::Read);
        assert_eq!(value1, 4096);
    }

    #[test]
    fn test_insufficient_data() {
        let data = [0u8; 10];
        let result = RawEvent::parse(&data);
        assert!(matches!(result, Err(ParseError::InsufficientData { .. })));
    }

    #[test]
    fn test_postgres_event_types() {
        assert_eq!(PgEventType::from(100), PgEventType::QueryStart);
        assert_eq!(PgEventType::from(101), PgEventType::QueryEnd);
        assert_eq!(PgEventType::from(50), PgEventType::Unknown);
    }
}
