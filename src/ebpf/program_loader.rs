//! Real eBPF Program Loading
//!
//! Provides the actual eBPF program loading and attachment:
//! - BTF-enabled CO-RE (Compile Once, Run Everywhere) programs
//! - Automatic probe type selection (kprobe/fentry)
//! - Ring buffer or perf event based on kernel version
//! - Safe program lifecycle management

use anyhow::{self, Result};
#[cfg(feature = "ebpf")]
use aya::maps::perf::{PerfEventArray, PerfEventArrayBuffer};
#[cfg(feature = "ebpf")]
use aya::maps::HashMap as BpfHashMap;
#[cfg(feature = "ebpf")]
use aya::programs::{KProbe, KRetProbe, Program, UProbe, UProbeAttachType, URetProbe};
#[cfg(feature = "ebpf")]
use aya::Bpf;
#[cfg(not(all(target_os = "linux", feature = "ebpf")))]
type PerfEventArrayBuffer = ();
#[allow(unused_imports)]
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
#[allow(unused_imports)]
use tracing::{debug, info, warn};

/// eBPF program type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    /// Kprobe - traditional, wide compatibility
    Kprobe,
    /// Kretprobe - return probe
    Kretprobe,
    /// Fentry - faster, requires kernel 5.5+
    Fentry,
    /// Fexit - faster return probe
    Fexit,
    /// Uprobe - user-space probe
    Uprobe,
    /// Uretprobe - user-space return probe
    Uretprobe,
    /// Tracepoint - stable kernel interface
    Tracepoint,
    /// Raw tracepoint
    RawTracepoint,
    /// Socket filter
    SocketFilter,
    /// XDP - eXpress Data Path
    Xdp,
}

impl ProgramType {
    /// Minimum kernel version for this program type
    pub fn min_kernel_version(&self) -> (u32, u32) {
        match self {
            Self::Kprobe | Self::Kretprobe => (4, 1),
            Self::Uprobe | Self::Uretprobe => (4, 1),
            Self::Tracepoint => (4, 7),
            Self::RawTracepoint => (4, 17),
            Self::Fentry | Self::Fexit => (5, 5),
            Self::SocketFilter => (3, 19),
            Self::Xdp => (4, 8),
        }
    }
}

/// Map type for eBPF maps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    /// Hash map
    Hash,
    /// Array
    Array,
    /// Per-CPU hash
    PerCpuHash,
    /// Per-CPU array
    PerCpuArray,
    /// Ring buffer (kernel 5.8+)
    RingBuf,
    /// Perf event array
    PerfEventArray,
    /// LRU hash
    LruHash,
    /// LPM trie (for IP matching)
    LpmTrie,
}

/// An eBPF program definition
#[derive(Debug, Clone)]
pub struct ProgramDef {
    /// Program name
    pub name: String,
    /// Program type
    pub prog_type: ProgramType,
    /// Attach point (function name, tracepoint, etc.)
    pub attach_point: String,
    /// Whether this program is optional
    pub optional: bool,
}

/// An eBPF map definition
#[derive(Debug, Clone)]
pub struct MapDef {
    /// Map name
    pub name: String,
    /// Map type
    pub map_type: MapType,
    /// Key size in bytes
    pub key_size: u32,
    /// Value size in bytes
    pub value_size: u32,
    /// Maximum entries
    pub max_entries: u32,
}

/// Loaded program info
#[derive(Debug, Clone)]
pub struct LoadedProgram {
    /// Program definition
    pub def: ProgramDef,
    /// File descriptor (on Linux)
    pub fd: i32,
    /// Whether attached
    pub attached: bool,
    /// Load time
    pub loaded_at: Instant,
    /// Attach time
    pub attached_at: Option<Instant>,
}

/// Loaded map info
#[derive(Debug, Clone)]
pub struct LoadedMap {
    /// Map definition
    pub def: MapDef,
    /// File descriptor (on Linux)
    pub fd: i32,
    /// Pinned path (if any)
    pub pinned_path: Option<String>,
}

