# Instance-only renderer updates

## What problem this solves

Changing an instance transform or prototype selection previously required a
geometry re-upload. Camera-driven work such as automatic detail selection needs
to switch resident instances between already-uploaded meshes without touching
vertex or index buffers.

## How it works

`Renderer::update_static_instances` changes transforms and prototype selection
using resident vertex/index buffers. Initial scene upload retains metadata for
unused prototypes too, so a previously unused LOD can become visible without a
geometry upload. Empty updates hide instances while retaining geometry; an empty
`replace_static_scene` releases the scene.

Planning validates prototype indices, finite transformed bounds, quarter-turn yaw
and the 16 MiB instance budget (`STATIC_INSTANCE_BUDGET_BYTES`) before GPU writes.
The existing frame fence protects instance writes and buffer growth. Growth
constructs a replacement before swapping; smaller updates reuse capacity.
Geometry buffers are never rewritten by this API. Accepted updates invalidate
shadow depth and indirect light consistently with other geometry changes.
Collision/source data are outside this renderer operation.

## What was verified

Host only.

- RED `39c6f40` requires the missing instance-update planner and geometry
  metadata. GREEN `d071d06` passes 12 static-scene tests, including selection of
  unused geometry, negative rotated bounds, invalid-input retention and
  hide/restore.
- `ba24090` extends the Vulkan example to 11 frames covering reselect, growth,
  rejected-update retention, hide/restore and shadows. Host llvmpipe Vulkan
  1.4.318 with validation enabled passes without validation errors.

```sh
cargo test -p matterweave-render --lib static_scene
cargo build -p matterweave-render --example instancing_smoke
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a "$CARGO_TARGET_DIR/debug/examples/instancing_smoke"
```

Logs:
`/mnt/bench/matterweave-dev/performance/engine-02/instance-update-{red,green,native}.log`.

## Limits and open work

- No Android speed or energy claim is made.
- This is the renderer operation needed by automatic detail selection; the
  adapter and the Android transition check are separate integration work.
