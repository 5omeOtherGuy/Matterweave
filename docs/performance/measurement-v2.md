# Frame capture schema v2

Opt-in developer frame capture produced by `apps/explorer/src/metrics.rs`. This
document defines exactly what each column means, in what unit, when it is missing
and what it does **not** measure. It describes the instrumentation only; it is not
a performance claim and contains no measured results.

Implemented in this slice: typed rows with exact integer identities, main-thread
CPU busy time, stage wall times, explicit API wait boundaries and a first set of
work counters. Deliberately still missing: streaming-job, collision-publication
and edit-event counters (see [Not measured](#not-measured-in-this-slice)).

Schema v2 has not been shipped or accepted, so its column set was finalised in
place rather than versioned again: this document is the current v2 contract and
`SCHEMA_VERSION` stays 2.

## Requesting a capture

Unchanged from v1: write an integer frame count (`1..=240000`) into
`profile-frames.txt` beside the world save. The request is consumed only after a
fresh output file is created; a malformed or out-of-range request is retained and
no capture starts. Normal gameplay creates no capture and writes no per-frame data.

Output file: `frame-profile-v2-<unix-millis>.csv`, created with `create_new`.
Captures written by earlier builds (`frame-profile-<unix-millis>.csv`) use the v1
column set and are never read, renamed or overwritten by this code.

## File format

```text
#matterweave-frame-capture schema_version=2
draw_attempt_id,renderer_epoch,...,shadow_caster_meshes
<one row per instrumented draw attempt>
```

- Line 1 declares the schema; consumers must reject a file whose declared version
  they do not implement.
- Line 2 is the column header, exactly `metrics::COLUMNS` in order.
- Every row has one cell per column. An empty cell means the value was
  unavailable, unsupported or rejected as invalid. Empty is never zero.
- Durations are milliseconds with four decimals. Negative, infinite and NaN
  durations are rejected and written as empty cells.
- Identities are decimal integers written from `u64`/`u32`. They are never
  converted to floating point, so values above 2^53 stay exact.
- Rows are appended in draw order; the capture stops after the requested count.

## Identities

| Column | Meaning |
| --- | --- |
| `draw_attempt_id` | Unique, monotonically increasing per instrumented draw attempt within one process run. Starts at 1. Increments for retries and out-of-memory attempts. |
| `renderer_epoch` | Increments on every renderer creation/recreation (`resumed`). All per-renderer counters below reset with it. |
| `presented_count` | Count of draw attempts that returned from a successful `vkQueuePresentKHR` call, at the end of this attempt. |
| `result` | `presented`, `retry` or `out_of_memory`. |
| `submitted_gpu_frame_id` | Identity of the GPU submission made by *this* attempt, from the renderer's submission counter. Empty when the attempt submitted nothing, which includes retries that failed before submission but **not** a retry caused by an out-of-date present. |
| `completed_gpu_frame_id` | Identity of a previously completed GPU submission whose timestamps were read during this attempt. Empty when no new completion was observed. |
| `completed_gpu_renderer_epoch` | Renderer epoch that owns `completed_gpu_frame_id`. Present exactly when that column is present. |

Rules that follow from these definitions:

- Redraw identity (`draw_attempt_id`), submission identity
  (`submitted_gpu_frame_id`) and completion identity
  (`completed_gpu_frame_id`) are distinct. Join CPU rows to GPU work only through
  `(renderer_epoch, submitted_gpu_frame_id)` and
  `(completed_gpu_renderer_epoch, completed_gpu_frame_id)`.
- The submission counter restarts at 1 for each new renderer. Two rows with the
  same GPU frame id but different epochs are different submissions. Completion
  deduplication is keyed on the epoch/id pair, so a recreated renderer cannot
  suppress or falsely join a completion (`GpuCompletionTracker`).
- A retry row is a real draw attempt whose frame was not presented. A retry can
  happen **before** a submission (zero-size surface, swapchain recreation,
  `vkAcquireNextImageKHR` returning `ERROR_OUT_OF_DATE_KHR`) — then
  `submitted_gpu_frame_id` is empty — or **after** one, when
  `vkQueuePresentKHR` returns `ERROR_OUT_OF_DATE_KHR`; then the row keeps the
  real submission id, because the GPU work was submitted and will complete.
  Do not assume retry rows have no submission.
- `presented_count` advances only when the present call returned without error
  (`VK_SUCCESS` or `VK_SUBOPTIMAL_KHR`). An out-of-date present is classified as
  a retry and does not advance it. It is **not** a scanout, refresh or
  compositor-presentation count: a successful call means the request was
  accepted, not that an image reached the display.

Attempts skipped before the renderer is touched (no window, zero-size surface,
unfocused application) are not draw attempts and produce no row. A fatal renderer
error terminates the frame before the row is written, so the last attempt of a
fatal run is absent by design.

## Timings

All wall times use `std::time::Instant` on the main thread and are elapsed time,
not proof of work. `draw_interval_wall_ms`, `main_wall_ms`,
`stream_request_elapsed_ms` and `mesh_sync_wall_ms` reuse clock readings the
engine already took before this instrumentation existed. Every other stage,
wait and CPU-busy reading happens only while a capture is active, so an
uninstrumented run performs **no capture-only clock readings** — it still pays
the pre-existing baseline readings (the four above, the save timer and the GPU
timestamp bookkeeping in `timing.rs`), which are unchanged and must still be
budgeted in any overhead measurement.

| Column | Boundary |
| --- | --- |
| `draw_interval_wall_ms` | Time since the previous drawn frame's start (`dt` used by simulation). |
| `main_wall_ms` | Whole instrumented draw attempt on the main thread. |
| `main_cpu_busy_ms` | Main-thread CPU busy time over a span contained in `main_wall_ms` (see below). |
| `stream_request_elapsed_ms` | Elapsed time in the streaming request/poll section. **Elapsed, not work**: a no-op poll and a completed job both produce a duration here. |
| `physics_wall_ms` | Grab update plus the fixed-step physics call for this frame. |
| `mesh_sync_wall_ms` | Whole `sync_render_meshes` call, including chunk retention/uploads and the dynamic build and upload below. |
| `mesh_sync_fence_wait_wall_ms` | Sum of the blocking submission-fence waits inside this frame's `retain_chunks`, `upload_chunk`, `upload` and `upload_dynamic` calls, each timed at its own call site. |
| `dynamic_mesh_build_wall_ms` | Construction of the dynamic body mesh (`Physics::dynamic_mesh`). |
| `dynamic_upload_wall_ms` | `Renderer::upload_dynamic` for that mesh, including its internal fence wait. |
| `render_wall_ms` | `Renderer::render_with_lighting` call. |
| `save_wall_ms` | Save time accumulated since the previous row, including saves triggered outside the draw and saves that failed. Zero when no save ran. |
| `render_fence_wait_wall_ms` | Blocking submission-fence wait inside the draw, at its own call site. |
| `acquire_wall_ms` | `vkAcquireNextImageKHR` call duration. |
| `present_wall_ms` | `vkQueuePresentKHR` call duration: queueing the request only. |
| `gpu_prev_render_ms` | GPU queue timestamp span of a previously completed submission (see identities). |
| `gpu_prev_shadow_ms` | Shadow-pass portion of that submission; empty when it rendered no shadow pass. |

Overlap, explicitly:

- `main_wall_ms` contains the stream, physics, mesh-sync and render spans, plus
  uninstrumented remainder (input, HUD, camera, bookkeeping). The stage columns do
  not partition it and must not be summed and compared to it as if they did.
- `mesh_sync_wall_ms` contains `dynamic_mesh_build_wall_ms`,
  `dynamic_upload_wall_ms` and `mesh_sync_fence_wait_wall_ms`.
- `render_wall_ms` contains `render_fence_wait_wall_ms`, `acquire_wall_ms` and
  `present_wall_ms`.

### Fence wait scope

The engine's upload and retain calls wait on the same submission fence before the
draw does, so the frame normally blocks during mesh sync, not in the draw. The
waits are therefore reported in two named columns and never as one total:

- `mesh_sync_fence_wait_wall_ms` / `mesh_sync_fence_waits`: the pre-draw waits
  and how many happened. `mesh_sync_fence_waits` is 0 on a frame that uploaded
  nothing and retained every chunk; the wall column is then `0.0000`, an explicit
  zero rather than a missing value.
- `render_fence_wait_wall_ms`: the wait inside the draw, which is usually near
  zero precisely because the earlier waits already completed.

These two columns cover every `Commands::wait` call site in the renderer, so on
captured frames their sum is the frame's fence wait. They do **not** cover waits
from outside the renderer (there are none today) or implicit driver-internal
waits inside other Vulkan calls, which remain unmeasured. Never report either
column alone as "the" wait, and do not attribute the difference between
`dynamic_upload_wall_ms` and `mesh_sync_fence_wait_wall_ms` to buffer writes
without checking `mesh_sync_fence_waits`.

The waits are accumulated per frame and reset explicitly before mesh sync
(`Renderer::begin_frame_diagnostics`), so a previous frame's waits cannot leak
into a row.
- `gpu_prev_*` values belong to an earlier submission, never to this row's work,
  and are a queue timestamp span, not pure shader execution time: it includes any
  queue-internal stalls between the recorded top-of-pipe and bottom-of-pipe marks.

### CPU busy time

`main_cpu_busy_ms` is the delta of `CLOCK_THREAD_CPUTIME_ID` across the draw
attempt, read through a narrow `libc::clock_gettime` wrapper
(`metrics::thread_cpu_time`, used via `metrics::CpuBusySpan`). It is independent
of wall time: a frame that waits on a fence or the compositor shows wall time
without matching busy time, which is what discriminates stationary CPU work from
waiting.

The two clocks are read adjacently, not simultaneously: the wall clock starts
first and the CPU clock starts immediately after; the CPU clock stops immediately
before the wall clock stops, with no renderer or diagnostics queries in between.
So the busy interval is strictly contained in the wall interval and
`main_cpu_busy_ms <= main_wall_ms` up to clock resolution. The row-building work
after the wall stop (completion, diagnostics and counter reads) is in neither
interval.

- Supported on Linux and Android only. On other targets the reading is `None` and
  the cell is empty; an unsupported clock is never reported as zero.
- A failed `clock_gettime` call, an out-of-range `timespec` or a backwards delta
  yields an empty cell rather than a fabricated value.
- Readings are taken only while a capture is active.
- It is thread CPU time, not process CPU time: background streaming/meshing
  threads are excluded.

## Work counters

Counters are exact integers for this attempt unless stated otherwise. A counter
is missing when its source was unavailable (for example no renderer, or
diagnostics disabled). The `completed_gpu_*` and `gpu_prev_*` columns are
additionally empty on every attempt that observed no newly completed submission,
which is the normal case for most rows — that is a pacing gap, not an
unavailable source.

| Column | Meaning |
| --- | --- |
| `physics_fixed_steps` | Fixed physics steps actually executed by this frame's step call (0 when the accumulator did not reach a step). |
| `voxel_bodies_total` | Voxel bodies known to physics. Excludes the kinematic character. |
| `voxel_bodies_active` | Enabled and awake bodies. |
| `voxel_bodies_sleeping` | Enabled but sleeping bodies: retained state, no solver work. |
| `voxel_bodies_not_simulated` | Bodies the previous fixed step disabled, and therefore did not step at all: outside the resident columns, beyond the distance limit, **or** clamped at the below-world floor (`y <= -32`). The bucket does not distinguish those reasons. |
| `mesh_sync_fence_waits` | Number of fence waits summed into `mesh_sync_fence_wait_wall_ms`; 0 when the frame made none. |
| `chunk_mesh_uploads` | Chunk meshes uploaded to the GPU during this attempt. |
| `dynamic_mesh_builds` | Dynamic body-mesh constructions during this attempt. |
| `dynamic_mesh_uploads` | Dynamic body-mesh uploads during this attempt. |
| `save_attempts` | Saves started since the previous row, successful or not (matches `save_wall_ms`). |
| `save_failures` | Subset of `save_attempts` that failed. A failed save still consumed work and time. |
| `shadow_caster_meshes` | Non-empty meshes submitted to this attempt's shadow pass, independent of camera culling. **Empty when the attempt submitted nothing**: the renderer keeps reporting the previous pass's count, so it is written only for rows carrying a `submitted_gpu_frame_id`. A submitted attempt whose presentation retried did do this shadow work. |
| `gpu_prev_shadows` | 1 when the completed submission rendered shadows, else 0. |
| `gpu_prev_shadow_map_size` | Shadow map size of that completed submission. |

`voxel_bodies_total == active + sleeping + not_simulated`. Distant, unsupported
and below-world bodies are reported honestly as `not_simulated` instead of being
counted as work; splitting that bucket by reason is not implemented.

## Not measured in this slice

- Actual compositor presentation identity, presentation/scanout timestamps and
  display refresh joins. No presentation-time extension is used; do not derive a
  presentation timestamp from `present_wall_ms`. A `presented` result means the
  present call was accepted, nothing more.
- Driver-internal waits inside Vulkan calls other than the fence wait sites
  listed above, and any wait outside the renderer.
- Why a body is in `voxel_bodies_not_simulated` (residency, distance or
  below-world clamp).
- Energy, power and battery state. Nothing here is an energy measurement.
- Streaming-job counters (requested/completed/discarded chunk jobs),
  collision-publication events, edit events and queue depths. Only the
  *elapsed* time of the streaming section is captured, and it is labelled as
  elapsed, not work. Deferred to a later instrumentation slice.
- Allocation counts, retained capacity and process/graphics memory.
- Instrumentation overhead on the phone, and same-build repeatability. The
  pre-existing GPU timestamp queries do not qualify the new logging overhead.
  These are device measurements owned by the campaign lead and are **not run**.
- Any per-frame CPU busy time for background threads.

## Compatibility and change rules

- Schema v1 captures remain valid history and are not converted.
- While v2 is unshipped and unaccepted, column corrections land in v2 itself;
  this document is the contract. Once a capture on this schema is accepted, any
  further column addition, removal, reordering or redefinition requires
  incrementing `SCHEMA_VERSION` and the header line. Consumers must check the
  declared version before parsing and reject versions they do not implement.
- `metrics::COLUMNS` is the single source of column order; row writing follows it
  and a test asserts the complete name-to-value mapping of a fully populated row,
  not merely the row width.
