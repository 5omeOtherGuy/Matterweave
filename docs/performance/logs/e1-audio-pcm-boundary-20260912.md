# E1 audio PCM ownership boundary — 2026-09-12

## Actions Taken

- Verified starting HEAD `c277f158a4db06df6bc821cddc1cae7aff01a3bb` and preserved
  unrelated STATUS/ADR deltas. Retained completed-command acknowledgments, Unload
  generations, pause compensation, error-state handling and prior regressions.
- Added four ownership/concurrency fixtures first. RED checkpoint `75fd1ec` records
  the intended **compile-time** failures: no private PcmPool module, no independent
  service pool owner, and no raw-owner pointer accessor. This is an implementation
  contract gate, not a runtime reproduction or proof of aliasing UB.
- Replaced the live mixer's owned PCM Box with an independent `Arc<PcmPool>` shared
  by the service and mixer. Registration never dereferences the core or its Box.
- Converted the mixer Box to raw ownership before callback wiring, with one private
  owner that reconstructs it only after backend shutdown. Removed the manually
  asserted AudioService Send implementation: the private backend trait now requires
  Send, so the service derives it from its fields.
- Audited all production control/core/pool accesses, callback dereferences and
  unsafe Send/Sync implementations. Inspected pinned ringbuf 0.5.1's actual
  producer/consumer release/acquire implementation, not just its type names.
- Ran package tests, allocation detector, strict scoped clippy, formatting and a
  twenty-cycle host diagnostic. No descendants, device access or Android builds.

## Issues & Friction

- `UnsafeCell` alone would not repair exclusive live-core aliasing: the previous
  registration accessed PCM through `&mut MixerCore`/its Box while the callback
  held its own mutable core reference. Separation of the allocation and removal
  of live owning Box/reference access are both necessary.
- Retaining a Box after deriving a temporary mutable-reference-based callback
  pointer would complicate the ownership/provenance argument. `Box::into_raw`
  before any callback escapes avoids that issue; the wrapper never dereferences
  its pointer during ordinary control operations.
- The first concurrent test build borrowed `&CorePtr`, requiring Sync, which is
  deliberately not implemented. Corrected the fixture to move the Send pointer
  wrapper into the render thread; no Sync assertion was added to bypass the error.
- Miri is **NOT RUN**. Installed rustup components contain cargo, clippy, rustc,
  rustfmt, LLVM tools and host/Android std, but not Miri. No toolchain installation
  was attempted. Functional and allocation tests do not prove all aliasing rules.

## Decisions & Rationale

### Reuse and bounded representation

Use std `Arc`, `Box` and per-sample `UnsafeCell<f32>` plus the existing pinned FIFO;
no dependency, new queue, lock, allocator or custom ownership counter was added.
`Arc<[f32]>` alone cannot support registration after sharing without mutation;
copy-on-write would duplicate PCM and locks would violate callback requirements.
The private UnsafeCell wrapper supplies only checked range writes and value reads.
Its contract still forbids all conflicting payload access; it is not synchronization.

The service allocates exactly `MAX_PCM_BYTES / 4` initialized cells, and construction
rejects lengths beyond that cap. UnsafeCell has its inner f32's layout, preserving
4 MiB of sample storage. An Arc control allocation adds constant-sized ownership
metadata, not a second PCM buffer. Registration validates once then writes directly
into the owned destination cells, with one assignment per sample and no staging
copy. The callback does not clone/drop Arcs, allocate, resize, block or free PCM.

### Ownership and reference proof

1. **Core allocation/provenance.** `CoreOwner::new` consumes the initial Box with
   `Box::into_raw` before backend creation (`src/backend.rs`). While callbacks may
   run, there is no owning Box or control-side reference to the mixer allocation.
   `CoreOwner::ptr(&self)` copies a raw pointer only; borrowing/moving the service
   or owner accesses pointer metadata, not the pointee. CoreOwner is not Clone
   and exposes no Deref or mutable-core accessor.
