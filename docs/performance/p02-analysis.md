# P02 paired analysis

`tools/performance/analyze_wetland_pairs.py` reads completed trials from the phone
collector manifest and writes a fresh JSON/Markdown summary directory. It does
not contact the device. Example:

```sh
python3 tools/performance/analyze_wetland_pairs.py --input /absolute/pairs-run --out /absolute/new-analysis
python3 -m unittest discover -s tools/performance -p 'test_analyze_wetland_pairs.py' -v
```

The statistical-analysis workflow is descriptive: each trial pair is one
replicate; individual frames are correlated. Three planned pairs do not establish
reliable confidence intervals or statistical significance. Means, medians,
percentiles, IQR, missing observations and raw paired differences are retained.
No observations are discarded as outliers.

Compositor presentation timestamps use column two, deduplicated across repeated
histories. Both endpoints must first appear within the measurement window and
must have been observed consecutively in an actual dump. Unobserved history gaps
are reported separately, never invented as long frames. First-observation window
boundaries are approximate. Verified interval duration divided by selected span
is not coverage of the entire requested window.

The process CPU interval uses independent host midpoint times, CLK_TCK and stable
PID/start ticks. App CSV stage distributions cover the entire capture, including
startup/warmup: the CSV has no absolute timestamp anchor for an exact compositor
window join. CPU utilization is not energy measurement.

Pair controls check source composition/generator, recorded environment, fixture
hash, final camera/eye, shadow settings and total body count. Mutual start
temperature differences must stay within 1 C battery and 2 C skin; matching both
trials independently to an earlier reference alone is insufficient. Screenshots
still require human/lead inspection; metadata cannot prove visual equivalence.
Thermal and memory summaries retain all reported observations, including heating
and thermal-status changes during measurement.

## Executed evidence, 2026-09-08

The first two trials of `completion-02/p02-fullmap/pairs-a3` were analyzed offline.
All metadata controls matched. The lead inspected both `entered.png` captures:
same visible terrain, flora, water, camera and UI, with only animated water phase
differences apparent. These are entrance snapshots, not temporal-stability proof.

| First pair | Mean presentation ms | Median ms | p95 ms | Unsupported gaps | Process core equivalent |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reference | 73.551 | 66.325 | 116.065 | 0 | 0.599 |
| Candidate | 24.260 | 16.583 | 33.166 | 0 | 0.465 |

This is one two-minute measurement after two minutes of warmup for each build on
the OnePlus 13, using frozen generator-2 content. Repeats remain pending; this is
not a sustained-run, energy, generator-3 or release-performance claim. Exact build
hashes, environment and raw trial provenance remain in collector trial manifests.
Large raw files stay under `/mnt/bench/matterweave-dev/performance/completion-02`;
release artifact publication remains pending.

Ten focused Python tests passed: overlapping history deduplication, warmup/end
boundaries, missing history, sentinel timestamps, invalid clocks/dumps, quantiles,
process names containing parentheses, process restarts, mismatched scene/quality
and mutual temperature matching. GLM's earlier 300-second worker timed out without
producing code; the lead implemented and verified this tool. No trial-level
optimization acceptance is inferred from that worker or these software tests.


Build-profile correction: the original candidate build JSON incorrectly says
“Rust Android release profile.” Both frozen sources map Gradle `assembleDebug`
to Cargo `dev` (`opt-level = 2`, `debug = 0`), as verified directly at `0e5b3be`.
The original hashed manifests remain intact; this correction accompanies them.
The paired APKs use the same actual profile. No release-LTO performance inference
is made from them. New build manifests name the actual Cargo profile explicitly.


The first A4 pair (second completed pair overall) also matches metadata and the
lead-reviewed entrance images. Reference mean/median/p95:75.820/66.329/116.066ms;
candidate supported intervals:24.265/16.583/33.165ms. Candidate history has nine
unsupported gaps covering1.7547% of the selected120.012-second span. A conservative
mean upper bound over that span is24.653ms: divide total span by the count of
known consecutive endpoint pairs, assigning at least one interval to every gap.
Any hidden intermediate presentations increase that denominator. This bounds the
mean without inventing a percentile distribution; it does not repair missing
history. The analysis tool reports both supported statistics and this bound.

A repeatable tradeoff is already visible in these two pairs. Reference late skin
readings were39.139/39.503C, thermal status0; candidate49.199/49.872C, status2.
Candidate mean process PSS was288523.25/288509.25kB, versus reference
330632.5/326371.75kB (OS-reported units retained; no RSS/PSS addition). Candidate CPU
core-equivalent usage0.465/0.466 versus0.599/0.615. Faster presentation and lower
CPU/memory therefore do not establish lower heat or energy. Thermal regression
must accompany any acceptance decision. A visible30/60Hz target is being developed
as a separate cap experiment; source density, resolution, effects and simulation
remain fixed. No thermal benefit from that untested cap is claimed.
