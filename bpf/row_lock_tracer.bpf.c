// bpf/row_lock_tracer.bpf.c
// Row lock tracer (uprobes on heap_lock_tuple)
//
// Emits row_lock_event records with a per-PID wait map to calculate wait
// duration. lock identifiers are placeholders (tuple_offset/relfilenode)
// derived from function arguments when available.

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

struct row_lock_event {
    __u64 timestamp_ns;
    __u32 pid;
    __u64 relfilenode;    // placeholder (use relation pointer as surrogate)
    __u64 tuple_offset;   // placeholder (offset/ctid hash)
    __u32 lock_mode;      // e.g., 0=shared,1=exclusive
    __u64 wait_duration_ns;
} __attribute__((packed));

struct row_lock_wait_state {
    __u64 start_ns;
    __u64 relfilenode;
    __u64 tuple_offset;
    __u32 lock_mode;
} __attribute__((packed));

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32); // pid
    __type(value, struct row_lock_wait_state);
    __uint(max_entries, 10240);
} row_lock_wait_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} row_lock_events SEC(".maps");

// uprobe for heap_lock_tuple(Relation, HeapTuple, Buffer, ...)
SEC("uprobe/heap_lock_tuple")
int trace_heap_lock_tuple(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;

    struct row_lock_wait_state state = {};
    state.start_ns = bpf_ktime_get_ns();

    void *relation = (void *)PT_REGS_PARM1(ctx);
    state.relfilenode = ((__u64)relation) & 0xffffffff; // surrogate

    // lock mode often arg4 (LockTupleMode/int)
    state.lock_mode = (__u32)PT_REGS_PARM4(ctx);

    bpf_map_update_elem(&row_lock_wait_map, &pid, &state, BPF_ANY);
    return 0;
}

SEC("uretprobe/heap_lock_tuple")
int trace_heap_lock_tuple_ret(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;

    struct row_lock_wait_state *state = bpf_map_lookup_elem(&row_lock_wait_map, &pid);
    if (!state) {
        return 0;
    }

    __u64 end_ns = bpf_ktime_get_ns();
    __u64 wait_ns = end_ns - state->start_ns;

    struct row_lock_event ev = {};
    ev.timestamp_ns = end_ns;
    ev.pid = pid;
    ev.relfilenode = state->relfilenode;
    ev.tuple_offset = state->tuple_offset;
    ev.lock_mode = state->lock_mode;
    ev.wait_duration_ns = wait_ns;

    bpf_perf_event_output(ctx, &row_lock_events, BPF_F_CURRENT_CPU, &ev, sizeof(ev));
    bpf_map_delete_elem(&row_lock_wait_map, &pid);
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
