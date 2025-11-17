// bpf/lock_contention.bpf.c - Advanced lock contention tracking
// Identifies blocking queries and quantifies wait impact

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

// Lock contention event
struct lock_contention_event {
    __u64 timestamp_ns;
    __u32 blocking_pid;
    __u32 waiting_pid;
    __u64 blocking_query_hash;
    __u64 waiting_query_hash;
    __u64 wait_duration_ns;
    __u8 lock_type;  // 0=row, 1=table, 2=advisory
    __u8 lock_mode;  // 0=shared, 1=exclusive
    __u8 _pad[2];
} __attribute__((packed));

// Blocking query statistics
struct blocking_stats {
    __u64 blocking_query_hash;
    __u32 num_awaiting_queries;
    __u64 awaiting_duration_total_ms;
    __u64 futex_waits;
    __u32 blocking_count;
} __attribute__((packed));

// Lock wait state
struct lock_wait_state {
    __u64 wait_start_ns;
    __u64 query_hash;
    __u32 blocking_pid;
    __u8 lock_type;
    __u8 lock_mode;
} __attribute__((packed));

// Maps
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // waiting PID
    __type(value, struct lock_wait_state);
    __uint(max_entries, 10000);
} lock_wait_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u64);  // blocking query_hash
    __type(value, struct blocking_stats);
    __uint(max_entries, 10000);
} blocking_query_stats SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // PID
    __type(value, __u64);  // query_hash
    __uint(max_entries, 10000);
} pid_query_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} lock_events SEC(".maps");

// Track lock acquisition attempts
SEC("kprobe/pg_lock_acquire")
int trace_lock_acquire_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    // Get current query hash for this PID
    __u64 *query_hash = bpf_map_lookup_elem(&pid_query_map, &pid);
    if (!query_hash) {
        return 0;
    }
    
    // Record lock wait start
    struct lock_wait_state state = {};
    state.wait_start_ns = bpf_ktime_get_ns();
    state.query_hash = *query_hash;
    
    bpf_map_update_elem(&lock_wait_map, &pid, &state, BPF_ANY);
    
    return 0;
}

// Track successful lock acquisition
SEC("kretprobe/pg_lock_acquire")
int trace_lock_acquire_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct lock_wait_state *state = bpf_map_lookup_elem(&lock_wait_map, &pid);
    if (!state) {
        return 0;
    }
    
    // Check if lock was acquired successfully
    int ret = PT_REGS_RC(ctx);
    if (ret == 0) {
        // Lock acquired - calculate wait time
        __u64 end_ns = bpf_ktime_get_ns();
        __u64 wait_duration = end_ns - state->wait_start_ns;
        
        // If significant wait time, this indicates contention
        if (wait_duration > 1000000) {  // > 1ms
            // Emit lock contention event
            struct lock_contention_event event = {};
            event.timestamp_ns = end_ns;
            event.waiting_pid = pid;
            event.waiting_query_hash = state->query_hash;
            event.wait_duration_ns = wait_duration;
            event.blocking_pid = state->blocking_pid;
            event.lock_type = state->lock_type;
            event.lock_mode = state->lock_mode;
            
            bpf_perf_event_output(ctx, &lock_events, BPF_F_CURRENT_CPU, &event, sizeof(event));
        }
    }
    
    // Cleanup
    bpf_map_delete_elem(&lock_wait_map, &pid);
    
    return 0;
}

// Track futex waits (kernel-level locking)
SEC("kprobe/futex_wait")
int trace_futex_wait(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    __u64 *query_hash = bpf_map_lookup_elem(&pid_query_map, &pid);
    if (!query_hash) {
        return 0;
    }
    
    // Record futex wait for statistics
    struct lock_wait_state state = {};
    state.wait_start_ns = bpf_ktime_get_ns();
    state.query_hash = *query_hash;
    
    bpf_map_update_elem(&lock_wait_map, &pid, &state, BPF_ANY);
    
    return 0;
}