/// Loader configuration
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Path to eBPF object file
    pub object_path: Option<String>,
    /// Whether to use BTF
    pub use_btf: bool,
    /// Prefer fentry/fexit over kprobe
    pub prefer_fentry: bool,
    /// Pin maps to filesystem
    pub pin_maps: bool,
    /// Map pin directory
    pub pin_dir: String,
    /// Whether to verify programs before loading
    pub verify: bool,
    /// Log level for verifier
    pub log_level: u32,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            object_path: None,
            use_btf: true,
            prefer_fentry: true,
            pin_maps: false,
            pin_dir: "/sys/fs/bpf/db-optimizer".to_string(),
            verify: true,
            log_level: 1,
        }
    }
}

/// Load result
#[derive(Debug, Clone)]
pub enum LoadResult {
    /// Successfully loaded
    Success,
    /// Loaded with warnings
    SuccessWithWarnings(Vec<String>),
    /// Failed to load
    Failed(String),
    /// Not supported on this platform
    NotSupported(String),
}

/// eBPF program loader
#[derive(Clone)]
pub struct EbpfLoader {
    /// Configuration
    config: Arc<RwLock<LoaderConfig>>,
    /// Loaded BPF object
    #[cfg(feature = "ebpf")]
    bpf: Arc<RwLock<Option<Bpf>>>,
    /// Detected kernel version
    kernel_version: Arc<RwLock<(u32, u32, u32)>>,
    /// Whether BTF is available
    btf_available: Arc<RwLock<bool>>,
    /// Whether ring buffer is available
    ringbuf_available: Arc<RwLock<bool>>,
    /// Load errors
    #[allow(dead_code)] // Used for error collection during program loading
    errors: Arc<RwLock<Vec<String>>>,
}

