# eBPF Lock Tracing (Vaanyze)

This doc summarizes the lock tracing pipeline and how it maps to BPF programs and API endpoints.

## Programs and Maps

| Program | Section(s) | Perf Map | Emits |
|---------|------------|----------|-------|
| `bpf/lock_contention.bpf.c` | `uprobe/pg_lock_acquire`, `uprobe/pg_wait_for_lock` | `lock_events` | Blocking/waiting PIDs, query hashes, wait duration |
| `bpf/lwlock_tracer.bpf.c` | `uprobe/ LWLockAcquire`, `uretprobe/LWLockAcquire` | `lwlock_events` | LWLock pointer surrogate, mode, wait duration |
| `bpf/row_lock_tracer.bpf.c` | `uprobe/heap_lock_tuple`, `uretprobe/heap_lock_tuple` | `row_lock_events` | Relation pointer surrogate, lock mode, wait duration |

Build: `make -C bpf` (outputs to `bpf/.output/*.bpf.o`, packaged fallback `/usr/local/lib/db-optimizer/*.bpf.o`).

## Aya Wiring
- Programs registered in `EbpfManager`: `add_default_lock_program()`, `add_lwlock_program()`, `add_row_lock_program()`.
- Perf buffers read in `EbpfManager` consumers and fed into `LockTracer`.
- Timestamps are kernel monotonic; severity for contention events computed in DTO.

## API Endpoints
- `/api/v1/locks/contention` — summarized contention (primary lock tracer).
- `/api/v1/locks/contention/events` — raw contention events.
- `/api/v1/locks/lwlocks/events` — LWLock events.
- `/api/v1/locks/rows/events` — row lock events.
- `/api/v1/stats/metrics` — Prometheus counters: processed/dropped events, perf read errors.

## Known Limitations (to improve)
- LWLock and row-lock tracers currently use pointer surrogates for lock identifiers; extend to decode true IDs (e.g., LWLock tranche, relation OIDs, tuple offsets) from function args/USDT.
- No table/tuple name resolution yet; requires DB metadata lookup.
- Dashboard shows contention events; LWLock/row-lock events are exposed via API but not yet visualized.*** End Patch```​
