// bpf/events.bpf.c - Lightweight kernel event emission

#include <linux/bpf.h>
#include <linux/ptrace.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

// Event types
enum event_type {
    EVENT_READ = 1,
    EVENT_WRITE = 2,
    EVENT_SCHED_SWITCH = 3,
    EVENT_PAGE_FAULT = 4,
    EVENT_QUERY_START = 5,
    EVENT_QUERY_END = 6,
    EVENT_FSYNC = 7,
    EVENT_LOCK_WAIT = 8,
};

// Event structure - minimal for kernel safety
struct event {
    __u64 timestamp_ns;
    __u32 pid;
    __u32 tid;
    __u8  type;
    __u8  _pad[3];
    __u64 value1;   // e.g., read bytes or duration
    __u64 value2;   // auxiliary data
} __attribute__((packed));

// Maps for perf event output
struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} events SEC(".maps");

// Map to track PIDs we're monitoring
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);
    __type(value, __u8);
    __uint(max_entries, 2048);
} tracked_pids SEC(".maps");

// Map for storing query start times
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);
    __type(value, __u64);
    __uint(max_entries, 1024);
} query_start_times SEC(".maps");

// Sampling configuration
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __type(key, __u32);
    __type(value, __u32);
    __uint(max_entries, 4);
} sampling_config SEC(".maps");

// Helper to check if PID should be tracked
static __always_inline int should_track(__u32 pid) {
    __u8 *tracked = bpf_map_lookup_elem(&tracked_pids, &pid);
    return tracked && *tracked;
}

// Helper to check sampling rate
static __always_inline int should_sample(__u32 type) {
    __u32 key = type;
    __u32 *rate = bpf_map_lookup_elem(&sampling_config, &key);
    if (!rate || *rate == 0) return 1;  // No sampling
    
    // Simple sampling based on timestamp
    __u64 ts = bpf_ktime_get_ns();
    return (ts % *rate) == 0;
}

// Trace VFS read operations
SEC("kprobe/vfs_read")
int trace_vfs_read(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    if (!should_track(pid)) return 0;
    if (!should_sample(EVENT_READ)) return 0;
    
    struct event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.tid = bpf_get_current_pid_tgid() & 0xFFFFFFFF;
    e.type = EVENT_READ;
    e.value1 = PT_REGS_PARM3(ctx);  // Count/size parameter
    
    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// Trace VFS write operations
SEC("kprobe/vfs_write")
int trace_vfs_write(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    if (!should_track(pid)) return 0;
    if (!should_sample(EVENT_WRITE)) return 0;
    
    struct event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.tid = bpf_get_current_pid_tgid() & 0xFFFFFFFF;
    e.type = EVENT_WRITE;
    e.value1 = PT_REGS_PARM3(ctx);  // Count/size parameter
    
    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// Trace scheduler context switches
SEC("tp_btf/sched_switch")
int trace_sched_switch(u64 *ctx) {
    struct task_struct *prev = (struct task_struct *)ctx[1];
    struct task_struct *next = (struct task_struct *)ctx[2];
    
    __u32 prev_pid = BPF_CORE_READ(prev, tgid);
    
    if (!should_track(prev_pid)) return 0;
    if (!should_sample(EVENT_SCHED_SWITCH)) return 0;
    
    struct event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = prev_pid;
    e.tid = BPF_CORE_READ(prev, pid);
    e.type = EVENT_SCHED_SWITCH;
    e.value1 = BPF_CORE_READ(prev, __state);  // Task state
    
    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// Trace page faults
SEC("tp_btf/page_fault_user")
int trace_page_fault(u64 *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    if (!should_track(pid)) return 0;
    if (!should_sample(EVENT_PAGE_FAULT)) return 0;
    
    struct event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.tid = bpf_get_current_pid_tgid() & 0xFFFFFFFF;
    e.type = EVENT_PAGE_FAULT;
    e.value1 = ctx[0];  // Fault address
    e.value2 = ctx[1];  // Error code
    
    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// Trace fsync operations
SEC("kprobe/do_fsync")
int trace_fsync(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;
    
    if (!should_track(pid)) return 0;
    
    struct event e = {};
    e.timestamp_ns = bpf_ktime_get_ns();
    e.pid = pid;
    e.tid = bpf_get_current_pid_tgid() & 0xFFFFFFFF;
    e.type = EVENT_FSYNC;
    
    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