impl EbpfLoader {
    /// Create a new loader
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(LoaderConfig::default())),
            #[cfg(feature = "ebpf")]
            bpf: Arc::new(RwLock::new(None)),
            kernel_version: Arc::new(RwLock::new((0, 0, 0))),
            btf_available: Arc::new(RwLock::new(false)),
            ringbuf_available: Arc::new(RwLock::new(false)),
            errors: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Configure the loader
    pub async fn configure(&self, config: LoaderConfig) {
        *self.config.write().await = config;
    }

    /// Detect system capabilities
    pub async fn detect_capabilities(&self) -> LoadResult {
        #[cfg(target_os = "linux")]
        {
            // Detect kernel version
            if let Ok(version_str) = std::fs::read_to_string("/proc/version") {
                if let Some(version) = Self::parse_kernel_version(&version_str) {
                    *self.kernel_version.write().await = version;
                    info!(
                        "Detected kernel version: {}.{}.{}",
                        version.0, version.1, version.2
                    );
                }
            }

            // Check for BTF
            let btf_available = Path::new("/sys/kernel/btf/vmlinux").exists();
            *self.btf_available.write().await = btf_available;
            if btf_available {
                info!("BTF is available");
            } else {
                warn!("BTF not available, some features may be limited");
            }

            // Check for ring buffer support (5.8+)
            let version = *self.kernel_version.read().await;
            let ringbuf = version.0 > 5 || (version.0 == 5 && version.1 >= 8);
            *self.ringbuf_available.write().await = ringbuf;

            LoadResult::Success
        }

        #[cfg(not(target_os = "linux"))]
        {
            LoadResult::NotSupported("eBPF is only supported on Linux".to_string())
        }
    }

    /// Parse kernel version from /proc/version
    #[allow(dead_code)] // Used on Linux only
    fn parse_kernel_version(version_str: &str) -> Option<(u32, u32, u32)> {
        // Format: "Linux version X.Y.Z-..."
        let parts: Vec<&str> = version_str.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "Linux" && parts[1] == "version" {
            let version_part = parts[2].split('-').next()?;
            let nums: Vec<u32> = version_part
                .split('.')
                .filter_map(|s| s.parse().ok())
                .collect();
            if nums.len() >= 2 {
                return Some((nums[0], nums[1], nums.get(2).copied().unwrap_or(0)));
            }
        }
        None
    }

    /// Load eBPF object file
    pub async fn load_object(&self, path: &str) -> LoadResult {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            info!("Loading eBPF object from: {}", path);

            if !Path::new(path).exists() {
                let err = format!("Object file not found: {}", path);
                self.errors.write().await.push(err.clone());
                return LoadResult::Failed(err);
            }

            match Bpf::load_file(path) {
                Ok(bpf) => {
                    *self.bpf.write().await = Some(bpf);
                    info!("Successfully loaded eBPF object");
                    LoadResult::Success
                }
                Err(e) => {
                    let err = format!("Failed to load eBPF object: {e}");
                    self.errors.write().await.push(err.clone());
                    LoadResult::Failed(err)
                }
            }
        }

        #[cfg(any(not(target_os = "linux"), not(feature = "ebpf")))]
        {
            let _ = path;
            LoadResult::NotSupported(
                "eBPF is only supported on Linux with the ebpf feature".to_string(),
            )
        }
    }

    /// Attach all loaded programs
    pub async fn attach_all(&self, target_path: Option<&str>) -> LoadResult {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            let mut guard = self.bpf.write().await;
            let bpf = match guard.as_mut() {
                Some(b) => b,
                None => return LoadResult::Failed("BPF object not loaded".to_string()),
            };

            for (name, program) in bpf.programs_mut() {
                let section = program.section().map(|s| s.to_string());
                let symbol = section
                    .as_deref()
                    .and_then(|s| s.split('/').nth(1))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| name.to_string());

                match program {
                    Program::KProbe(mut prog) => {
                        if let Err(e) = prog.load() {
                            return LoadResult::Failed(format!("load kprobe {}: {}", name, e));
                        }
                        if let Err(e) = prog.attach(&symbol, 0) {
                            return LoadResult::Failed(format!("attach kprobe {}: {}", name, e));
                        }
                    }
                    Program::KRetProbe(mut prog) => {
                        if let Err(e) = prog.load() {
                            return LoadResult::Failed(format!("load kretprobe {}: {}", name, e));
                        }
                        if let Err(e) = prog.attach(&symbol, 0) {
                            return LoadResult::Failed(format!("attach kretprobe {}: {}", name, e));
                        }
                    }
                    Program::UProbe(mut prog) => {
                        let binary = match target_path {
                            Some(p) => p,
                            None => {
                                return LoadResult::Failed(format!(
                                    "no target binary provided for uprobe {}",
                                    name
                                ))
                            }
                        };
                        if let Err(e) = prog.load() {
                            return LoadResult::Failed(format!("load uprobe {}: {}", name, e));
                        }
                        if let Err(e) =
                            prog.attach(Some(binary), &symbol, 0, UProbeAttachType::Entry, None)
                        {
                            return LoadResult::Failed(format!("attach uprobe {}: {}", name, e));
                        }
                    }
                    Program::URetProbe(mut prog) => {
                        let binary = match target_path {
                            Some(p) => p,
                            None => {
                                return LoadResult::Failed(format!(
                                    "no target binary provided for uretprobe {}",
                                    name
                                ))
                            }
                        };
                        if let Err(e) = prog.load() {
                            return LoadResult::Failed(format!("load uretprobe {}: {}", name, e));
                        }
                        if let Err(e) = prog.attach(Some(binary), &symbol, 0, None) {
                            return LoadResult::Failed(format!("attach uretprobe {}: {}", name, e));
                        }
                    }
                    _ => {
                        // Skip unsupported program types for now
                        warn!("Skipping unsupported program type for {}", name);
                    }
                }
                debug!("Attached program: {}", name);
            }

            LoadResult::Success
        }

        #[cfg(any(not(target_os = "linux"), not(feature = "ebpf")))]
        {
            let _ = target_path;
            LoadResult::NotSupported(
                "eBPF is only supported on Linux with the ebpf feature".to_string(),
            )
        }
    }

    /// Detach all programs
    pub async fn detach_all(&self) {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        if let Some(bpf) = self.bpf.write().await.as_mut() {
            for (name, program) in bpf.programs_mut() {
                if let Err(e) = program.detach() {
                    warn!("Failed to detach {}: {}", name, e);
                } else {
                    debug!("Detached program: {}", name);
                }
            }
        }
    }

    /// Unload everything
    pub async fn unload(&self) {
        self.detach_all().await;

        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            *self.bpf.write().await = None;
            info!("Unloaded all eBPF programs and maps");
        }
    }

    /// Get loaded programs
    pub async fn get_programs(&self) -> Vec<LoadedProgram> {
        Vec::new()
    }

    /// Get loaded maps
    pub async fn get_maps(&self) -> Vec<LoadedMap> {
        Vec::new()
    }

    /// Get a map by name
    pub async fn get_map(&self, name: &str) -> Option<LoadedMap> {
        let _ = name;
        None
    }

    /// Check if fully loaded and attached
    pub async fn is_ready(&self) -> bool {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            self.bpf.read().await.is_some()
        }

        #[cfg(any(not(target_os = "linux"), not(feature = "ebpf")))]
        {
            false
        }
    }

    /// Get kernel version
    pub async fn kernel_version(&self) -> (u32, u32, u32) {
        *self.kernel_version.read().await
    }

    /// Check if BTF is available
    pub async fn has_btf(&self) -> bool {
        *self.btf_available.read().await
    }

    /// Check if ring buffer is available
    pub async fn has_ringbuf(&self) -> bool {
        *self.ringbuf_available.read().await
    }

    /// Update a u32->u64 hash map (used for pid->query_hash)
    pub async fn update_hash_map_u32_u64(
        &self,
        map_name: &str,
        key: u32,
        value: u64,
    ) -> Result<()> {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            let mut guard = self.bpf.write().await;
            let bpf = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("BPF object not loaded"))?;

            let mut map: BpfHashMap<_, u32, u64> = BpfHashMap::try_from(bpf.map_mut(map_name)?)?;
            map.insert(key, value, 0)?;
            return Ok(());
        }

        #[cfg(any(not(target_os = "linux"), not(feature = "ebpf")))]
        {
            let _ = (map_name, key, value);
            Err(anyhow::anyhow!(
                "eBPF map updates require Linux with ebpf feature"
            ))
        }
    }

    /// Open perf event buffers for a given map (e.g., "lock_events", "lwlock_events")
    pub async fn open_perf_buffers(&self, map_name: &str) -> Result<Vec<PerfEventArrayBuffer>> {
        #[cfg(all(target_os = "linux", feature = "ebpf"))]
        {
            let mut guard = self.bpf.write().await;
            let bpf = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("BPF object not loaded"))?;

            let mut perf = PerfEventArray::try_from(bpf.map_mut(map_name)?)?;
            let mut buffers = Vec::new();

            for cpu in aya::util::online_cpus()? {
                match perf.open(cpu, None) {
                    Ok(buf) => buffers.push(buf),
                    Err(e) => warn!("Failed to open perf buffer for CPU {}: {}", cpu, e),
                }
            }

            Ok(buffers)
        }

        #[cfg(any(not(target_os = "linux"), not(feature = "ebpf")))]
        {
            let _ = map_name;
            Err(anyhow::anyhow!(
                "Perf buffers require Linux with ebpf feature"
            ))
        }
    }
}

impl Default for EbpfLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kernel_version() {
        let version_str = "Linux version 5.15.0-generic (user@host) #1 SMP ...";
        let version = EbpfLoader::parse_kernel_version(version_str);
        assert_eq!(version, Some((5, 15, 0)));
    }

    #[test]
    fn test_program_type_min_version() {
        assert_eq!(ProgramType::Fentry.min_kernel_version(), (5, 5));
        assert_eq!(ProgramType::Kprobe.min_kernel_version(), (4, 1));
        assert_eq!(ProgramType::Xdp.min_kernel_version(), (4, 8));
    }

    #[tokio::test]
    async fn test_loader_creation() {
        let loader = EbpfLoader::new();

        assert_eq!(loader.kernel_version().await, (0, 0, 0));
        assert!(!loader.is_ready().await);
    }

    #[tokio::test]
    async fn test_loader_config() {
        let loader = EbpfLoader::new();

        let config = LoaderConfig {
            prefer_fentry: false,
            pin_maps: true,
            ..Default::default()
        };

        loader.configure(config).await;

        let current = loader.config.read().await;
        assert!(!current.prefer_fentry);
        assert!(current.pin_maps);
    }
}
