# Changelog

User-visible changes. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (`0.y.z` while pre-release).

## [Unreleased]

### Added

- Landscape mode: a streamed voxel landscape with distance rings that keep their
  voxel steps, aerial perspective and water that reads as water
  ([#64](https://github.com/5omeOtherGuy/Matterweave/pull/64), [#65](https://github.com/5omeOtherGuy/Matterweave/pull/65), [#67](https://github.com/5omeOtherGuy/Matterweave/pull/67), [#69](https://github.com/5omeOtherGuy/Matterweave/pull/69), [#70](https://github.com/5omeOtherGuy/Matterweave/pull/70), [#71](https://github.com/5omeOtherGuy/Matterweave/pull/71),
  [#76](https://github.com/5omeOtherGuy/Matterweave/pull/76),
  [#77](https://github.com/5omeOtherGuy/Matterweave/pull/77),
  [#79](https://github.com/5omeOtherGuy/Matterweave/pull/79),
  [#82](https://github.com/5omeOtherGuy/Matterweave/pull/82)).
- Sky dome and clouds, selectable as off, low or high
  ([#74](https://github.com/5omeOtherGuy/Matterweave/pull/74),
  [#80](https://github.com/5omeOtherGuy/Matterweave/pull/80)).
- Flora: wind-blown grass fields, blade ground cover out to 72 m and trees that fade
  with distance ([#68](https://github.com/5omeOtherGuy/Matterweave/pull/68), [#72](https://github.com/5omeOtherGuy/Matterweave/pull/72),
  [#85](https://github.com/5omeOtherGuy/Matterweave/pull/85),
  [#89](https://github.com/5omeOtherGuy/Matterweave/pull/89),
  [#92](https://github.com/5omeOtherGuy/Matterweave/pull/92)).
- World mode: walk a player across solid landscape terrain, through three biomes to
  landmarks you can see and reach
  ([#87](https://github.com/5omeOtherGuy/Matterweave/pull/87),
  [#90](https://github.com/5omeOtherGuy/Matterweave/pull/90)).
- Per-voxel surface variation within each material
  ([#84](https://github.com/5omeOtherGuy/Matterweave/pull/84)).
- Render scale, with one-pixel voxel steps kept through the upscale
  ([#83](https://github.com/5omeOtherGuy/Matterweave/pull/83),
  [#88](https://github.com/5omeOtherGuy/Matterweave/pull/88)).
- Mossbound, a separate monster-catching RPG sample with its own APK and desktop host
  ([#93](https://github.com/5omeOtherGuy/Matterweave/pull/93),
  [#94](https://github.com/5omeOtherGuy/Matterweave/pull/94)).

### Fixed

- The app stops rendering, simulating and running workers while the activity is
  paused ([#81](https://github.com/5omeOtherGuy/Matterweave/pull/81)).
- Sunlit white surfaces keep their form instead of washing out up close
  ([#91](https://github.com/5omeOtherGuy/Matterweave/pull/91)).

### Changed

- The Rust toolchain is 1.99.1; naga 30, glam 0.34 and pollster 1.0 replace older
  pins ([#100](https://github.com/5omeOtherGuy/Matterweave/pull/100),
  [#101](https://github.com/5omeOtherGuy/Matterweave/pull/101),
  [#102](https://github.com/5omeOtherGuy/Matterweave/pull/102),
  [#104](https://github.com/5omeOtherGuy/Matterweave/pull/104)).

## [0.5.0] - 2026-09-08

Engine detail and background lighting; see the
[release notes](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.5.0).

[Unreleased]: https://github.com/5omeOtherGuy/Matterweave/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.5.0
