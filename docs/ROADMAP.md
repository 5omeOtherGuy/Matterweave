# Delivery roadmap

This roadmap is an ordered delivery strategy, not an estimate or release promise.
Latest published prerelease: v0.5.0. Consult [STATUS](STATUS.md) for checks actually
run and exact source/build/artifact distinctions.

Released slices:

- v0.1 (M0/M1): the owner reported a successful OnePlus 13 run.
- [v0.2](V0.2.md) advances M2/M3 with greedy chunk rendering, bounded terrain
  streaming and interactive physics. Full M2 comparison and M3 stress gates remain
  open; M4–M6 remain future work.
- [v0.3](V0.3.md) implements dynamic shadows, bounded background preparation and a
  destruction playground; phone validation completed and v0.3.0 was released. Its
  multi-model protocol and [execution log](../execution_log.md) record the delivery
  process. It advances M3 and direct lighting and does not close equivalent-quality
  M2 comparisons or full M4 indirect-light/detail gates.
- v0.4.0 and v0.5.0 are published Android development prereleases; see below.

## In progress

The [engine completion orchestration plan](ENGINE_COMPLETION_PLAN.md) defines the
current E0–E8 execution order and parallel work boundaries for closing M2–M6. It
preserves this roadmap's acceptance criteria. Reflections and Voxel Relay passed
short physical-device functional gates on 2026-09-12; audio failed its suspend gate.
The existing second sample needs reuse/release acceptance, not replacement.

The [performance campaign](PERFORMANCE_PLAN.md) is executing, beginning with
trustworthy measurements and redundant-work reduction. The required
[dense alien showcase](SHOWCASE.md) supplies demanding real workloads throughout.
Use unplugged, matched thermal conditions and the [benchmark protocol](BENCHMARKS.md);
the numeric showcase and performance budgets are working targets, not achieved results.

**Done.** Foundation/flora source merged through PR6/PR7. PR8 delivered full 128 m
source terrain, fine-source collision/editing, water, 6,192 plants across 10 species and
ordinary Android chooser/controls; Generator 3 ground/elevated routes pass continuous host
physics, with the short waterside itinerary recorded too. Normal phone entry, movement and
edit persistence, plus 6-to-29-body destruction, were observed on development builds.
Complete journal/respawn recovery is host/native verified, and the later shadow-cache
0.4.0 APK was device-checked and released through PR8. The engine now reuses unchanged
shadow depth with host Vulkan and Android functional checks. Automatic detail selection
and instance-only updates pass a 9-phase native Vulkan gate; background collision
preparation/publication passes a direct native Android floor creation/removal gate;
source-version-checked collision preparation and shared chunk snapshots are host-tested on
the integration branch. The first diffuse indirect reference passes Android
functional/lifecycle checks, and the background adapter passes host and Android checks with
nine Android presentations during CPU preparation. Production frame-loop scheduling is
integrated and device-checked.

**Open.** Automatic LOD transitions, indirect illumination/reflections, the
equivalent-quality renderer comparison, sustained efficiency and thermal acceptance,
streaming completion, a second released sample (M6) and the remaining M2–M6 gates. The
background adapter's production cadence, upload costs and quality/latency acceptance
remain open. The first diffuse indirect reference's 39–45 ms synchronous full
preparation/upload phases motivated background scheduling. Two matched P02 pairs show
faster presentation and lower CPU and memory with increased heating; the third pair
confirms the tradeoff. The [bounded full-image ray/raster/hybrid checks](performance/renderer-comparison.md)
pass on Android and host, advancing M2 feasibility while equivalent production quality and
primary-path cost selection remain open. These increments do not close M3/M4 acceptance,
and v0.5 delivers them through PR9/PR10 without closing M2–M6.

**Closed.** The separate 30/60 Hz frame-cap experiment was reassessed as engine pacing
work: `matterweave-pacing` is integrated into the production frame loop and passes a host
and a physical OnePlus 13 gate. Production frame-loop scheduling is no longer open; its
fixed cap survives as one policy, and no efficiency or thermal benefit is claimed. See
[frame pacing](performance/frame-pacing.md).

Owner steering (2026-09-08): prioritize reusable engine implementation. The wetland
supplies validation workloads; additional demo-save compatibility and showcase UI/content
refinement are not gates ahead of M2–M5 engine progress.

## M0 — Reproducible Android foundation

Apply accepted ADR-0014 and resolve the Rust foundation proposal in ADR-0015. Pin compatible Rust/Cargo, edition/MSRV, JDK, Gradle wrapper, Android Gradle Plugin, SDK/NDK and shader tools; add CMake only for components requiring it. Commit the workspace lock/toolchain records. Assess existing Android integration and graphics components before custom work. Choose and document an initial Android/API/Vulkan profile; verify capabilities rather than inferring them from marketing names. Build a native ARM64 application with lifecycle, surface handling, multitouch actions, logging, capability reporting and frame instrumentation. Add Rust checks, host/native builds and APK artifacts in CI.

**Done:** a clean checkout builds an APK using documented commands; the application presents a frame and responds to input; background/resume and surface recreation are handled; a physical-device smoke result is recorded when access exists. If hardware is absent, publish the APK and mark that subgate outstanding. Verify native-library page-size compatibility and packaging under the chosen toolchain. No voxel fidelity or mobile performance claim is made at this stage.

