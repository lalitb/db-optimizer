// bpf/postgres.bpf.c - PostgreSQL-specific uprobes

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

// Event types specific to PostgreSQL
enum pg_event_type {
    PG_QUERY_START = 100,
    PG_QUERY_END = 101,
    PG_PARSE_START = 102,
    PG_BIND = 103,
    PG_EXECUTE = 104,
};

// Session state tracking
struct session_state {
    __u64 current_query_hash;
    __u64 prepared_stmt_hash;
    __u64 query_start_ns;
    __u32 backend_pid;
    __u8  is_prepared;
    __u8  _pad[3];
} __attribute__((packed));

// PostgreSQL event
struct pg_event {
    __u64 timestamp_ns;
    __u32 pid;
    __u8  type;
    __u8  _pad[3];
    __u64 query_hash;
    __u64 value;
} __attribute__((packed));

// Maps
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // PID
    __type(value, struct session_state);
    __uint(max_entries, 1000);
} session_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} pg_events SEC(".maps");

// Simple hash function for query text
static __always_inline __u64 hash_string(const char *str, int max_len) {
    __u64 hash = 5381;
    char c;
    
    #pragma unroll
    for (int i = 0; i < max_len && i < 256; i++) {
        if (bpf_probe_read_user(&c, 1, str + i) != 0)
            break;
        if (c == 0)
            break;
        hash = ((hash << 5) + hash) + c;
    }
    
    return hash;
}

// Hook simple query execution
SEC("uprobe/postgres:exec_simple_query")
int trace_simple_query(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct session_state state = {};
    state.backend_pid = pid;
    state.is_prepared = 0;
    state.query_start_ns = bpf_ktime_get_ns();
    
    // Get query text pointer and hash it
    char *query_string = (char *)PT_REGS_PARM1(ctx);
    state.current_query_hash = hash_string(query_string, 256);
    
    // Update session state
    bpf_map_update_elem(&session_map, &pid, &state, BPF_ANY);
    
    // Emit query start event
    struct pg_event e = {};
    e.timestamp_ns = state.query_start_ns;
    e.pid = pid;
    e.type = PG_QUERY_START;
    e.query_hash = state.current_query_hash;
    
    bpf_perf_event_output(ctx, &pg_events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    
    return 0;
}

// Hook query completion
SEC("uretprobe/postgres:exec_simple_query")
int trace_simple_query_return(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct session_state *state = bpf_map_lookup_elem(&session_map, &pid);
    if (!state) return 0;
    
    __u64 end_ns = bpf_ktime_get_ns();
    __u64 duration = end_ns - state->query_start_ns;
    
    // Emit query end event
    struct pg_event e = {};
    e.timestamp_ns = end_ns;
    e.pid = pid;
    e.type = PG_QUERY_END;
    e.query_hash = state->current_query_hash;
    e.value = duration;
    
    bpf_perf_event_output(ctx, &pg_events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    
    return 0;
}

// Hook prepared statement execution
SEC("uprobe/postgres:exec_execute_message")
int trace_prepared_exec(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    struct session_state state = {};
    state.backend_pid = pid;
    state.is_prepared = 1;
    state.query_start_ns = bpf_ktime_get_ns();
    
    // Portal name for prepared statements
    char *portal_name = (char *)PT_REGS_PARM1(ctx);
    long max_rows = PT_REGS_PARM2(ctx);
    
    // Hash portal name with max_rows for uniqueness
    state.prepared_stmt_hash = hash_string(portal_name, 64);
    state.prepared_stmt_hash ^= (__u64)max_rows;
    
    bpf_map_update_elem(&session_map, &pid, &state, BPF_ANY);
    
    // Emit execute event
    struct pg_event e = {};
    e.timestamp_ns = state.query_start_ns;
    e.pid = pid;
    e.type = PG_EXECUTE;
    e.query_hash = state.prepared_stmt_hash;
    
    bpf_perf_event_output(ctx, &pg_events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    
    return 0;
}

// Hook Parse message (prepared statement definition)
SEC("uprobe/postgres:exec_parse_message")
int trace_parse_message(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    // Get statement name and query text
    char *stmt_name = (char *)PT_REGS_PARM1(ctx);
    char *query_string = (char *)PT_REGS_PARM2(ctx);
    
    __u64 query_hash = hash_string(query_string, 256);
    
    // Emit parse event
    struct pg_event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.type = PG_PARSE_START;
    e.query_hash = query_hash;
    
    bpf_perf_event_output(ctx, &pg_events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    
    return 0;
}

// Hook Bind message (parameter binding)
SEC("uprobe/postgres:exec_bind_message")
int trace_bind_message(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    char *portal_name = (char *)PT_REGS_PARM1(ctx);
    char *stmt_name = (char *)PT_REGS_PARM2(ctx);
    
    __u64 portal_hash = hash_string(portal_name, 64);
    __u64 stmt_hash = hash_string(stmt_name, 64);
    
    // Emit bind event
    struct pg_event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.type = PG_BIND;
    e.query_hash = portal_hash;
    e.value = stmt_hash;
    
    bpf_perf_event_output(ctx, &pg_events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
