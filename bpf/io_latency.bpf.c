// bpf/io_latency.bpf.c - I/O time attribution for query performance analysis
// Tracks disk I/O patterns and correlates with query execution

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

// I/O event tracking
struct io_event {
    __u64 timestamp_ns;
    __u32 pid;
    __u32 tid;
    __u64 query_hash;
    __u64 io_duration_ns;
    __u64 bytes;
    __u32 disk_seeks;
    __u8 io_type;  // 0=read, 1=write, 2=fsync
    __u8 _pad[3];
} __attribute__((packed));

// Per-query I/O statistics
struct io_stats {
    __u64 total_io_time_ns;
    __u64 read_time_ns;
    __u64 write_time_ns;
    __u64 fsync_time_ns;
    __u64 bytes_read;
    __u64 bytes_written;
    __u32 read_count;
    __u32 write_count;
    __u32 fsync_count;
    __u32 disk_seeks;
    __u32 page_faults;
} __attribute__((packed));

// Block I/O tracking
struct bio_start_state {
    __u64 start_ns;
    __u64 sector;
    __u32 size;
} __attribute__((packed));

// Maps
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u64);  // query_hash
    __type(value, struct io_stats);
    __uint(max_entries, 10000);
} query_io_stats SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // PID
    __type(value, __u64);  // current query_hash
    __uint(max_entries, 10000);
} pid_to_query SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u64);  // bio request pointer
    __type(value, struct bio_start_state);
    __uint(max_entries, 10000);
} bio_start_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} io_events SEC(".maps");

// Track block I/O start
SEC("kprobe/blk_account_io_start")
int trace_bio_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    // Check if this PID is executing a query
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    // Get bio request info
    void *bio_ptr = (void *)PT_REGS_PARM1(ctx);
    
    struct bio_start_state state = {};
    state.start_ns = bpf_ktime_get_ns();
    
    // Store bio start time
    __u64 bio_key = (__u64)bio_ptr;
    bpf_map_update_elem(&bio_start_map, &bio_key, &state, BPF_ANY);
    
    return 0;
}

// Track block I/O completion
SEC("kprobe/blk_account_io_done")
int trace_bio_done(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    void *bio_ptr = (void *)PT_REGS_PARM1(ctx);
    __u64 bio_key = (__u64)bio_ptr;
    
    struct bio_start_state *start = bpf_map_lookup_elem(&bio_start_map, &bio_key);
    if (!start) {
        return 0;
    }
    
    __u64 end_ns = bpf_ktime_get_ns();
    __u64 io_duration = end_ns - start->start_ns;
    
    // Update query I/O stats
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (!stats) {
        struct io_stats new_stats = {};
        new_stats.total_io_time_ns = io_duration;
        new_stats.disk_seeks = 1;
        bpf_map_update_elem(&query_io_stats, query_hash_ptr, &new_stats, BPF_NOEXIST);
    } else {
        __sync_fetch_and_add(&stats->total_io_time_ns, io_duration);
        __sync_fetch_and_add(&stats->disk_seeks, 1);
    }
    
    // Cleanup
    bpf_map_delete_elem(&bio_start_map, &bio_key);
    
    return 0;
}

// Track read system calls
SEC("kprobe/vfs_read")
int trace_read_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->read_count, 1);
    }
    
    return 0;
}

SEC("kretprobe/vfs_read")
int trace_read_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    ssize_t bytes = PT_REGS_RC(ctx);
    if (bytes <= 0) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->bytes_read, (__u64)bytes);
    }
    
    return 0;
}

// Track write system calls
SEC("kprobe/vfs_write")
int trace_write_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->write_count, 1);
    }
    
    return 0;
}

SEC("kretprobe/vfs_write")
int trace_write_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    ssize_t bytes = PT_REGS_RC(ctx);
    if (bytes <= 0) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->bytes_written, (__u64)bytes);
    }
    
    return 0;
}

// Track fsync calls (critical for transaction commits)
SEC("kprobe/vfs_fsync")
int trace_fsync_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->fsync_count, 1);
    }
    
    return 0;
}

// Track page faults (indicates cache misses)
SEC("kprobe/handle_mm_fault")
int trace_page_fault(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash_ptr = bpf_map_lookup_elem(&pid_to_query, &pid);
    if (!query_hash_ptr) {
        return 0;
    }
    
    struct io_stats *stats = bpf_map_lookup_elem(&query_io_stats, query_hash_ptr);
    if (stats) {
        __sync_fetch_and_add(&stats->page_faults, 1);
    }
    
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