## M1 — First useful voxel slice

Implement queryable CPU voxel data, material identities, a seeded small scene, simple rendering, touch movement/look/action, and one visible edit interaction. Add stable IDs/revisions, reference ray queries and a minimal save/load round trip. Use a deliberately straightforward baseline; an initially dense small fixture is allowed as a reference, not a production-world allocation strategy.

**Done:** a person can explore and edit real voxel content in the APK; a save/reload preserves those edits; tests cover chunk/voxel boundaries, negative coordinates and query correctness; memory and timing counters describe real workload quantities. Device behavior is recorded separately from build/host results. Commit exact build/install/test instructions and a short capture or reproducible manual procedure.

## M2 — Representation and renderer selection

Use M1 fixtures to compare focused compute traversal, extracted-surface rasterization and a hybrid prototype. Investigate sparse blocks, static versus dynamic object data, initial LOD/residency and visibility. Include an orthographic workload, dense foliage/thin geometry, close detail, occlusion and dynamic edits. Time-box each experiment around a decision; reuse common fixtures and infrastructure.

**Done:** a report uses the [benchmark protocol](BENCHMARKS.md), compares equivalent quality and total costs, and records an accept/reject/defer conclusion in ADR-0005/0006/0007. Select a primary path with evidence. If physical hardware is absent, keep mobile performance selection provisional; continue data correctness and integration work. Retain a minimal reference implementation and useful regression fixtures.

## M3 — Interactive physics and bounded streaming

Assess Rust physics first and integrate a qualifying library, character movement, dynamic voxel objects and a constraint interaction. Jolt adoption requires a documented major advantage under ADR-0014, including binding/integration costs; it is not a mandatory comparison if no credible major gap exists. Reuse or extend qualifying components for a bounded fracture/destruction example, versioned collision updates and edit persistence. Stress streaming, eviction, cancellation, rapid camera reversal and memory pressure. A renderer/physics update policy must define the interval between a world edit and collision publication.

**Done:** a breakable structure reacts physically; movement and queries remain correct after edits and LOD changes; stale jobs cannot restore old geometry/collision; saves round-trip state; queues, staging and residency obey explicit limits. Record mass/inertia handling and limitations. Run sustained native tests when hardware is available.

## M4 — Dynamic lighting and stable detail

Build the Lumen-like GI/reflection capability on a direct-light baseline. Add temporal inputs and evaluate reconstruction. Refine automatic detail selection/transitions for thin geometry, zoom and movement. Compare cached/probe lighting and selective tracing; optional hardware RT is a capability path, not an assumed universal feature.

**Done:** recorded tests show changing indirect illumination after moving lights and opening/closing an enclosure; reflections respond to scene changes; geometry transitions are visually reviewed; ghosting, disocclusion and lighting latency have explicit observations and thresholds. Costs and quality are measured together. ADR-0008/0009 are resolved for this milestone without promising all later research features.

## M5 — Sustained mobile fidelity

Profile actual bottlenecks across the declared initial device set. Tune useful parallel work, memory traffic, materials, shadows, foliage/atmosphere, streaming, frame pacing and thermal adaptation. Inventory accelerator capabilities and execute focused experiments justified by existing workloads. Reuse qualifying Rust interfaces, extend existing ones, or create a narrow backend when necessary/significantly advantageous. Evaluate NPU candidates with transfer and synchronization costs included; record useful negative results. Keep new interfaces independently testable with explicit capability and lifetime contracts.

**Done:** a sustained benchmark report includes the actual quality profile, device/driver/OS, resolution, frame-time distribution, memory, thermal behavior and any unmeasured quantities. Adaptive and fixed-quality runs are separated. The supported profile/device table is evidence-based. No claim of optimizing all Android hardware is made from one phone.

## M6 — Reusable framework proof

Create two small playable samples using the same engine. Suggested pair: an exploration/destruction scene and a top-down or 2.5D procedural turn-based encounter game with original assets. Demonstrate different cameras, environments and mechanics through shared input, world, physics/query, save and rendering services. Add the minimal animation, audio, UI and asset pipeline those samples actually need.

**Done:** both samples build and run with documented authoring and extension steps, seeded behavior and persistence. Game-specific rules are outside the engine. Real mobile controls are usable. The reusable engine's supported capabilities and limitations are documented; the second game does not require forking engine code.

## Later research and productization

Neural radiance caching, ReSTIR-family sampling, frame generation, advanced accelerator paths, fluids/soft bodies, broader device support, multiplayer, a production editor and scripting can follow demonstrated need. Prototype these independently of the core milestone path. Owner decisions for licensing and distribution are tracked in [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).

## Definition of engine demonstrator done

M0–M6 acceptance criteria are met for a declared device/profile set, or remaining gaps are explicitly identified rather than hidden. A fresh checkout builds, documented samples run on real Android hardware, and evidence covers fine geometry, automatic detail, dynamic lighting, interactive physics, resource efficiency and reuse. The repository contains all code, build instructions, dependency provenance, results and known limitations needed to continue development. This is an engine demonstrator, not a claim of commercial production readiness.