2. **Unique mutable callback borrow.** Only the serialized render role dereferences
   CorePtr: AAudio's data callback or the synchronous mock render. Each reference
   lasts only for that invocation. The error callback touches shared atomics only.
   Ordinary service operations use mirrors, the FIFO, shared atomics and the
   separate pool Arc, never the core allocation. No concurrent mutable core
   references are permitted by the backend contract.
3. **PCM is not exclusively reachable through the core borrow.** The mixer's
   `&mut self` contains an Arc handle, not ownership of the pool Box. Arc dereference
   yields shared PcmPool access. The service has a separate Arc to that same object.
   PcmPool exposes immutable allocation metadata and shared references to
   UnsafeCells; neither caller nor pool methods create `&mut PcmPool`, a mutable
   whole-pool slice, or references to inner f32 values. `get().write` and
   `get().read` access only the chosen cell's raw payload, copying f32 by value.
4. **New registration/write ownership.** The service's single control thread
   obtains a checked free range, which is either never published or retired under
   the completed-Unload rule below. The source slice cannot alias the private
   pool through any public API. `PcmPool::write` bounds-checks the whole destination
   before writing and requires no reader, writer or inner reference to overlap it.
   Other cells may be read concurrently, under shared UnsafeCell-containing
   references. This permission does not extend to conflicting accesses.
5. **Publication happens after writes.** Only after the complete write does the
   service enqueue LoadClip, and Play follows it in the same single-producer FIFO.
   ringbuf 0.5.1 writes the command before publishing its index; shared write-index
   publication is Release and consumer index observation is Acquire. Thus a voice's
   pool reads happen after its sample initialization. Cached indices do not change
   this: newly visible commands require fetch, and successful pushes commit.
   Inspected dependency references: `src/traits/producer.rs:60`,
   `src/traits/consumer.rs:106`, `src/wrap/caching.rs:114,133`, and
   `src/rb/shared.rs:97-133` in the pinned registry source.
6. **Retirement excludes readers before reuse.** The matching-generation Unload
   silences all voices for that clip. Its FIFO sequence is published with Release
   only after callback PCM reads and live-mask writes finish. Control-side Acquire
   must cover that sequence before the range returns to the free list. A callback
   finishing after enqueue but before consuming Unload cannot grant reuse. Stale
   handles cannot enqueue later Plays for the retired clip; replacement Load/Play
   follows the newly completed write. The previous deterministic regression and
   exact full-pool PCM tests remain unchanged in meaning.
7. **Recreation and shutdown.** Recreation closes/drops the old backend and joins
   its callbacks before copying the same raw pointer into a replacement. It never
   dereferences the core or reconstructs a Box. Failed open/start leaves CoreOwner
   and both pool owners alive. `AudioService::drop` drops backend first; declaration
   order then drops CoreOwner, reconstructing the original Box exactly once, then
   the service pool Arc. The core's Arc is destroyed only after callback join; the
   service's Arc remains alive through that destruction. No callback-owned pool
   reference survives the join.
8. **Send/Sync.** PcmPool's unsafe Sync implementation is justified because safe
   methods access only immutable metadata; every payload read/write is unsafe
   with the range/exclusion/order obligations above. Send is automatic for its
   Box of UnsafeCell<f32>. CorePtr's existing unsafe Send permits raw pointer
   transfer only, requiring its unique render role and owner lifetime; it is not
   Sync. CoreOwner inherits that Send property without dereferencing the target.
   OutputBackend: Send makes AudioService automatically Send; a compile assertion
   checks this. The existing AAudio wrapper's Send contract is unchanged.

This addresses the identified source-level aliasing blocker. The proof depends on
backend callback serialization/close-join guarantees and the documented private
unsafe call-site obligations. It is not a claim of formal or independently reviewed
Rust alias-model verification; unsafe review and native rerun remain gates.

## Solutions Applied

- New private `src/pcm.rs`: fixed initialized per-cell storage, checked unsafe range
  write/value read contracts, and documented Sync justification.
- `src/backend.rs`: private raw CoreOwner with Box::into_raw/from_raw lifetime,
  strengthened pointer obligations and Send backend bound.
