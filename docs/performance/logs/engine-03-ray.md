# Engine-03 ray recovery log

**Actions:** Hy4 extended retained ray packing/shader work with a native Vulkan
probe example. Lead recovered dirty output after timeout, ran all 73 renderer
tests, strict Clippy and synchronization validation.

**Issues:** Worker numeric checks passed but omitted the render-pass dependency
before image readback. Lead observed 48 READ_AFTER_WRITE hazards and retained
RED at 6f4ea61. Worker Clippy needed a test initializer correction.

**Decisions:** Keep this as an explicit bounded reference harness; no production
renderer selection follows from one-pixel probes. Preserve CPU oracle semantics
and disclose edge/inside differences from exposed-surface rasterization.

**Solutions:** Added attachment-write/layout-transition to transfer-read dependency,
transfer-write to host-read barrier, and orderly command/framebuffer release.
Seven focused tests, 73 renderer tests, strict Clippy and 24 GPU probes pass;
synchronization validation reports no errors after correction. Android pending.

**Insights:** Numeric readback agreement on one driver does not establish correct
GPU synchronization. The native check must examine validation output as well as
its terminal count.

**Android correction:** The initial Adreno run failed two of 24 numeric probes,
retained in phone-ray-3f4aa5b/report.txt. Boundary and near-plane precision were
reproduced at b6fede0, then stabilized with explicit tolerances. Six offset probes
constrain the grid-plane snap. Adreno now passes 30/30; host synchronization
validation is rerun on the same expanded harness before integration.
