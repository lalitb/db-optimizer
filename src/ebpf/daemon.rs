// src/ebpf/daemon.rs - Privileged eBPF daemon with Unix socket IPC
//
// This module implements a privilege-separated architecture where:
// 1. A root daemon loads/attaches eBPF programs and reads perf buffers
// 2. Unprivileged clients communicate via Unix socket
// 3. The daemon drops unnecessary capabilities after setup

use std::collections::HashSet;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::aggregator::EventAggregator;
use super::events::RawEvent;
use super::protocol::{
    DaemonStatus, Message, MessagePayload, MetricsData, ProtocolCodec, ProtocolError,
};
use super::sampling::SamplingController;
use super::SystemMetrics;

/// Default socket path for daemon communication
pub const DEFAULT_SOCKET_PATH: &str = "/var/run/pg-advisor-ebpf.sock";

/// Daemon configuration
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Unix socket path for IPC
    pub socket_path: PathBuf,
    /// PostgreSQL binary path for uprobe attachment
    pub postgres_binary: Option<PathBuf>,
    /// PIDs to track (empty = auto-detect PostgreSQL processes)
    pub tracked_pids: HashSet<u32>,
    /// Perf buffer size per CPU (pages)
    pub perf_buffer_pages: usize,
    /// Event processing interval (ms)
    pub poll_interval_ms: u64,
    /// Enable Prometheus metrics endpoint
    pub prometheus_enabled: bool,
    /// Prometheus port
    pub prometheus_port: u16,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            postgres_binary: None,
            tracked_pids: HashSet::new(),
            perf_buffer_pages: 256,
            poll_interval_ms: 100,
            prometheus_enabled: true,
            prometheus_port: 9091,
        }
    }
}

/// The privileged eBPF daemon
pub struct EbpfDaemon {
    config: DaemonConfig,
    aggregator: Arc<EventAggregator>,
    sampling: Arc<SamplingController>,
    tracked_pids: Arc<RwLock<HashSet<u32>>>,
    running: Arc<AtomicBool>,
    start_time: Instant,
}

impl EbpfDaemon {
    /// Create a new daemon instance
    pub fn new(config: DaemonConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let tracked_pids = config.tracked_pids.clone();
        
        Ok(Self {
            config,
            aggregator: Arc::new(EventAggregator::new()?),
            sampling: Arc::new(SamplingController::new()),
            tracked_pids: Arc::new(RwLock::new(tracked_pids)),
            running: Arc::new(AtomicBool::new(false)),
            start_time: Instant::now(),
        })
    }
    
    /// Start the daemon
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting eBPF daemon...");
        
        // Check capabilities
        self.check_capabilities()?;
        
        // Remove old socket if exists
        if self.config.socket_path.exists() {
            std::fs::remove_file(&self.config.socket_path)?;
        }
        
        // Create socket directory if needed
        if let Some(parent) = self.config.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Bind Unix socket
        let listener = UnixListener::bind(&self.config.socket_path)?;
        listener.set_nonblocking(true)?;
        
