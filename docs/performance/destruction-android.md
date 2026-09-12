# Rendered destruction acceptance

PR #33 (`c86980b`) delivered the bounded physics API and its independent host
review. This follow-up runs the same physics and actual dynamic mesh upload path
inside the native Vulkan app. A diagnostic HUD identifies the current phase and
live body/weld counts. It owns only its request marker and report; no user save is
loaded or written.

The fixture has six source bodies containing 64 half-metre voxels and two welds.
It renders the original structure, loose-body fracture, all 64 pieces while
simulated, a JSON PhysicsSave round trip, renderer recreation/zero extent, and 20
internal reset/load cycles. The runtime checks 24 kg conserved mass, body/collider/
constraint counts, and bounded retained dynamic mesh bytes after each reset.
Internal reset/load cycles are distinct from actual Android lifecycle callbacks.

At source `d22954c`, the initial Android run on OnePlus 13 / Android 16 /
Adreno 830 (Vulkan 1.3.284, driver 2150760522, validation disabled) reported
**PASS**: 1501 presented frames, 1520 uploads, 1519 rebuilds, exactly 64 pieces,
24 kg conserved mass, 20 reset/load cycles, one internal renderer recreation and
one actual HOME/resume. The latter was injected during phase 2. Retained dynamic
mesh bytes stayed at 92,160 under the declared 137,216 bound. This is a CPU mesh
cache capacity check, not proof of bounded total app/GPU memory or no leaks.
The debug APK SHA-256 and immutable report/log hashes are in the
[initial-run manifest](../evidence/2026-09-12-destruction/manifest.json).

Independent Gemini review (`w_d4d2607a`) then found two concrete defects: surface
resize/recreation could repeat phase preparation while retaining frame counts, and
a retried final reset frame could re-enter the completed last phase and duplicate
its summary with the restored six-body state. Lead source `5347e91` preserves
simulation/preparation across GPU resource loss and separates finalization from
phase progression. Two regression tests drive the real progression with controlled
presentation results, checking unchanged physics on Retry, exact frame quotas,
exactly one 64-body final-phase summary, and one final reset. Internal recreation
and Android suspend/resume remain separate events.

Corrected source `5347e91` passes the repeated physical Android run with the
same 1501/1520/1519 presentation/upload/rebuild totals and mesh-byte bound. This
time HOME/resume was injected during phase 4, after internal recreation: exactly
one internal recreation, one platform suspend/resume pair, 25 unique phase summaries
with 60 frames each, and one final restored frame. All four current user-save
hashes match the fresh pre-run backup. The full app suite passes **157 tests**
(one existing ignored), including all 10 destruction tests; scoped strict Clippy,
formatting and a 151-frame host Vulkan run pass. Corrective independent Gemini review `w_004fc6f2` found no defects; lead accepted
the repair against source, host tests and the corrected physical run. This is no
performance, thermal or complete M3 claim.

Unmodified phone captures: [loose fracture, phase 1](../evidence/2026-09-12-destruction/phase-1.png),
[full fracture, phase 2](../evidence/2026-09-12-destruction/phase-2.png),
[reset cycle, phase 16](../evidence/2026-09-12-destruction/phase-16.png).
The HUD determines the capture phase; screenshot timing can lag report polling.

The lead also corrected an inherited documentation claim: `[6,2,2]` is not the
largest permitted body shape under the 32-voxel/axis-6 limits. Fracture energy now
describes the actual dimension-dependent radial term and subsequent speed clamp
without an incorrect universal numeric maximum. Physics behavior is unchanged.

Developer entry: `--destruction-check --save /disposable/directory/unused.json`.
Android uses the existing one-shot `files/engine-check.txt` marker containing
`destruction`; the report is `files/destruction-check-report.txt`. A fresh run must
not reuse an old report as evidence. M3 streaming reversal/cancellation/pressure
acceptance remains separate and belongs to the preserved D2.3 assignment.
