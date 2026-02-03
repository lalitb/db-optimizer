// bpf/lwlock_tracer.bpf.c
// LWLock tracer (uprobes/uretprobes)
//
// Emits lwlock_event records when LWLock acquisition completes.
// Uses a per-PID wait map to calculate wait duration. lock_id/mode are
// captured from function arguments (pointer value + mode int).

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>

struct lwlock_event {
    __u64 timestamp_ns;
    __u32 pid;
    __u32 lock_id;      // LWLock identifier (pointer hash)
    __u32 mode;         // 0=shared,1=exclusive
    __u64 wait_duration_ns;
} __attribute__((packed));

struct lwlock_wait_state {
    __u64 start_ns;
    __u32 lock_id;
    __u32 mode;
} __attribute__((packed));

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, __u32);  // pid
    __type(value, struct lwlock_wait_state);
    __uint(max_entries, 10240);
} lwlock_wait_map SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} lwlock_events SEC(".maps");

// uprobe for LWLockAcquire(LWLock *, LWLockMode)
SEC("uprobe/LWLockAcquire")
int trace_lwlock_acquire(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;

    struct lwlock_wait_state state = {};
    state.start_ns = bpf_ktime_get_ns();

    // arg0: LWLock pointer (we store lower 32 bits as id surrogate)
    void *lock_ptr = (void *)PT_REGS_PARM1(ctx);
    state.lock_id = ( (__u64)lock_ptr ) & 0xffffffff;

    // arg1: mode (LWLockMode enum, Shared=0, Exclusive=1)
    state.mode = (__u32)PT_REGS_PARM2(ctx);

    bpf_map_update_elem(&lwlock_wait_map, &pid, &state, BPF_ANY);
    return 0;
}

// uretprobe for LWLockAcquire
SEC("uretprobe/LWLockAcquire")
int trace_lwlock_acquire_ret(struct pt_regs *ctx) {
    __u32 pid = bpf_get_current_pid_tgid() >> 32;

    struct lwlock_wait_state *state = bpf_map_lookup_elem(&lwlock_wait_map, &pid);
    if (!state) {
        return 0;
    }

    __u64 end_ns = bpf_ktime_get_ns();
    __u64 wait_ns = end_ns - state->start_ns;

    struct lwlock_event ev = {};
    ev.timestamp_ns = end_ns;
    ev.pid = pid;
    ev.lock_id = state->lock_id;
    ev.mode = state->mode;
    ev.wait_duration_ns = wait_ns;

    bpf_perf_event_output(ctx, &lwlock_events, BPF_F_CURRENT_CPU, &ev, sizeof(ev));
    bpf_map_delete_elem(&lwlock_wait_map, &pid);
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
