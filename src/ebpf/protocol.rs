// src/ebpf/protocol.rs - Binary IPC protocol for daemon communication
//
// This module defines the protocol for communication between the
// privileged eBPF daemon and unprivileged client processes. Uses
// bincode for efficient binary serialization with version tagging.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use thiserror::Error;

/// Current protocol version
pub const PROTOCOL_VERSION: u8 = 1;

/// Maximum message size (1MB)
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// Message wrapper with version for compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub version: u8,
    pub payload: MessagePayload,
}

impl Message {
    pub fn new(payload: MessagePayload) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            payload,
        }
    }
}

/// All possible message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessagePayload {
    // === Commands (client -> daemon) ===
    
    /// Start profiling specific PIDs
    StartProfiling { pids: Vec<u32> },
    
    /// Stop profiling specific PIDs
    StopProfiling { pids: Vec<u32> },
    
    /// Get metrics for a specific PID
    GetMetrics { pid: u32 },
    
    /// Get all metrics
    GetAllMetrics,
    
    /// Get daemon status
    GetStatus,
    
    /// Set sampling rate (1 = every event, N = 1 in N events)
    SetSamplingRate { 
        event_type: u8,
        rate: u32,
    },
    
    /// Shutdown the daemon
    Shutdown,
    
    // === Responses (daemon -> client) ===
    
    /// Metrics response for a single PID
    MetricsResponse(MetricsData),
    
    /// All metrics response
    AllMetricsResponse(Vec<MetricsData>),
    
    /// Daemon status response
    StatusResponse(DaemonStatus),
    
    /// Success acknowledgment
    Ok,
    
    /// Error response
    Error { code: u32, message: String },
}

/// Metrics data for a single PID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsData {
    pub pid: u32,
    pub timestamp_ns: u64,
    
    // Time breakdown (microseconds)
    pub total_time_us: u64,
    pub cpu_time_us: u64,
    pub io_wait_time_us: u64,
    pub lock_wait_time_us: u64,
    
    // I/O metrics
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_syscalls: u32,
    pub write_syscalls: u32,
    pub fsync_count: u32,
    
    // Memory metrics
    pub page_faults: u32,
    pub context_switches: u32,
    
    // Event quality
    pub events_processed: u64,
    pub events_dropped: u64,
    
    // Query correlation
    pub query_hash: Option<u64>,
}

/// Daemon status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub uptime_secs: u64,
    pub is_running: bool,
    pub programs_loaded: usize,
    pub programs_attached: usize,
    pub tracked_pids: Vec<u32>,
    pub total_events_processed: u64,
    pub total_events_dropped: u64,
    pub confidence_score: f64,
    pub cpu_usage_percent: f64,
    pub memory_usage_bytes: u64,
}

/// Protocol codec for encoding/decoding messages
pub struct ProtocolCodec;

impl ProtocolCodec {
    /// Encode a message to bytes with length prefix
    pub fn encode(msg: &Message) -> Result<Vec<u8>, ProtocolError> {
        let payload = bincode::serialize(msg)
            .map_err(|e| ProtocolError::SerializationError(e.to_string()))?;
        
        if payload.len() > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge {
                size: payload.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }
        
        // 4-byte length prefix (little endian)
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);
        