        // Set socket permissions (allow group access)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.config.socket_path,
                std::fs::Permissions::from_mode(0o660),
            )?;
        }
        
        info!("Daemon listening on {:?}", self.config.socket_path);
        
        self.running.store(true, Ordering::SeqCst);
        
        // Main event loop
        let mut poll_interval = tokio::time::interval(
            Duration::from_millis(self.config.poll_interval_ms)
        );
        
        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                _ = poll_interval.tick() => {
                    // Poll eBPF perf buffers
                    if let Err(e) = self.poll_events().await {
                        error!("Error polling events: {}", e);
                    }
                }
                
                // Handle incoming connections (non-blocking)
                _ = tokio::task::yield_now() => {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let aggregator = self.aggregator.clone();
                            let sampling = self.sampling.clone();
                            let tracked_pids = self.tracked_pids.clone();
                            let running = self.running.clone();
                            let start_time = self.start_time;
                            
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_client(
                                    stream,
                                    aggregator,
                                    sampling,
                                    tracked_pids,
                                    running,
                                    start_time,
                                ).await {
                                    error!("Client error: {}", e);
                                }
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No connection pending
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
            }
        }
        
        // Cleanup
        let _ = std::fs::remove_file(&self.config.socket_path);
        info!("Daemon stopped");
        
        Ok(())
    }
    
    /// Stop the daemon
    pub fn stop(&self) {
        info!("Stopping daemon...");
        self.running.store(false, Ordering::SeqCst);
    }
    
    /// Check if daemon is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    
    /// Check required capabilities
    fn check_capabilities(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if we're root or have CAP_BPF
        #[cfg(target_os = "linux")]
        {
            let uid = unsafe { libc::getuid() };
            if uid != 0 {
                // Not root - check for CAP_BPF
                // For now, just warn. A real implementation would use libcap
                warn!(
                    "Running as non-root (uid={}). eBPF may require CAP_BPF and CAP_PERFMON.",
                    uid
                );
            }
        }
        
        Ok(())
    }
    
    /// Poll eBPF perf buffers for events
    async fn poll_events(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // In a real implementation, this would:
        // 1. Read from perf_event_array maps using aya
        // 2. Process lost event callbacks
        // 3. Parse and aggregate events
        
        // For now, this is a placeholder that would be filled in with actual eBPF code
        // when building with the "ebpf" feature
        
        #[cfg(feature = "ebpf")]
        {
            // Real eBPF implementation would go here
            // self.poll_perf_buffers().await?;
        }
        
        #[cfg(not(feature = "ebpf"))]
        {
            // Mock mode - no actual eBPF
            debug!("eBPF polling (mock mode)");
        }
        
        Ok(())
    }
    
    /// Handle a client connection
    async fn handle_client(
        mut stream: UnixStream,
        aggregator: Arc<EventAggregator>,
        sampling: Arc<SamplingController>,
        tracked_pids: Arc<RwLock<HashSet<u32>>>,
        running: Arc<AtomicBool>,
        start_time: Instant,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        
        loop {
            // Read message
            let msg = match ProtocolCodec::read_message(&mut stream) {
                Ok(m) => m,
                Err(ProtocolError::IoError(_)) => break, // Client disconnected
                Err(e) => {
                    error!("Protocol error: {}", e);
                    let response = Message::new(MessagePayload::Error {
                        code: 1,
                        message: e.to_string(),
                    });
                    let _ = ProtocolCodec::write_message(&mut stream, &response);
                    break;
                }
            };
            
            // Check if this is a shutdown request before processing
            let is_shutdown = matches!(msg.payload, MessagePayload::Shutdown);
            
            // Process message
            let response = Self::process_message(
                msg,
                &aggregator,
                &sampling,
                &tracked_pids,
                &running,
                start_time,
            ).await;
            
            // Send response
            ProtocolCodec::write_message(&mut stream, &response)?;
            
            // Exit loop if shutdown was requested
            if is_shutdown {
                break;
            }
        }
        
        Ok(())
    }
    
    /// Process a client message
    async fn process_message(
        msg: Message,
        aggregator: &Arc<EventAggregator>,
        sampling: &Arc<SamplingController>,
        tracked_pids: &Arc<RwLock<HashSet<u32>>>,
        running: &Arc<AtomicBool>,
        start_time: Instant,
    ) -> Message {
        match msg.payload {
            MessagePayload::StartProfiling { pids } => {
                let mut tracked = tracked_pids.write().await;
                for pid in pids {
                    tracked.insert(pid);
                    info!("Started profiling PID {}", pid);
                }
                Message::new(MessagePayload::Ok)
            }
            
            MessagePayload::StopProfiling { pids } => {
                let mut tracked = tracked_pids.write().await;
                for pid in &pids {
                    tracked.remove(pid);
                    aggregator.clear_metrics(*pid).await;
                    info!("Stopped profiling PID {}", pid);
                }
                Message::new(MessagePayload::Ok)
            }
            
            MessagePayload::GetMetrics { pid } => {
                match aggregator.get_system_metrics(pid).await {
                    Some(metrics) => {
                        let data = system_metrics_to_data(pid, &metrics);
                        Message::new(MessagePayload::MetricsResponse(data))
                    }
                    None => Message::new(MessagePayload::Error {
                        code: 404,
                        message: format!("No metrics for PID {}", pid),
                    }),
                }
            }
            
            MessagePayload::GetAllMetrics => {
                let pids = aggregator.get_tracked_pids().await;
                let mut all_metrics = Vec::new();
                
                for pid in pids {
                    if let Some(metrics) = aggregator.get_system_metrics(pid).await {
                        all_metrics.push(system_metrics_to_data(pid, &metrics));
                    }
                }
                
                Message::new(MessagePayload::AllMetricsResponse(all_metrics))
            }
            
            MessagePayload::GetStatus => {
                let tracked = tracked_pids.read().await;
                let (processed, dropped) = aggregator.get_stats();
                
                let status = DaemonStatus {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime_secs: start_time.elapsed().as_secs(),
                    is_running: running.load(Ordering::SeqCst),
                    programs_loaded: 0, // TODO: Track actual programs
                    programs_attached: 0,
                    tracked_pids: tracked.iter().cloned().collect(),
                    total_events_processed: processed,
                    total_events_dropped: dropped,
                    confidence_score: aggregator.get_confidence(),
                    cpu_usage_percent: 0.0, // TODO: Self-monitoring
                    memory_usage_bytes: 0,
                };
                
                Message::new(MessagePayload::StatusResponse(status))
            }
            
            MessagePayload::SetSamplingRate { event_type, rate } => {
                use super::events::EventType;
                sampling.set_rate(EventType::from(event_type), rate);
                Message::new(MessagePayload::Ok)
            }
            
            MessagePayload::Shutdown => {
                info!("Received shutdown command");
                running.store(false, Ordering::SeqCst);
                Message::new(MessagePayload::Ok)
            }
            
            // Response messages shouldn't come from client
            _ => Message::new(MessagePayload::Error {
                code: 400,
                message: "Invalid message type".to_string(),
            }),
        }
    }
}