- `src/service.rs`: independent pool Arc, registration through that owner, raw-only
  core metadata access, backend-first/core/pool drop order; no live Box dereference.
- `src/mixer.rs`: pool Arc and narrow value reads; existing completed-command
  publication remains after all reads. No callback allocation/ownership changes.
- `src/lib.rs`, crate README and private test modules: module wiring, documentation,
  adaptation of the deterministic after-drain fixture, four new tests.

### Verification

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/completion-20260912/targets/audio-pcm-boundary
export CARGO_BUILD_JOBS=2
cargo test -p matterweave-audio --locked
cargo clippy -p matterweave-audio --all-targets --locked -- -D warnings
cargo fmt -p matterweave-audio -- --check
cargo run -p matterweave-audio --locked --example audio_diagnostic -- --cycles 20
python3 tools/check_docs.py
git diff --check
```

| Criterion | Result | Evidence |
| --- | --- | --- |
| No mutable live core/whole-pool borrow during registration | PASS (source inspection/proof) | Production registration uses only the service's PcmPool Arc; core accesses are raw pointer copies, serialized render dereferences and post-join owner destruction. |
| Writes precede publication; reuse follows completed Unload | PASS (host + source proof) | Existing deterministic interleaving and real PCM lifetime tests; new concurrent registration/render and disjoint cell fixtures. |
| Fixed memory and no callback allocation/locks | PASS (host + inspection) | Fixed capped cell allocation; 10,000 callbacks report zero allocation/deallocation including Unload; no new callback lock or queue. |
| Tests, strict scoped clippy, fmt, host diagnostic | PASS | 47 tests: 19 unit, 21 acceptance, 2 lifetime, 4 recovery, 1 allocation; twenty diagnostic recovery/resume/shutdown cycles. |
| Independent unsafe review and Android rerun | NOT RUN | Lead-owned; no device or native build performed by this worker. |
| Miri / instrumented alias-model check | NOT RUN | Miri not installed; no coverage percentage or formal soundness result claimed. |

Documentation/diff verification: PASS — 162 Markdown files, 529 local links,
16 ADRs and 20 requirements; `git diff --check` clean.

New tests: registration inside a live mutable render invocation using the test
cut-point; two threads doing 10,000 disjoint cell read/write iterations; 2,000 real
render-thread callbacks of 64 stereo frames while control registers sixteen
16,384-sample clips; pool ownership surviving service teardown. The concurrent
service fixture publishes a half-pool 0.125 clip, writes new 0.75 clips into other
ranges, and requires every rendered sample to remain exactly 0.125. It does not
claim a guaranteed CPU overlap duration or performance measurement. Scoped threads
join even on panic before service/core destruction.

Retained logs are under
`/mnt/bench/matterweave-dev/completion-20260912/audio-pcm-boundary/`:
`red.txt`, `test.txt`, `clippy.txt`, `fmt.txt`, `diagnostic.txt`, `docs.txt`, and
`access-audit.txt`. Build targets are removed after final checks, not the text logs.

Unrelated initial/final SHA-256 values remain:

- STATUS: `73076404114a4e5726b10349f21f93fda1b42b1a8c531e89bb0f2a27d6cf2f9f`
- ADR-0016: `1295b92ca250d8588ca01b5847760a05002c1496beb0394f90c79c588918a343`

## Insights

- Disjoint indices and correct FIFO acknowledgment establish race exclusion, not
  valid ownership references by themselves. Separating the Arc allocation and
  never borrowing a live owning Box completes the missing reference argument.
- UnsafeCell only permits shared-reference interior mutation; it does not permit
  multiple exclusive references or concurrent conflicting sample access.
- Core allocation ownership and callback access are separate responsibilities:
  one raw owner controls destruction, while serialized callbacks temporarily
  borrow its target. PCM is shared independently and reclaimed by command proof.
- Passing host tests is necessary evidence, not an independent unsafe review.
  The lead should review this boundary and rerun Android before delivery; prior
  native passes at older revisions do not validate this changed ownership design.