        Ok(frame)
    }
    
    /// Decode a message from bytes (expects length-prefixed data)
    pub fn decode(data: &[u8]) -> Result<Message, ProtocolError> {
        if data.len() < 4 {
            return Err(ProtocolError::IncompleteFrame);
        }
        
        // Read length prefix
        let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        
        if len > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge {
                size: len,
                max: MAX_MESSAGE_SIZE,
            });
        }
        
        if data.len() < 4 + len {
            return Err(ProtocolError::IncompleteMessage {
                expected: len,
                actual: data.len() - 4,
            });
        }
        
        // Deserialize payload
        let msg: Message = bincode::deserialize(&data[4..4 + len])
            .map_err(|e| ProtocolError::DeserializationError(e.to_string()))?;
        
        // Version compatibility check
        if msg.version > PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion {
                received: msg.version,
                supported: PROTOCOL_VERSION,
            });
        }
        
        Ok(msg)
    }
    
    /// Read a complete message from a reader
    pub fn read_message<R: Read>(reader: &mut R) -> Result<Message, ProtocolError> {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)
            .map_err(|e| ProtocolError::IoError(e.to_string()))?;
        
        let len = u32::from_le_bytes(len_buf) as usize;
        
        if len > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge {
                size: len,
                max: MAX_MESSAGE_SIZE,
            });
        }
        
        // Read payload
        let mut payload = vec![0u8; len];
        reader.read_exact(&mut payload)
            .map_err(|e| ProtocolError::IoError(e.to_string()))?;
        
        // Deserialize
        let msg: Message = bincode::deserialize(&payload)
            .map_err(|e| ProtocolError::DeserializationError(e.to_string()))?;
        
        Ok(msg)
    }
    
    /// Write a complete message to a writer
    pub fn write_message<W: Write>(writer: &mut W, msg: &Message) -> Result<(), ProtocolError> {
        let encoded = Self::encode(msg)?;
        writer.write_all(&encoded)
            .map_err(|e| ProtocolError::IoError(e.to_string()))?;
        writer.flush()
            .map_err(|e| ProtocolError::IoError(e.to_string()))?;
        Ok(())
    }
}

/// Protocol errors
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Incomplete frame: need at least 4 bytes for length prefix")]
    IncompleteFrame,
    
    #[error("Incomplete message: expected {expected} bytes, got {actual}")]
    IncompleteMessage { expected: usize, actual: usize },
    
    #[error("Message too large: {size} bytes exceeds max of {max}")]
    MessageTooLarge { size: usize, max: usize },
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    
    #[error("Unsupported protocol version: received {received}, supported up to {supported}")]
    UnsupportedVersion { received: u8, supported: u8 },
    
    #[error("I/O error: {0}")]
    IoError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_encode_decode_roundtrip() {
        let msg = Message::new(MessagePayload::GetMetrics { pid: 12345 });
        
        let encoded = ProtocolCodec::encode(&msg).unwrap();
        let decoded = ProtocolCodec::decode(&encoded).unwrap();
        
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        match decoded.payload {
            MessagePayload::GetMetrics { pid } => assert_eq!(pid, 12345),
            _ => panic!("Wrong payload type"),
        }
    }
    
    #[test]
    fn test_metrics_response() {
        let data = MetricsData {
            pid: 123,
            timestamp_ns: 1234567890,
            total_time_us: 1000,
            cpu_time_us: 800,
            io_wait_time_us: 200,
            lock_wait_time_us: 0,
            bytes_read: 4096,
            bytes_written: 0,
            read_syscalls: 10,
            write_syscalls: 0,
            fsync_count: 0,
            page_faults: 5,
            context_switches: 2,
            events_processed: 100,
            events_dropped: 0,
            query_hash: Some(0xdeadbeef),
        };
        
        let msg = Message::new(MessagePayload::MetricsResponse(data));
        let encoded = ProtocolCodec::encode(&msg).unwrap();
        let decoded = ProtocolCodec::decode(&encoded).unwrap();
        
        match decoded.payload {
            MessagePayload::MetricsResponse(d) => {
                assert_eq!(d.pid, 123);
                assert_eq!(d.bytes_read, 4096);
                assert_eq!(d.query_hash, Some(0xdeadbeef));
            }
            _ => panic!("Wrong payload type"),
        }
    }
    
    #[test]
    fn test_incomplete_frame() {
        let data = [0u8; 2];
        let result = ProtocolCodec::decode(&data);
        assert!(matches!(result, Err(ProtocolError::IncompleteFrame)));
    }
    
    #[test]
    fn test_message_too_large() {
        // Craft a fake length prefix indicating huge message
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(&(MAX_MESSAGE_SIZE as u32 + 1).to_le_bytes());
        
        let result = ProtocolCodec::decode(&data);
        assert!(matches!(result, Err(ProtocolError::MessageTooLarge { .. })));
    }
}
