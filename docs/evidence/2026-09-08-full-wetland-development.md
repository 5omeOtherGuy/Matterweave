# Full wetland development trial — 2026-09-08

This is installed development evidence, not showcase acceptance or a release.
Source `7e1143ea7807b759d8cb4314bba0955056ccdf86` built without concurrent source
edits. APK SHA256 `35cbd2e666c3c4457d230fbc8d11b15011603c5b49a7366654d16cbaee6720cc`.
Gradle debug package uses the project's optimized Rust Android release profile.
ARM64 ELF/ZIP16KiB alignment and APK v2 signature verification pass.

OnePlus13 CPH2653, Android16, fingerprint
`OnePlus/CPH2653EEA/OP5D55L1:16/BP2A.250605.015/V.R4T3.52da06f-2e397f6-2e81775:user/release-keys`;
Adreno830,Vulkan1.3.284,driver2150760522. Physical surface3168×1440.
Initially unplugged/no charging,76% battery,31.9°C. No matched thermal protocol was
run for these short functional trials, so no causal performance claim follows.

The ordinary chooser entered the full generator2 map:34,864,520 expanded cells,
8,324 total placements,6,221 plants/ten species. CPU scene preparation logged1.337s;
this excludes first-frame/upload latency. Scene counts/composition are in
[the host manifest](full-wetland-generator2.json); they are not measured phone RAM.

Observed checks:

- Existing generator1 wetland file retained; generator2 saved into
  `wetland-session.json.recovery-1.json`.
- Normal movement and full-map rendering, source Remove/Place actions and persisted
  journal reload after process relaunch. Exact saved edits: instance
  `i_shell_5_8_3`, `[24,3,13]`→0 and `[24,3,14]`→10.
- Ordinary walking entered shallow then deep basin water. Saved eye moved from
  `[46.019775,13.267179,72.35753]` to `[59.38605,10.767099,72.43977]`,
  below the12m basin surface; submerged tint rendered. This checks nonblocking
  water/physical bed behavior, not fluid simulation or a complete waterside route.
- HOME/resume rendered the actual scene. The separate lifecycle trial recorded180
  valid rows but only renderer epoch1; renderer destruction/recreation is not
  established by that trial. Initial1800-row capture finished before HOME/resume.
- Legacy world SHA256 remained
  `986bcd5629cebcdc8678f712fe899a4555c75e7b5c7c11e4efd48070bba67907`.
- Shadow on/off at an unchanged camera shows the visible triangular stair shading
  comes from the shadow pass. Full near/far/temporal quality acceptance remains open;
  disabling shadows for a comparison is not the delivered quality solution.

Both profiles pass the typed identity/format validator, with no missing draw
attempts or orphan completions. Unavailable streaming timings remain blank. These
are short functional captures, not controlled or sustained benchmark results.

Ground/elevated continuous Rapier walking subsequently failed the host regression
at `3af4115`; route clearance alone did not establish traversability. A30cm bounded
Rapier autostep fixes quarter-metre stairs (45 physics tests pass) but does not yet
fix the full routes. A55cm trial made no further progress and was rejected.
Phone complete-route/destruction/visual gates and matched comparisons remain open.

Raw local artifacts and SHA manifest:
`/mnt/bench/matterweave-dev/performance/completion-02/phone-final/`.
They are not yet published durable release artifacts. Phone is force-stopped;
shadow setting restored after the diagnostic. Latest prerelease remains v0.3.0.
