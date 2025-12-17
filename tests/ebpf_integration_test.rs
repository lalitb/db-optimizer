// eBPF Integration Tests
//
// These tests verify eBPF functionality on Linux systems.
// They are conditionally compiled and will be skipped on non-Linux platforms.
//
// To run these tests:
//   - Requires Linux kernel 5.8+ with BTF support
//   - Requires CAP_BPF or root privileges
//   - Run with: cargo test --test ebpf_integration_test -- --ignored
//
// For CI/CD, these tests should run in a privileged Linux container.

/// Tests that work on all platforms using mock-friendly components
mod common_tests {
    use pg_query_advisor::ebpf::{
        DegradationManager, LiveMapConfig, LoaderConfig, MetricValue, PrometheusRegistry,
        QueryNormalizer, SessionTracker, VacuumDetector, VacuumType, WalIoCorrelator,
    };

    #[cfg(not(target_os = "linux"))]
    use pg_query_advisor::ebpf::CapabilityLevel;

    /// Test query normalizer with async API
    #[tokio::test]
    async fn test_query_normalizer() {
        let normalizer = QueryNormalizer::new();

        // Test normalization and hashing
        let query = "SELECT * FROM users WHERE id = 123 AND name = 'test'";
        let hash = normalizer.normalize_and_hash(query).await;

        println!("Query hash: {}", hash);
        assert!(hash > 0);

        // Same query should produce same hash
        let hash2 = normalizer.normalize_and_hash(query).await;
        assert_eq!(hash, hash2);

        // Different query should produce different hash
        let different_query = "SELECT * FROM orders WHERE order_id = 456";
        let hash3 = normalizer.normalize_and_hash(different_query).await;
        assert_ne!(hash, hash3);

        let stats = normalizer.get_stats().await;
        println!("Normalizer stats: {:?}", stats);
        assert!(stats.normalized_count >= 2);
    }

    /// Test session tracker with async API
    #[tokio::test]
    async fn test_session_tracker() {
        let tracker = SessionTracker::new();

        // Create a session
        let pid = 12345u32;
        let session = tracker.get_or_create_session(pid).await;
        println!("Created session for PID {}: {:?}", pid, session);
        assert_eq!(session.backend_pid, pid);

        // Record a query
        let query_hash = 0xDEADBEEF;
        tracker.record_simple_query_start(pid, query_hash).await;

        // Check current query
        let current = tracker.get_current_query_hash(pid).await;
        assert_eq!(current, Some(query_hash));

        // End query
        tracker.record_query_end(pid).await;

        // Get stats
        let stats = tracker.get_stats().await;
        println!("Tracker stats: {:?}", stats);
        assert!(stats.active_sessions >= 1);
    }

    /// Test WAL I/O correlator
    #[tokio::test]
    async fn test_wal_correlator() {
        let correlator = WalIoCorrelator::new();

        // Test I/O summary (without adding events, just verify structure)
        let summary = correlator.get_io_summary().await;
        println!("I/O summary: {:?}", summary);

        // Get WAL info
        let wal_info = correlator.get_wal_info().await;
        println!("WAL info: {:?}", wal_info);
    }

    /// Test vacuum detector
    #[tokio::test]
    async fn test_vacuum_detector() {
        let detector = VacuumDetector::new();

        let pid = 12345u32;

        // Start a vacuum
        detector
            .start_vacuum(
                pid,
                VacuumType::Manual,
                "public.users".to_string(),
                "testdb".to_string(),
            )
            .await;

        // Check active vacuums
        let active = detector.get_active_vacuums().await;
        assert!(!active.is_empty());
        println!("Active vacuums: {:?}", active);

        // Complete the vacuum
        detector.complete_vacuum(pid).await;

        // Check stats
        let stats = detector.get_stats().await;
        println!("Vacuum stats: {:?}", stats);
        assert!(stats.total_vacuums >= 1);
    }

    /// Test degradation manager
    #[tokio::test]
    async fn test_degradation_manager() {
        let manager = DegradationManager::new();

        // Get initial level - may be Disabled on non-Linux or Full on Linux
        let level = manager.current_level().await;
        println!("Initial level: {:?}", level);

        // On non-Linux, eBPF is disabled, so level should be Disabled
        // On Linux without CAP_BPF, level should also be Disabled
        // On Linux with CAP_BPF, level should be Full
        #[cfg(not(target_os = "linux"))]
        assert_eq!(level, CapabilityLevel::Disabled);

        // Check if degraded - on non-Linux this is effectively always "degraded"
        let is_degraded = manager.is_degraded().await;
        println!("Is degraded: {}", is_degraded);

        // Get full status
        let status = manager.get_status().await;
        println!("Status: {:?}", status);
    }