SEC("kretprobe/futex_wait")
int trace_futex_wait_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct lock_wait_state *state = bpf_map_lookup_elem(&lock_wait_map, &pid);
    if (!state) {
        return 0;
    }
    
    __u64 end_ns = bpf_ktime_get_ns();
    __u64 wait_duration = end_ns - state->wait_start_ns;
    
    // If this was a long futex wait, it indicates contention
    if (wait_duration > 100000) {  // > 0.1ms
        // Update statistics (this helps identify CPU contention vs lock contention)
        struct blocking_stats *stats = bpf_map_lookup_elem(&blocking_query_stats, &state->query_hash);
        if (stats) {
            __sync_fetch_and_add(&stats->futex_waits, 1);
        }
    }
    
    bpf_map_delete_elem(&lock_wait_map, &pid);
    
    return 0;
}

// Track which query is holding a lock when another waits
SEC("kprobe/pg_wait_for_lock")
int trace_wait_for_lock(struct pt_regs *ctx) {
    __u32 waiting_pid = bpf_get_current_pid_tgid() >> 32;
    
    // Get waiting query info
    __u64 *waiting_query = bpf_map_lookup_elem(&pid_query_map, &waiting_pid);
    if (!waiting_query) {
        return 0;
    }
    
    // Get blocking PID from function parameters
    __u32 blocking_pid = (__u32)PT_REGS_PARM1(ctx);
    
    // Get blocking query info
    __u64 *blocking_query = bpf_map_lookup_elem(&pid_query_map, &blocking_pid);
    if (!blocking_query) {
        return 0;
    }
    
    // Record wait state with blocker info
    struct lock_wait_state state = {};
    state.wait_start_ns = bpf_ktime_get_ns();
    state.query_hash = *waiting_query;
    state.blocking_pid = blocking_pid;
    
    bpf_map_update_elem(&lock_wait_map, &waiting_pid, &state, BPF_ANY);
    
    // Update blocking query statistics
    struct blocking_stats *stats = bpf_map_lookup_elem(&blocking_query_stats, blocking_query);
    if (!stats) {
        struct blocking_stats new_stats = {};
        new_stats.blocking_query_hash = *blocking_query;
        new_stats.num_awaiting_queries = 1;
        new_stats.blocking_count = 1;
        bpf_map_update_elem(&blocking_query_stats, blocking_query, &new_stats, BPF_NOEXIST);
    } else {
        __sync_fetch_and_add(&stats->num_awaiting_queries, 1);
        __sync_fetch_and_add(&stats->blocking_count, 1);
    }
    
    return 0;
}

SEC("kretprobe/pg_wait_for_lock")
int trace_wait_for_lock_return(struct pt_regs *ctx) {
    __u32 waiting_pid = bpf_get_current_pid_tgid() >> 32;
    
    struct lock_wait_state *state = bpf_map_lookup_elem(&lock_wait_map, &waiting_pid);
    if (!state) {
        return 0;
    }
    
    __u64 end_ns = bpf_ktime_get_ns();
    __u64 wait_duration = end_ns - state->wait_start_ns;
    __u64 wait_duration_ms = wait_duration / 1000000;
    
    // Get blocking query hash
    __u64 *blocking_query = bpf_map_lookup_elem(&pid_query_map, &state->blocking_pid);
    if (blocking_query) {
        // Emit detailed lock contention event
        struct lock_contention_event event = {};
        event.timestamp_ns = end_ns;
        event.waiting_pid = waiting_pid;
        event.blocking_pid = state->blocking_pid;
        event.waiting_query_hash = state->query_hash;
        event.blocking_query_hash = *blocking_query;
        event.wait_duration_ns = wait_duration;
        event.lock_type = state->lock_type;
        event.lock_mode = state->lock_mode;
        
        bpf_perf_event_output(ctx, &lock_events, BPF_F_CURRENT_CPU, &event, sizeof(event));
        
        // Update blocking statistics
        struct blocking_stats *stats = bpf_map_lookup_elem(&blocking_query_stats, blocking_query);
        if (stats) {
            __sync_fetch_and_add(&stats->awaiting_duration_total_ms, wait_duration_ms);
            if (stats->num_awaiting_queries > 0) {
                __sync_fetch_and_add(&stats->num_awaiting_queries, -1);
            }
        }
    }
    
    bpf_map_delete_elem(&lock_wait_map, &waiting_pid);
    
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
