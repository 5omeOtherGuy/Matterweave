# Interactive Terrain Lab

Source `aa020ee` was installed and operated on OnePlus13 Android16. Choose **EXPLORE TERRAIN LAB** in the app. Drag MOVE to translate, drag elsewhere to look, hold UP/DOWN to fly, switch FINE/MIXED, and use DIG/BUILD/SAVE/RELOAD. BACK returns to the chooser. The lab uses only `terrain-lab.json`.

[Device/build identity and exact checklist](../evidence/2026-09-12-terrain-lab/manifest.json), [lifecycle/geometry log](../evidence/2026-09-12-terrain-lab/runtime.log), [fine capture](../evidence/2026-09-12-terrain-lab/fine.png), [mixed capture](../evidence/2026-09-12-terrain-lab/mixed.png).

Phone checks passed: same-camera mode changes,4-cell dig,8-cell build, saved edit reload, process restart/load, elevation and release, HOME/suspend/resume with renderer recreation, BACK/chooser. Existing three saves remain byte-identical. Source count19284 becomes19280 after digging,19288 after building, and returns19280 after reload. Physical simultaneous touch, network-disabled execution and audio audibility remain unrun.

Host:14 focused lab tests pass; full app125 pass/1ignored before the additional elevation test; integrated terrain example six tests pass; four native lab frames render with software Vulkan validation. Independent Gemini review found missing touch elevation (fixed and separately reviewed); the suggested carry-over menu pointer was rejected because pointer reset across scenes is intentional. Strict app Clippy and CI are required before merge.

Reproduce the build/install using [DEVELOPMENT](../DEVELOPMENT.md). Host launch: `matterweave-explorer --terrain-lab --save /new/disposable/directory/world.json`; add `--smoke-frames 4` for a bounded render check. Focused test: `cargo test --locked -p matterweave-explorer --lib terrain_lab`. Scaled geometry regression: `cargo test --locked -p matterweave-render --example terrain_tiles_smoke`.

This is a bounded functional terrain lab: one resident authoritative world, synchronous derived rebuild on change and a fly camera without collision. It is not the microvoxel visual target, seamless production landscape streaming or a measured performance result. Those roadmap gates remain open.
