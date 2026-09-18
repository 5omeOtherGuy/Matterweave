# Fidelity measurement

Three independent scripts that decide visual-fidelity claims about the landscape
sample. They read captures; they do not produce them. Nothing here renders, and
nothing here is a quality setting.

The point of the corpus is that "it looks better" is not a criterion. Each claim
in a landscape PR should name the metric, the framing and the two numbers.

## The scripts

| Script | Question it answers |
| --- | --- |
| `voxel_step_metric.py` | Does distant terrain read as **voxel-stepped** rather than as a smooth interpolated surface? |
| `frame_gap_metrics.py` | **Which** gap to the reference is largest right now (hue spread, content density, saturation, bare ground)? |
| `veg_metrics.py`, `veg_metrics_palette.py`, `veg_metrics_ref.py` | Ground-cover coverage, blade fraction and palette match against a reference crop. |

Usage is `--help` on each; every one takes image paths and an optional
`--crop X0,Y0,X1,Y1`, and none of them hardcode a path.

### Gates

`voxel_step_metric.py` is the only script with a pass/fail threshold:

- **K1 (edge density) ≥ 0.020** — below this the surface reads as smooth.
- **K2 (distinct colours per kilopixel) ≥ 18**.

The verdict is only valid on a crop that actually contains terrain. A crop with
sky in it scores edge density from cloud detail and will mislead; that mistake
was made once in this project (a native-vs-0.8 render-scale comparison used a
crop with too much sky and produced a meaningless SMOOTH verdict at both
scales). Calibration crops for the reference and for this sample's captures are
in the `--help` epilog.

`frame_gap_metrics.py` deliberately has no pass/fail. It ranks gaps so the next
piece of work can be chosen from evidence; as of the last run the largest were
hue spread at distance (~17x the reference) and near-field hue spread.

## Capturing what these read

Host captures are correctness evidence only — llvmpipe, not the phone.

```sh
# landscape sample, settled, no HUD, fixed camera, one frame, then exit
MATTERWEAVE_LANDSCAPE_EYE=x,y,z,yaw[,pitch] MATTERWEAVE_LANDSCAPE_HUD=off \
MATTERWEAVE_LANDSCAPE_SHOT=/tmp/shot.ppm \
  xvfb-run -a target/debug/matterweave-explorer --landscape --clouds low \
  --smoke-frames 300 --save /tmp/w.json
```

On a device there is no command line, so the same settings come from
`files/landscape.txt` beside the save (`clouds low`, `eye x,y,z,yaw,pitch`,
`render-scale 0.8`, `shot /path`). See the landscape section of
[DEVELOPMENT.md](../../docs/DEVELOPMENT.md) for the full marker grammar and the
`adb`/`run-as` recipe.

`MATTERWEAVE_LANDSCAPE_FLORA=off|shadow` is the difference mode that separates
vegetation from terrain: a pixel that differs from the `shadow` frame is
vegetation, and one that differs from the `off` frame is vegetation or its
shadow.

## The corpus

The captures themselves are **not committed** — about 52 MB of PNGs, and the
reference half is third-party video frames. Regenerate the `ours` half with the
commands above; the reference half is frames from the MishMash micro-voxel
devlog (YouTube `xGkWWfO87no`), captured from the video at the framings recorded
in [fidelity-spec.md](fidelity-spec.md).

`fidelity-spec.md` holds the derived criteria and the reference measurements
behind them; `frame-gap-report.md` holds the ranked gap analysis from one run.
Both quote image names from that uncommitted corpus, so treat the numbers as the
record and re-derive the images.

## Rules

- A measurement needs its conditions: device, OS, driver, resolution, build
  configuration, commit, sun angle, cloud quality and HUD state, or it is not a
  measurement.
- Never infer phone behaviour from desktop or emulator numbers.
- `frame_ms` on a device is a *presented interval* and is vsync-capped. A value
  at the refresh ceiling proves the target is met, not that there is headroom.
- Compare like with like: the same crop, the same camera, the same cloud setting
  and the same screen state on a power measurement.