    /// Test Prometheus registry
    #[tokio::test]
    async fn test_prometheus_registry() {
        let registry = PrometheusRegistry::new("db_optimizer");

        // Add metrics using helper constructors
        let metric = MetricValue::gauge("test_gauge", "A test gauge", 42.5);
        registry.add_metric(metric).await;

        let counter = MetricValue::counter("test_counter", "A test counter", 100.0);
        registry.add_metric(counter).await;

        // Export
        let output = registry.export().await;
        println!("Prometheus output:\n{}", output);

        assert!(output.contains("test_gauge"));
        assert!(output.contains("test_counter"));
    }

    /// Test configuration structures
    #[test]
    fn test_live_map_config() {
        let config = LiveMapConfig::default();

        println!("Live map config: {:?}", config);
        assert!(config.enabled);
    }

    /// Test loader config
    #[test]
    fn test_loader_config() {
        let config = LoaderConfig::default();

        println!("Loader config: {:?}", config);
    }

    /// Test quick hash function
    #[test]
    fn test_quick_hash() {
        use pg_query_advisor::ebpf::query_normalizer::quick_hash;

        let hash1 = quick_hash("SELECT * FROM users");
        let hash2 = quick_hash("SELECT * FROM users");
        let hash3 = quick_hash("SELECT * FROM orders");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}

/// Linux-specific tests that require eBPF
#[cfg(target_os = "linux")]
mod linux_tests {
    use pg_query_advisor::ebpf::{CgroupCollector, EbpfConfig, EbpfLoader, EbpfManager, EbpfMode};

    /// Test that eBPF manager initializes correctly
    #[tokio::test]
    #[ignore = "Requires Linux with eBPF support and CAP_BPF"]
    async fn test_ebpf_manager_initialization() {
        let config = EbpfConfig::default();
        let manager = EbpfManager::new(config);
        let stats = manager.get_stats().await;
        println!("eBPF Manager initialized successfully");
        println!("Mode: {:?}", stats.mode);
        assert!(matches!(
            stats.mode,
            EbpfMode::Privileged | EbpfMode::Mock | EbpfMode::Disabled
        ));
    }

    /// Test container cgroup detection
    #[tokio::test]
    #[ignore = "Requires Linux with cgroup support"]
    async fn test_container_detection() {
        let collector = CgroupCollector::new();

        // Detect runtime
        let runtime = collector.detect_runtime();
        println!("Detected runtime: {:?}", runtime);

        // Try to get self metrics
        if let Some(metrics) = collector.collect_self_metrics() {
            println!("Self metrics:");
            println!("  CPU usage: {:.2}%", metrics.cpu_usage_percent);
            println!("  Memory: {} MB", metrics.memory_usage_bytes / 1024 / 1024);
        }
    }

    /// Test eBPF program loader
    #[tokio::test]
    #[ignore = "Requires Linux with eBPF support and CAP_BPF"]
    async fn test_program_loader() {
        let loader = EbpfLoader::new();

        // Check BTF availability
        let btf_available = loader.btf_available();
        println!("BTF available: {}", btf_available);

        // List available programs
        let programs = loader.list_available_programs();
        println!("Available programs: {:?}", programs);
    }
}

/// Performance benchmark tests
mod benchmarks {
    use pg_query_advisor::ebpf::{QueryNormalizer, SessionTracker};
    use std::time::Instant;

    #[tokio::test]
    #[ignore = "Run explicitly for benchmarking"]
    async fn bench_query_normalization() {
        let normalizer = QueryNormalizer::new();
        let iterations = 10_000;

        // Warm up
        for _ in 0..100 {
            let _ = normalizer
                .normalize_and_hash("SELECT * FROM users WHERE id = 1")
                .await;
        }

        let start = Instant::now();
        for i in 0..iterations {
            let query = format!(
                "SELECT * FROM users WHERE id = {} AND name = 'user{}'",
                i, i
            );
            let _ = normalizer.normalize_and_hash(&query).await;
        }
        let elapsed = start.elapsed();

        let stats = normalizer.get_stats().await;

        println!("Query normalization benchmark:");
        println!("  Iterations: {}", iterations);
        println!("  Total time: {:?}", elapsed);
        println!(
            "  Per query: {:.2} µs",
            elapsed.as_micros() as f64 / iterations as f64
        );
        println!("  Cache hits: {}", stats.cache_hits);
        println!("  Cache misses: {}", stats.cache_misses);
    }

    #[tokio::test]
    #[ignore = "Run explicitly for benchmarking"]
    async fn bench_session_tracking() {
        let tracker = SessionTracker::new();
        let iterations = 10_000;

        let start = Instant::now();
        for i in 0..iterations {
            let pid = (i % 1000) as u32 + 1000;
            let _ = tracker.get_or_create_session(pid).await;
            tracker.record_simple_query_start(pid, i as u64).await;
            tracker.record_query_end(pid).await;
        }
        let elapsed = start.elapsed();

        let stats = tracker.get_stats().await;

        println!("Session tracking benchmark:");
        println!("  Iterations: {}", iterations);
        println!("  Total time: {:?}", elapsed);
        println!(
            "  Per operation: {:.2} µs",
            elapsed.as_micros() as f64 / (iterations * 3) as f64
        );
        println!("  Active sessions: {}", stats.active_sessions);
    }
}
