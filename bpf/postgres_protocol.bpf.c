// bpf/postgres_protocol.bpf.c - Protocol-level Postgres query tracing
// Inspired by Coroot's production-ready eBPF tracing

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

// Query event with comprehensive metrics
struct postgres_query_event {
    __u64 timestamp_ns;
    __u32 pid;
    __u32 tid;
    __u64 query_hash;
    __u64 duration_ns;
    __u64 network_latency_ns;
    __u64 io_wait_ns;
    __u64 cpu_time_ns;
    __u32 bytes_sent;
    __u32 bytes_received;
    __u8 status;  // 0=success, 1=error
    __u8 _pad[3];
} __attribute__((packed));

// Query tracking state
struct query_state {
    __u64 start_ns;
    __u64 socket_write_ns;
    __u64 socket_read_ns;
    __u64 io_start_ns;
    __u64 total_io_ns;
    __u64 query_hash;
    __u32 bytes_sent;
    __u32 bytes_received;
    __u8 in_io;
    __u8 _pad[3];
} __attribute__((packed));

// Maps
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // PID
    __type(value, struct query_state);
    __uint(max_entries, 10000);
} query_state_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} postgres_events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u64);  // query_hash
    __type(value, __u64);  // counter
    __uint(max_entries, 10000);
} query_counter SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u64);  // query_hash
    __type(value, __u64);  // total duration
    __uint(max_entries, 10000);
} query_duration_total SEC(".maps");

// Hash function for query text (FNV-1a)
static __always_inline __u64 hash_query(const char *str, int max_len) {
    __u64 hash = 14695981039346656037ULL;
    char c;
    
    #pragma unroll
    for (int i = 0; i < max_len && i < 512; i++) {
        if (bpf_probe_read_user(&c, 1, str + i) != 0)
            break;
        if (c == 0)
            break;
        hash ^= (__u64)c;
        hash *= 1099511628211ULL;
    }
    
    return hash;
}

// Track socket writes (query sent to Postgres)
SEC("kprobe/tcp_sendmsg")
int trace_tcp_sendmsg(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->start_ns > 0 && state->socket_write_ns == 0) {
        state->socket_write_ns = bpf_ktime_get_ns();
        
        // Track bytes sent
        size_t size = PT_REGS_PARM3(ctx);
        state->bytes_sent += (__u32)size;
    }
    
    return 0;
}

// Track socket reads (response from Postgres)
SEC("kprobe/tcp_recvmsg")
int trace_tcp_recvmsg(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->start_ns > 0) {
        if (state->socket_read_ns == 0) {
            state->socket_read_ns = bpf_ktime_get_ns();
        }
        
        // Track bytes received
        size_t size = PT_REGS_PARM3(ctx);
        state->bytes_received += (__u32)size;
    }
    
    return 0;
}

// Track I/O operations (disk access)
SEC("kprobe/vfs_read")
int trace_vfs_read(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->start_ns > 0 && !state->in_io) {
        state->io_start_ns = bpf_ktime_get_ns();
        state->in_io = 1;
    }
    
    return 0;
}

SEC("kretprobe/vfs_read")
int trace_vfs_read_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->in_io) {
        __u64 end_ns = bpf_ktime_get_ns();
        __u64 io_duration = end_ns - state->io_start_ns;
        state->total_io_ns += io_duration;
        state->in_io = 0;
    }
    
    return 0;
}

SEC("kprobe/vfs_write")
int trace_vfs_write(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->start_ns > 0 && !state->in_io) {
        state->io_start_ns = bpf_ktime_get_ns();
        state->in_io = 1;
    }
    
    return 0;
}

SEC("kretprobe/vfs_write")
int trace_vfs_write_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (state && state->in_io) {
        __u64 end_ns = bpf_ktime_get_ns();
        __u64 io_duration = end_ns - state->io_start_ns;
        state->total_io_ns += io_duration;
        state->in_io = 0;
    }
    
    return 0;
}

// Query execution start
SEC("uprobe/postgres:exec_simple_query")
int trace_query_start(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    char *query_string = (char *)PT_REGS_PARM1(ctx);
    __u64 query_hash = hash_query(query_string, 512);
    
    struct query_state state = {};
    state.start_ns = bpf_ktime_get_ns();
    state.query_hash = query_hash;
    
    bpf_map_update_elem(&query_state_map, &pid, &state, BPF_ANY);
    
    // Update query counter
    __u64 *count = bpf_map_lookup_elem(&query_counter, &query_hash);
    if (count) {
        __sync_fetch_and_add(count, 1);
    } else {
        __u64 one = 1;
        bpf_map_update_elem(&query_counter, &query_hash, &one, BPF_NOEXIST);
    }
    
    return 0;
}

// Query execution end
SEC("uretprobe/postgres:exec_simple_query")
int trace_query_end(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    __u32 tid = bpf_get_current_pid_tgid();
    
    struct query_state *state = bpf_map_lookup_elem(&query_state_map, &pid);
    if (!state || state->start_ns == 0) {
        return 0;
    }
    
    __u64 end_ns = bpf_ktime_get_ns();
    __u64 total_duration = end_ns - state->start_ns;
    
    // Calculate network latency (if we captured socket operations)
    __u64 network_latency = 0;
    if (state->socket_write_ns > 0 && state->socket_read_ns > state->socket_write_ns) {
        network_latency = state->socket_read_ns - state->socket_write_ns;
    }
    
    // CPU time = total - I/O - network
    __u64 cpu_time = total_duration;
    if (cpu_time > state->total_io_ns) {
        cpu_time -= state->total_io_ns;
    }
    if (cpu_time > network_latency) {
        cpu_time -= network_latency;
    }
    
    // Get return status
    int ret = PT_REGS_RC(ctx);
    __u8 status = (ret == 0) ? 0 : 1;
    
    // Emit comprehensive query event
    struct postgres_query_event event = {};
    event.timestamp_ns = end_ns;
    event.pid = pid;
    event.tid = tid;
    event.query_hash = state->query_hash;
    event.duration_ns = total_duration;
    event.network_latency_ns = network_latency;
    event.io_wait_ns = state->total_io_ns;
    event.cpu_time_ns = cpu_time;
    event.bytes_sent = state->bytes_sent;
    event.bytes_received = state->bytes_received;
    event.status = status;
    
    bpf_perf_event_output(ctx, &postgres_events, BPF_F_CURRENT_CPU, &event, sizeof(event));
    
    // Update duration histogram
    __u64 *total = bpf_map_lookup_elem(&query_duration_total, &state->query_hash);
    if (total) {
        __sync_fetch_and_add(total, total_duration);
    } else {
        bpf_map_update_elem(&query_duration_total, &state->query_hash, &total_duration, BPF_NOEXIST);
    }
    
    // Clean up state
    bpf_map_delete_elem(&query_state_map, &pid);
    
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
