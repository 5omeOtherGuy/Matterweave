# Sustained streaming correctness

`matterweave-core/examples/stream_stress.rs` exercises the public `AsyncWorld`
API against synchronous `World` generation and meshing. It adds a correctness
gate without changing production algorithms; it is not a rendering benchmark.

## What problem this solves

Asynchronous streaming, meshing and persistence need a long-run check that keeps
the queue bounds and still delivers the latest requested data under continuous
supersession, edits, resets and save/load replacement.

## How it works

Seed `20260908` starts with checkerboard detail over 16 chunks, then alternates
between the center and four remote corners. The data intentionally stresses
mesh-result memory.

Each cycle queues meshes, edits the current resident source, issues reversed
stream requests and requires the final requested window to publish. Every
resident cell is compared against a synchronous snapshot, and every accepted
mesh is compared byte-for-byte against fresh meshing at the current revision.
Periodic resets and save/load replacements invalidate outstanding work. The final
stable mesh request must complete, so rejecting all work cannot falsely pass.
Failed or unavailable workers and a 20-second publication timeout fail the gate.

The check asserts the existing implementation limits at every poll: at most 32
queued mesh jobs, 8 results, 8 MiB result capacity, one pending/completed stream,
one worker job, 147 resident chunks and 512 stored overrides. It does not
simulate an OS low-memory kill or establish process-memory bounds from logical
counters.

## What was verified

| Run | Result |
| --- | --- |
| Host, two-second run at source `ea436d3` | 45 cycles, 437 accepted mesh comparisons, 12 resets, five save/reloads; peak queue 32 jobs; peak result capacity 8,386,560 bytes, below 8,388,608 |
| ARM64 Android, ten-minute run at `ea436d3` | 600.013 seconds, 39,761 cycles, 219,832 verified meshes, 9,296 resets, 3,615 save/reloads; peak queue 32; peak result capacity 8,387,904 bytes, below 8 MiB |

The process completed normally and removed its disposable save. Strict example
Clippy passes after replacing modulo predicates with the pinned toolchain's
`is_multiple_of`.

See [device summary](../evidence/2026-09-08-stream-stress.json) for the exact
binary checksum, OS fingerprint, build configuration and sampled health values.
Raw evidence: `/mnt/bench/matterweave-dev/performance/engine-03/phone-stream-ea436d3`.

## Limits and open work

- Health samples are observations at roughly ten-second intervals, not
  continuous peak monitoring.
- The fixture ran headless with no GPU workload; these results do not establish
  a supported gameplay frame rate or a thermal improvement.
- This is a CPU streaming/meshing/persistence stress workload, not a graphics
  frame benchmark or evidence of comparative thermal efficiency.
- Rendering, collision, OS memory-pressure recovery and complete M3 acceptance
  remain separate gates.