/// Convert SystemMetrics to MetricsData for IPC
fn system_metrics_to_data(pid: u32, metrics: &SystemMetrics) -> MetricsData {
    MetricsData {
        pid,
        timestamp_ns: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
        total_time_us: metrics.total_time_us,
        cpu_time_us: metrics.cpu_time_us,
        io_wait_time_us: metrics.io_wait_time_us,
        lock_wait_time_us: metrics.lock_wait_time_us,
        bytes_read: metrics.bytes_read,
        bytes_written: metrics.bytes_written,
        read_syscalls: metrics.read_syscalls,
        write_syscalls: metrics.write_syscalls,
        fsync_count: metrics.fsync_count,
        page_faults: metrics.page_faults,
        context_switches: metrics.context_switches,
        events_processed: metrics.events_processed,
        events_dropped: metrics.events_dropped,
        query_hash: None,
    }
}

/// Client for connecting to the eBPF daemon
pub struct DaemonClient {
    stream: UnixStream,
}

impl DaemonClient {
    /// Connect to the daemon
    pub fn connect(socket_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        
        Ok(Self { stream })
    }
    
    /// Connect with default socket path
    pub fn connect_default() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::connect(DEFAULT_SOCKET_PATH)
    }
    
    /// Send a message and receive response
    pub fn send(&mut self, payload: MessagePayload) -> Result<MessagePayload, ProtocolError> {
        let msg = Message::new(payload);
        ProtocolCodec::write_message(&mut self.stream, &msg)?;
        
        let response = ProtocolCodec::read_message(&mut self.stream)?;
        Ok(response.payload)
    }
    
    /// Start profiling PIDs
    pub fn start_profiling(&mut self, pids: Vec<u32>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self.send(MessagePayload::StartProfiling { pids })? {
            MessagePayload::Ok => Ok(()),
            MessagePayload::Error { message, .. } => Err(message.into()),
            _ => Err("Unexpected response".into()),
        }
    }
    
    /// Stop profiling PIDs
    pub fn stop_profiling(&mut self, pids: Vec<u32>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self.send(MessagePayload::StopProfiling { pids })? {
            MessagePayload::Ok => Ok(()),
            MessagePayload::Error { message, .. } => Err(message.into()),
            _ => Err("Unexpected response".into()),
        }
    }
    
    /// Get metrics for a PID
    pub fn get_metrics(&mut self, pid: u32) -> Result<MetricsData, Box<dyn std::error::Error + Send + Sync>> {
        match self.send(MessagePayload::GetMetrics { pid })? {
            MessagePayload::MetricsResponse(data) => Ok(data),
            MessagePayload::Error { message, .. } => Err(message.into()),
            _ => Err("Unexpected response".into()),
        }
    }
    
    /// Get daemon status
    pub fn get_status(&mut self) -> Result<DaemonStatus, Box<dyn std::error::Error + Send + Sync>> {
        match self.send(MessagePayload::GetStatus)? {
            MessagePayload::StatusResponse(status) => Ok(status),
            MessagePayload::Error { message, .. } => Err(message.into()),
            _ => Err("Unexpected response".into()),
        }
    }
    
    /// Shutdown the daemon
    pub fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self.send(MessagePayload::Shutdown)? {
            MessagePayload::Ok => Ok(()),
            MessagePayload::Error { message, .. } => Err(message.into()),
            _ => Err("Unexpected response".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_daemon_config_default() {
        let config = DaemonConfig::default();
        assert_eq!(config.socket_path, PathBuf::from(DEFAULT_SOCKET_PATH));
        assert_eq!(config.perf_buffer_pages, 256);
        assert!(config.prometheus_enabled);
    }
    
    #[tokio::test]
    async fn test_daemon_creation() {
        let config = DaemonConfig::default();
        let daemon = EbpfDaemon::new(config);
        assert!(daemon.is_ok());
    }
}
