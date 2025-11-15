// Lock contention tracking for PostgreSQL
// Tracks table-level lock acquisition, waits, and releases
// Based on patterns from pg-lock-tracer

#include <vmlinux.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

#define MAX_ENTRIES 10000
#define TASK_COMM_LEN 16

// Lock modes (from PostgreSQL src/include/storage/lockdefs.h)
enum LockMode {
    NO_LOCK = 0,
    ACCESS_SHARE_LOCK = 1,
    ROW_SHARE_LOCK = 2,
    ROW_EXCLUSIVE_LOCK = 3,
    SHARE_UPDATE_EXCLUSIVE_LOCK = 4,
    SHARE_LOCK = 5,
    SHARE_ROW_EXCLUSIVE_LOCK = 6,
    EXCLUSIVE_LOCK = 7,
    ACCESS_EXCLUSIVE_LOCK = 8,
};

// Lock event types
enum LockEventType {
    LOCK_ACQUIRE_START = 1,
    LOCK_ACQUIRE_SUCCESS = 2,
    LOCK_ACQUIRE_FAILED = 3,
    LOCK_RELEASE = 4,
};

// Lock event structure sent to userspace
struct lock_event {
    u64 timestamp_ns;
    u32 pid;
    u32 table_oid;      // PostgreSQL relation OID
    u8 lock_mode;       // LockMode enum
    u8 event_type;      // LockEventType enum
    u8 granted_fast;    // 1 if fastpath, 0 if slowpath
    u64 wait_time_ns;   // Time spent waiting (0 if no wait)
    char comm[TASK_COMM_LEN];
};

// Pending lock requests (for tracking wait times)
struct lock_request_key {
    u32 pid;
    u32 table_oid;
    u8 lock_mode;
};

struct lock_request_info {
    u64 start_time_ns;
    u8 granted_fast;
};

// Maps
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, MAX_ENTRIES);
    __type(key, struct lock_request_key);
    __type(value, struct lock_request_info);
} pending_locks SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 256 * 1024); // 256KB ring buffer
} events SEC(".maps");

// Helper to get current timestamp
static __always_inline u64 get_timestamp_ns(void) {
    return bpf_ktime_get_ns();
}

// Probe: LockAcquire - called when acquiring a lock
// Function signature: bool LockAcquire(LOCKTAG *locktag, LOCKMODE lockmode, ...)
SEC("uprobe/LockAcquire")
int trace_lock_acquire(struct pt_regs *ctx) {
    u64 pid_tgid = bpf_get_current_pid_tgid();
    u32 pid = pid_tgid >> 32;
    u32 tid = (u32)pid_tgid;
    
    // Get LOCKTAG pointer (first argument)
    void *locktag_ptr = (void *)PT_REGS_PARM1(ctx);
    if (!locktag_ptr)
        return 0;
    
    // Read relation OID from LOCKTAG
    // LOCKTAG structure: {u16 locktag_field1, u16 locktag_field2, u32 locktag_field3, ...}
    // For relation locks, locktag_field1 = database OID, locktag_field2 = relation OID
    u32 table_oid = 0;
    bpf_probe_read_user(&table_oid, sizeof(table_oid), locktag_ptr + 4); // Offset to locktag_field2
    
    // Get lock mode (second argument)
    u8 lock_mode = (u8)PT_REGS_PARM2(ctx);
    
    // Record pending lock request
    struct lock_request_key key = {
        .pid = pid,
        .table_oid = table_oid,
        .lock_mode = lock_mode,
    };
    
    struct lock_request_info info = {
        .start_time_ns = get_timestamp_ns(),
        .granted_fast = 0, // Will be updated in return probe
    };
    
    bpf_map_update_elem(&pending_locks, &key, &info, BPF_ANY);
    
    // Send LOCK_ACQUIRE_START event
    struct lock_event *event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
        return 0;
    
    event->timestamp_ns = info.start_time_ns;
    event->pid = pid;
    event->table_oid = table_oid;
    event->lock_mode = lock_mode;
    event->event_type = LOCK_ACQUIRE_START;
    event->granted_fast = 0;
    event->wait_time_ns = 0;
    bpf_get_current_comm(&event->comm, sizeof(event->comm));
    
    bpf_ringbuf_submit(event, 0);
    
    return 0;
}

// Probe: LockAcquire return - called when lock is granted or fails
SEC("uretprobe/LockAcquire")
int trace_lock_acquire_return(struct pt_regs *ctx) {
    u64 pid_tgid = bpf_get_current_pid_tgid();
    u32 pid = pid_tgid >> 32;
    
    // Get return value (1 = success, 0 = failure)
    int ret = (int)PT_REGS_RC(ctx);
    
    u64 now = get_timestamp_ns();
    
    // Note: We don't have the exact lock request details here,
    // so we'll need to iterate pending locks or use a different approach
    // For now, send a generic success/failure event
    
    struct lock_event *event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
        return 0;
    
    event->timestamp_ns = now;
    event->pid = pid;
    event->table_oid = 0; // Unknown in return probe
    event->lock_mode = 0;
    event->event_type = ret ? LOCK_ACQUIRE_SUCCESS : LOCK_ACQUIRE_FAILED;
    event->granted_fast = 0;
    event->wait_time_ns = 0;
    bpf_get_current_comm(&event->comm, sizeof(event->comm));
    
    bpf_ringbuf_submit(event, 0);
    
    return 0;
}

// Probe: UnlockRelationOid - called when releasing a lock
// Function signature: void UnlockRelationOid(Oid relid, LOCKMODE lockmode)
SEC("uprobe/UnlockRelationOid")
int trace_unlock_relation(struct pt_regs *ctx) {
    u64 pid_tgid = bpf_get_current_pid_tgid();
    u32 pid = pid_tgid >> 32;
    
    // Get relation OID (first argument)
    u32 table_oid = (u32)PT_REGS_PARM1(ctx);
    
    // Get lock mode (second argument)
    u8 lock_mode = (u8)PT_REGS_PARM2(ctx);
    
    // Clean up pending lock if exists
    struct lock_request_key key = {
        .pid = pid,
        .table_oid = table_oid,
        .lock_mode = lock_mode,
    };
    bpf_map_delete_elem(&pending_locks, &key);
    
    // Send LOCK_RELEASE event
    struct lock_event *event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event)
        return 0;
    
    event->timestamp_ns = get_timestamp_ns();
    event->pid = pid;
    event->table_oid = table_oid;
    event->lock_mode = lock_mode;
    event->event_type = LOCK_RELEASE;
    event->granted_fast = 0;
    event->wait_time_ns = 0;
    bpf_get_current_comm(&event->comm, sizeof(event->comm));
    
    bpf_ringbuf_submit(event, 0);
    
    return 0;
}

// Probe: LockRelationOid - alternative lock acquisition path
SEC("uprobe/LockRelationOid")
int trace_lock_relation_oid(struct pt_regs *ctx) {
    u64 pid_tgid = bpf_get_current_pid_tgid();
    u32 pid = pid_tgid >> 32;
    
    // Get relation OID (first argument)
    u32 table_oid = (u32)PT_REGS_PARM1(ctx);
    
    // Get lock mode (second argument)
    u8 lock_mode = (u8)PT_REGS_PARM2(ctx);
    
    // Record start time
    struct lock_request_key key = {
        .pid = pid,
        .table_oid = table_oid,
        .lock_mode = lock_mode,
    };
    
    struct lock_request_info info = {
        .start_time_ns = get_timestamp_ns(),
        .granted_fast = 0,
    };
    
    bpf_map_update_elem(&pending_locks, &key, &info, BPF_ANY);
    
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
