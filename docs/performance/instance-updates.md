# Instance-only renderer updates

`Renderer::update_static_instances` changes transforms and prototype selection
using resident vertex/index buffers. Initial scene upload retains metadata for
unused prototypes too, so a previously unused LOD can become visible without a
geometry upload. Empty updates hide instances while retaining geometry; an empty
`replace_static_scene` releases the scene.

Planning validates prototype indices, finite transformed bounds, quarter-turn yaw
and the16MiB instance budget before GPU writes. The existing frame fence protects
instance writes and buffer growth. Growth constructs a replacement before swapping;
smaller updates reuse capacity. Geometry buffers are never rewritten by this API.
Accepted updates invalidate shadow depth and indirect light consistently with
other geometry changes. Collision/source data are outside this renderer operation.

RED `39c6f40` requires the missing instance-update planner/geometry metadata.
GREEN `d071d06` passes12 static-scene tests, including selection of unused geometry,
negative rotated bounds, invalid-input retention and hide/restore. `ba24090`
extends the actual Vulkan example to11 frames covering reselect, growth, rejected
update retention, hide/restore and shadows. Host llvmpipe Vulkan1.4.318 with validation
enabled passes without validation errors. No Android speed/energy claim is made.

```sh
cargo test -p matterweave-render --lib static_scene
cargo build -p matterweave-render --example instancing_smoke
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a "$CARGO_TARGET_DIR/debug/examples/instancing_smoke"
```

Logs: `/mnt/bench/matterweave-dev/performance/engine-02/instance-update-{red,green,native}.log`.
This is the renderer operation needed by automatic detail selection; the adapter
and Android transition check are separate integration work.
