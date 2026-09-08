# Performance campaign: first unplugged stationary baseline

Evidence level: **one short physical-device baseline observation**. Not a matched
optimization comparison, a 20-minute sustained result, an energy measurement or a
completed performance campaign. The showcase has not been implemented.

## Verified conditions

- OnePlus 13 CPH2653, Android 16, display build BP4A.251205.006. The original v0.3
  release manifest records Adreno 830; no new driver qualification is claimed here.
- Installed APK was pulled and SHA-256 verified against v0.3.0:
  `352ce87ed6c667892b876c3026a039959903ff1e529901f92e4879d7058c7ba7`.
  Release source `2ce6ce3157e791ab6dd49ed00cd865ee5c1cacc6`, development opt-level 2.
- Current user save backed up, then restored byte-identically before the controlled
  run. Fixture SHA-256 `ca8a13a88f814374c57943543d6410b034047b97bb466206da2e6e7095f187af`;
  generator 1, seed 20260907, revision 42942, 29 bodies. App logs confirm 29 bodies,
  98 chunks and stationary eye `[12.0, 4.517111, 23.0]` throughout sampled FRAME logs.
- Shadows on, map size 2048, sun index 1, walking mode. Native landscape output
  3168×1440 was verified in a supporting app screenshot. No resolution adaptation.
- Brightness fixed at the prior value 6; min/peak refresh requested 60 Hz. All sampled
  SurfaceFlinger headers reported period 16666666 ns. Header period is display policy,
  not achieved app cadence. Touch/pointer developer overlays disabled for this run.
- User physically unplugged USB after TLS wireless ADB connected. All eight health
  samples reported AC/USB/wireless/dock power false. No simulated battery state used.
- Background updater/weather apps stopped and cached apps cleared. System, security,
  VPN and communication services preserved. Process inventories are private local
  evidence, not a claim of zero unrelated OS activity.
- Screen timeout temporarily extended from 30 minutes to 2 hours at the owner's
  request; effective setting verified, no device-policy cap. Secure lock unchanged.
- Idle readiness observed before launch: five samples across 120 seconds, battery
  30.1–30.5°C and current HAL skin 31.406–32.952°C, thermal status 0 throughout.
  This idle check is **not** the application-performance measurement.
- Ambient temperature and case conditions were not observed. Profiling and polling
  overhead remain unqualified. These conditions must be matched before comparison.

## Executed capture and result

Cold-launched Matterweave, then collected 120 seconds warmup and 120 seconds of
measurement. Collector completed at host elapsed 240.246 seconds with 475 dumps.
It checks the top-resumed app at startup and each health sample, rejects empty
histories for the frozen layer, and rejects external power. All checks passed.

Analysis uses SurfaceFlinger column 2, deduplicates valid actual-presentation
timestamps and selects endpoints first observed at host elapsed 120 ≤ t < 240.
A verified interval additionally requires consecutive endpoints co-observed in a
dump. This reuses the project's v0.3 evidence-analysis method, not the smoothed HUD.

| Measurement | Observed value |
| --- | ---: |
| Selected timestamps | 5,478 |
| Verified co-observed intervals | 5,477 |
| Unsupported selected gaps | 0 |
| Selected device-timestamp span | 119.988178 s |
| Mean interval | 21.907646 ms |
| Median interval | 16.583125 ms |
| p95 interval | 33.167458 ms |
| p99 interval | 33.169648 ms |
| Maximum verified interval | 33.172605 ms |
| Intervals strictly >33.3 / >50 / >100 ms | 0 / 0 / 0 |

Percentiles use the existing type-7 interpolation helper. The selected timestamp
span is **not** an exact coverage percentage for the requested host-time window.
There is no exact mapping between app CSV/GPU IDs and compositor timestamps.
No unsupported bridge is classified as a real frame stall.

All eight thermal samples reported status 0 (no throttling); this does not prove
absence of throttling between sparse samples. Battery temperature rose from 30.1
to 31.0°C; current HAL skin rose from 32.865 to 40.598°C. Cached temperature entries
were excluded. Battery level alone is not energy or power consumption.

**Interpretation:** this workload does not meet the campaign's p95≤16.67 ms target.
The result establishes a usable baseline observation, not the limiting subsystem,
same-build repeatability, sustained thermal behavior or an optimization win.

## Preserved failed attempt and correction

`baseline-01` was rejected: a foreground check found the launcher, and the original
surface layer became stale. No performance numbers are accepted from it. The
owner correctly questioned whether the idle check had been mistaken for an app
baseline. The idle check was readiness only; subsequent collection was strengthened
to validate app foreground and actual presentation histories. A relaunch screenshot
and two advancing current-layer histories confirmed real rendering before retry.
The precise cause of the foreground transition was not established.

## Reproduction and artifacts

Private raw evidence: `/mnt/bench/matterweave-dev/performance/run-01/device/`.
The baseline-02 directory contains runtime, raw surface histories, health samples,
completion marker, app log, CSV, post-run save and preliminary summary. Original
save and device/app inventories are not approved for public artifact publication.

- Raw SurfaceFlinger JSONL SHA-256:
  `dd1630e9a03ba5d4b154b28e65d5ee113cc0f3fdccd72027f864549b328f03e2`.
- Pulled v1 app CSV (`frame-profile-1788826133166.csv`, 962586 bytes) SHA-256:
  `8aec7cc27d7fbbeed0a61ad0adb6312711b2c17fe1c1c770ec4903f507fb0fc8`.
- The runtime collector is a short-run adaptation of the prior v0.3 collector,
  with foreground/power/valid-history guards. Offline analysis reuses v0.3
  `interval_stats`, `health_point` and `summarize_health`; raw hash and completion
  marker are checked before reporting. An initial strict parser rejected a blank
  trailing line; corrected it to ignore blank lines as the original analyzer does.
- Reproduce local analysis: `python3 /mnt/bench/matterweave-dev/performance/run-01/device/analyze-baseline.py`.

The app was stopped after collection, the app CSV flushed and pulled, and the user
save restored byte-identically. Original display/timeout/overlay settings remain
saved for restoration when device testing ends. This evidence is a local checkpoint;
a sanitized durable release artifact is still pending, along with repeated baselines,
P01 instrumentation overhead qualification and the mandatory showcase.
