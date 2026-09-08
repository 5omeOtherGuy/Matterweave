# Nonshipping full-map P02 reference

Base candidate0e5b3be (generator2, seed20260908). This branch deliberately
reintroduces advisory SUBOPTIMAL swapchain recreation and unconditional dynamic
geometry rebuild/upload, preserving interpolation, geometry, physics and scene
content. It is not for merging or release. Cache/WSI optimization unit tests that
assert reuse are intentionally incompatible with this reference behavior; geometry,
resource lifetime, capture and Android checks still apply. No speed claim before
fresh matched physical-device observations and identical scene/quality fixtures.

Paired candidate uses the unchanged0e5b3be source. Both APKs retain full map density,
source LOD, shadows1024, native3168×1440 on the same OnePlus13 and profiling enabled.
Capture/build/installed-source hashes and same-scene image checks must accompany
comparisons. Full campaign final-map workload acceptance remains separate if the
route generator changes after this frozen P02 fixture.
