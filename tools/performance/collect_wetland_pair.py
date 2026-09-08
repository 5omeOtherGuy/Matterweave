#!/usr/bin/env python3
"""Supervisor for a matched 3-pair full-wetland candidate/reference phone comparison.

This script only *collects* evidence for a paired optimization experiment. It
certifies nothing: it is not proof of a speedup, it makes no strict
GPU/compositor join, and it never derives a verdict. Analysis is the lead's.

What one trial does, in order:

  1. Install the requested frozen APK and verify the *installed* file hash.
  2. Record an idle readiness window with the authoritative validator
     (tools/performance/validate_conditions.validate_readiness): >= 5 samples,
     >= 120 s span, gaps <= 35 s, battery spread <= 1 C, skin spread <= 2 C,
     unplugged, non-mock, thermal status 0. Those invariants are never relaxed
     or re-implemented here. Every observation is retained; at most
     --max-observations observations are taken before failing closed.
     The FIRST trial's accepted window becomes the thermal matching reference
     for every later trial (battery within 1 C, skin within 2 C at start).
  3. Preflight the app's private files: no unrequested `profile-frames.txt`,
     no `detail-gallery.txt` (rejected, never silently overridden), and the
     base `wetland-session.json` actually invalid for the generator under test
     so that `SavedWetland::load_recovering` selects a recovery slot.
  4. Write ONE worker-owned fixture at
     `files/wetland-session.json.recovery-127.json` (highest recovery slot, so
     it wins over the user's existing lower-numbered recovery) and verify the
     bytes. User `world.json` and all other saves are never read for backup,
     never written and never restored.
  5. Launch, tap through the normal chooser, wait (bounded) for the real
     "WETLAND LOADED" log line, screenshot, then sample the app's actual
     SurfaceFlinger layer for warmup + measurement while gating on foreground
     and unplugged state and recording health every 30 s.
  6. Press HOME so the app saves and flushes its capture, force-stop, then pull
     the saved session and the frame-profile capture and check the scene really
     is the wetland (6 frozen bodies, non-zero cells).

Failures are fail-closed: artifacts and logs of a failed trial are preserved,
cleanup still runs, and no partial trial is presented as a result.

Only the files this script created are removed at final cleanup.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_conditions import (  # noqa: E402  (path set above, stdlib-only module)
    parse_battery_state,
    parse_thermal_state,
    validate_readiness,
)

PACKAGE = "dev.matterweave.explorer"
ACTIVITY = "android.app.NativeActivity"
FIXTURE_REMOTE_NAME = "wetland-session.json.recovery-127.json"
BASE_SAVE_NAME = "wetland-session.json"
PROFILE_REQUEST_NAME = "profile-frames.txt"
GALLERY_MARKER_NAME = "detail-gallery.txt"
PROFILE_ROWS = 100000
EXPECTED_GENERATOR = 2
EXPECTED_BODIES = 6
MAX_BATTERY_DELTA_C = 1.0
MAX_SKIN_DELTA_C = 2.0
LOADED_RE = re.compile(r"WETLAND LOADED: (\d+) cells / (\d+) placed objects")
LAYER_RE = re.compile(
    r"^RequestedLayerState\{(" + re.escape(PACKAGE) + r"/" + re.escape(ACTIVITY)
    + r"#\d+)(?:\s|\})"
)
MIN_SCREEN_OFF_TIMEOUT_MS = 300_000


class TrialError(RuntimeError):
    """A trial could not be collected under the required conditions."""


# --------------------------------------------------------------------------
# Pure helpers (unit-testable without a phone)
# --------------------------------------------------------------------------

def trial_plan(pairs):
    """Return the ordered trial list, alternating within-pair order AB/BA/AB."""
    if not isinstance(pairs, int) or isinstance(pairs, bool) or pairs < 1:
        raise ValueError("pairs must be a positive integer")
    plan = []
    for pair in range(1, pairs + 1):
        order = ("reference", "candidate") if pair % 2 else ("candidate", "reference")
        for position, variant in enumerate(order, start=1):
            plan.append({
                "trial": len(plan) + 1,
                "pair": pair,
                "position": position,
                "variant": variant,
                "name": f"pair{pair}-{position}-{variant}",
            })
    return plan


def parse_wetland_loaded(text):
    """Return {'cells','instances'} from the app's real load line, else None."""
    match = None
    for line in text.splitlines():
        found = LOADED_RE.search(line)
        if found:
            match = found
    if match is None:
        return None
    return {"cells": int(match.group(1)), "instances": int(match.group(2))}


def check_fixture(raw):
    """Validate the lead-provided benchmark session bytes; raise ValueError."""
    if len(raw) > 2 * 1024 * 1024:
        raise ValueError("fixture exceeds the app's 2 MiB save limit")
    try:
        save = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ValueError(f"fixture is not valid JSON: {error}") from None
    if not isinstance(save, dict):
        raise ValueError("fixture must be a JSON object")
    if save.get("version") != 1:
        raise ValueError(f"fixture version must be 1, got {save.get('version')!r}")
    if save.get("generator") != EXPECTED_GENERATOR:
        raise ValueError(
            f"fixture generator must be {EXPECTED_GENERATOR}, got {save.get('generator')!r}")
    if not isinstance(save.get("seed"), int) or isinstance(save.get("seed"), bool):
        raise ValueError("fixture seed must be an integer")
    if not isinstance(save.get("edits"), list):
        raise ValueError("fixture edits must be a list")
    physics = save.get("physics")
    if not isinstance(physics, dict) or physics.get("version") != 1:
        raise ValueError("fixture physics snapshot must be version 1")
    bodies = physics.get("bodies")
    if not isinstance(bodies, list) or len(bodies) != EXPECTED_BODIES:
        raise ValueError(
            f"fixture must hold exactly {EXPECTED_BODIES} frozen bodies, got "
            f"{len(bodies) if isinstance(bodies, list) else type(bodies).__name__}")
    eye = physics.get("eye")
    if not isinstance(eye, list) or len(eye) != 3 or not all(
            isinstance(n, (int, float)) and not isinstance(n, bool) for n in eye):
        raise ValueError("fixture eye must be three numbers")
    for key in ("yaw", "pitch"):
        if not isinstance(save.get(key), (int, float)) or isinstance(save.get(key), bool):
            raise ValueError(f"fixture {key} must be a number")
    if not isinstance(save.get("shadows"), bool):
        raise ValueError("fixture shadows must be a boolean")
    return {
        "seed": save["seed"],
        "generator": save["generator"],
        "edit_count": len(save["edits"]),
        "body_count": len(bodies),
        "eye": eye,
        "yaw": save["yaw"],
        "pitch": save["pitch"],
        "shadows": save["shadows"],
        "sha256": hashlib.sha256(raw).hexdigest(),
    }


def base_save_is_invalid_for(raw, generator):
    """True when the app's base save cannot load for `generator`.

    Read-only reasoning about bytes the script never writes.
    """
    if raw is None:
        return True
    try:
        save = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return True
    if not isinstance(save, dict):
        return True
    return save.get("generator") != generator or save.get("version") != 1


def thermal_deltas(row, reference):
    """Absolute battery/skin deltas between two raw readiness rows."""
    battery = abs(parse_battery_state(row["data"]["battery"])["temp_c"]
                  - parse_battery_state(reference["data"]["battery"])["temp_c"])
    skin = abs(parse_thermal_state(row["data"]["thermalservice"])["skin_c"]
               - parse_thermal_state(reference["data"]["thermalservice"])["skin_c"])
    return {"battery_delta_c": battery, "skin_delta_c": skin}


def check_thermal_match(row, reference):
    """Raise ValueError unless the start state matches the reference trial."""
    deltas = thermal_deltas(row, reference)
    if (deltas["battery_delta_c"] > MAX_BATTERY_DELTA_C
            or deltas["skin_delta_c"] > MAX_SKIN_DELTA_C):
        raise ValueError(
            "pair start mismatch vs reference trial: battery "
            f"{deltas['battery_delta_c']:.1f} C (max {MAX_BATTERY_DELTA_C}), skin "
            f"{deltas['skin_delta_c']:.1f} C (max {MAX_SKIN_DELTA_C})")
    return deltas


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_apk_metadata(apk_path, variant):
    """Read the frozen build manifest beside an APK, if present."""
    manifest = apk_path.with_name(apk_path.stem + "-build.json")
    if not manifest.exists():
        raise TrialError(f"missing frozen build manifest {manifest}")
    data = json.loads(manifest.read_text())
    if data.get("variant") != variant:
        raise TrialError(f"{manifest} declares variant {data.get('variant')!r}, expected {variant}")
    for key in ("apk_sha256", "source_commit", "generator", "composition_hash"):
        if not data.get(key):
            raise TrialError(f"{manifest} is missing {key}")
    actual = sha256_file(apk_path)
    if actual != data["apk_sha256"]:
        raise TrialError(f"{apk_path} sha256 {actual} != manifest {data['apk_sha256']}")
    return {"manifest": str(manifest), "manifest_sha256": sha256_file(manifest), **data}


# --------------------------------------------------------------------------
# Device access (every call bounded)
# --------------------------------------------------------------------------

class Device:
    def __init__(self, adb, serial, package):
        self.base = [str(adb)] + (["-s", serial] if serial else [])
        self.package = package

    def run(self, args, timeout, check=True, binary=False, stdin=None):
        result = subprocess.run(
            self.base + list(args), capture_output=True,
            text=not binary, input=stdin, timeout=timeout)
        if check and result.returncode != 0:
            detail = result.stderr if not binary else result.stderr.decode("utf-8", "replace")
            raise TrialError(f"adb {' '.join(args)} failed ({result.returncode}): {detail.strip()[:400]}")
        return result

    def shell(self, *args, timeout=20, check=True):
        return self.run(["shell", *args], timeout=timeout, check=check).stdout

    def run_as(self, *args, timeout=20, check=True):
        return self.shell("run-as", self.package, *args, timeout=timeout, check=check)

    def pull_private(self, name, timeout=60):
        """Return the exact bytes of an app-private file, or None if absent."""
        result = self.run(["exec-out", "run-as", self.package, "cat", "files/" + name],
                          timeout=timeout, check=False, binary=True)
        return result.stdout if result.returncode == 0 else None

    def push_private(self, name, data, timeout=60):
        self.run(["shell", f'run-as {self.package} sh -c "cat > files/{name}"'],
                 timeout=timeout, binary=True, stdin=data)

    def list_files(self, timeout=20):
        return set(self.run_as("ls", "files", timeout=timeout, check=False).split())

    def screenshot(self, path, timeout=60):
        result = self.run(["exec-out", "screencap", "-p"], timeout=timeout, binary=True)
        path.write_bytes(result.stdout)

    def app_layer(self, attempts=20):
        choices = []
        for _ in range(attempts):
            listing = self.shell("dumpsys", "SurfaceFlinger", "--list", timeout=20)
            choices = [m.group(1) for line in listing.splitlines()
                       if (m := LAYER_RE.match(line.strip()))]
            if len(choices) == 1:
                return choices[0]
            time.sleep(0.5)
        raise TrialError(f"expected exactly one app layer, saw {choices!r}")

    def foreground(self):
        activity = self.shell("dumpsys", "activity", "activities", timeout=25)
        return any("topResumedActivity=" in line and (self.package + "/") in line
                   for line in activity.splitlines())


# --------------------------------------------------------------------------
# Trial stages
# --------------------------------------------------------------------------

def install_and_verify(device, apk_path, expected_sha, out, args):
    device.run(["install", "-r", "-d", str(apk_path)], timeout=args.install_timeout)
    path_line = device.shell("pm", "path", device.package, timeout=30).strip()
    if not path_line.startswith("package:") or "\n" in path_line:
        raise TrialError(f"unexpected pm path output: {path_line!r}")
    remote = path_line.removeprefix("package:").strip()
    if not re.fullmatch(r"[A-Za-z0-9_./=+-]+", remote):
        raise TrialError(f"refusing unusual apk path {remote!r}")
    installed = device.shell("sha256sum", remote, timeout=120).split()
    if not installed:
        raise TrialError("could not hash the installed apk")
    record = {"apk": str(apk_path), "remote_path": remote,
              "installed_sha256": installed[0], "expected_sha256": expected_sha}
    (out / "installed-apk.json").write_text(json.dumps(record, indent=2) + "\n")
    if installed[0] != expected_sha:
        raise TrialError("installed apk hash does not match the frozen build; "
                         "refusing to benchmark a mislabeled build")
    return record


def record_idle_window(device, out, reference_row, args):
    """Take bounded idle observations until readiness AND pair matching pass."""
    device.shell("am", "force-stop", device.package, timeout=30)
    rows = []
    start = time.monotonic()
    observations = out / "observations.jsonl"
    problems = []
    for index in range(args.max_observations):
        time.sleep(max(0.0, start + index * args.sample_interval - time.monotonic()))
        row = {"observation": index,
               "elapsed_s": time.monotonic() - start,
               "wall_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "data": {service: device.shell("dumpsys", service, timeout=25)
                        for service in ("battery", "thermalservice")}}
        rows.append(row)
        with observations.open("a") as handle:
            handle.write(json.dumps(row) + "\n")
        try:
            summary = validate_readiness(rows[-5:])
            deltas = check_thermal_match(row, reference_row) if reference_row else None
        except ValueError as error:
            problems.append({"observation": index, "reason": str(error)})
            print(f"  observation {index}: {error}", flush=True)
            continue
        summary["pair_start"] = deltas
        summary["observations_taken"] = len(rows)
        (out / "readiness.json").write_text(json.dumps(summary, indent=2) + "\n")
        (out / "readiness-rejections.json").write_text(json.dumps(problems, indent=2) + "\n")
        return summary, rows[-5:]
    (out / "readiness-rejections.json").write_text(json.dumps(problems, indent=2) + "\n")
    raise TrialError(
        f"no matched stable idle window within {args.max_observations} observations; "
        "no app capture launched")


def preflight_files(device, out, fixture_bytes, first_trial):
    """Check app-private state before writing the single worker-owned fixture."""
    before = device.list_files()
    if PROFILE_REQUEST_NAME in before:
        raise TrialError("an unrequested profile capture request already exists; "
                         "not consuming another worker's request")
    if GALLERY_MARKER_NAME in before:
        raise TrialError("detail-gallery.txt is present; rejecting rather than "
                         "silently overriding the device's gallery configuration")
    base = device.pull_private(BASE_SAVE_NAME)
    if not base_save_is_invalid_for(base, EXPECTED_GENERATOR):
        raise TrialError(
            f"base {BASE_SAVE_NAME} is valid for generator {EXPECTED_GENERATOR}; the "
            "recovery fixture would NOT be selected and the user's save must not be touched")
    existing = device.pull_private(FIXTURE_REMOTE_NAME)
    if first_trial and existing is not None:
        raise TrialError(f"files/{FIXTURE_REMOTE_NAME} already exists; refusing to "
                         "overwrite a slot this run does not own")
    device.push_private(FIXTURE_REMOTE_NAME, fixture_bytes)
    written = device.pull_private(FIXTURE_REMOTE_NAME)
    if written != fixture_bytes:
        raise TrialError("fixture bytes on device differ from the provided fixture")
    state = {
        "files_before": sorted(before),
        "base_save_present": base is not None,
        "base_save_sha256": hashlib.sha256(base).hexdigest() if base is not None else None,
        "base_save_invalid_for_generator": EXPECTED_GENERATOR,
        "fixture_remote": "files/" + FIXTURE_REMOTE_NAME,
        "fixture_sha256": hashlib.sha256(fixture_bytes).hexdigest(),
        "fixture_preexisting": existing is not None,
        "note": "user world.json and other saves were never read for backup, "
                "written, or restored",
    }
    (out / "files-preflight.json").write_text(json.dumps(state, indent=2) + "\n")
    return before, state


def record_environment(device, out):
    settings = {}
    for namespace, key in (("system", "screen_brightness"), ("system", "screen_brightness_mode"),
                           ("system", "screen_off_timeout"), ("system", "peak_refresh_rate"),
                           ("system", "min_refresh_rate")):
        settings[key] = device.shell("settings", "get", namespace, key, timeout=20,
                                     check=False).strip()
    props = {}
    for prop in ("ro.product.model", "ro.product.device", "ro.build.fingerprint",
                 "ro.build.version.release", "ro.build.version.sdk"):
        props[prop] = device.shell("getprop", prop, timeout=20, check=False).strip()
    (out / "settings.json").write_text(json.dumps(settings, indent=2) + "\n")
    (out / "device-props.json").write_text(json.dumps(props, indent=2) + "\n")
    for name, cmd in (("display", ("dumpsys", "display")),
                      ("window", ("dumpsys", "window", "displays")),
                      ("battery-start", ("dumpsys", "battery")),
                      ("thermal-start", ("dumpsys", "thermalservice"))):
        (out / (name + ".txt")).write_text(device.shell(*cmd, timeout=30, check=False))
    timeout_ms = settings.get("screen_off_timeout", "")
    adequate = timeout_ms.isdecimal() and int(timeout_ms) >= MIN_SCREEN_OFF_TIMEOUT_MS
    if not adequate:
        raise TrialError(
            f"screen_off_timeout={timeout_ms!r} is below {MIN_SCREEN_OFF_TIMEOUT_MS} ms; "
            "set it on the device yourself - this script never changes display settings")
    return {"settings": settings, "props": props}


def enter_wetland(device, out, args):
    """Launch, tap through the chooser and wait for the app's real load line."""
    (out / "launch.txt").write_text(
        device.shell("am", "start", "-W", "-n", f"{device.package}/{ACTIVITY}", timeout=60))
    deadline = time.monotonic() + args.launch_timeout
    pid = ""
    while time.monotonic() < deadline:
        pid = device.shell("pidof", device.package, timeout=20, check=False).strip()
        if pid.isdecimal():
            break
        time.sleep(1)
    if not pid.isdecimal():
        raise TrialError("app process did not start")
    log_file = (out / "app.log").open("w")
    logcat = subprocess.Popen(
        device.base + ["logcat", "--pid=" + pid, "-T", "1", "-s", "Matterweave:I"],
        stdout=log_file, stderr=subprocess.STDOUT)
    try:
        time.sleep(args.chooser_delay)
        device.screenshot(out / "chooser.png")
        x, y = args.tap
        device.shell("input", "tap", str(x), str(y), timeout=20)
        (out / "tap.json").write_text(json.dumps({"x": x, "y": y, "pid": pid}) + "\n")
        loaded = None
        deadline = time.monotonic() + args.load_timeout
        while time.monotonic() < deadline:
            time.sleep(2)
            log_file.flush()
            loaded = parse_wetland_loaded((out / "app.log").read_text(errors="replace"))
            if loaded:
                break
        if not loaded:
            raise TrialError("app never reported WETLAND LOADED; this is not the "
                             "wetland scene - refusing to measure it")
        if loaded["cells"] <= 0 or loaded["instances"] <= 0:
            raise TrialError(f"empty wetland scene reported: {loaded}")
        time.sleep(args.settle_delay)
        device.screenshot(out / "entered.png")
        if not device.foreground():
            raise TrialError("app is not the top resumed activity after entering")
        (out / "scene-loaded.json").write_text(json.dumps(loaded, indent=2) + "\n")
        return pid, loaded, logcat, log_file
    except BaseException:
        logcat.terminate()
        logcat.wait(timeout=10)
        log_file.close()
        raise


def collect_window(device, out, pid, layer, args):
    """Bounded in-process SurfaceFlinger/health collector (adapted from run-01).

    Records raw presentation history, health every 30 s with foreground and
    unplugged gates, and whole-process CPU ticks across the measurement window.
    No strict GPU/compositor join is claimed; host and device clocks are kept
    raw and separate.
    """
    total = args.warmup + args.measure
    clock_ticks = device.shell("getconf", "CLK_TCK", timeout=20).strip()
    if not clock_ticks.isdecimal():
        raise TrialError(f"unexpected CLK_TCK {clock_ticks!r}")
    stat_path = f"/proc/{pid}/stat"  # whole thread group, not just the UI task
    start = time.monotonic()
    next_health = 0.0
    cpu = []
    (out / "runtime.json").write_text(json.dumps({
        "layer": layer, "pid": pid, "clk_tck": int(clock_ticks),
        "start_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "start_monotonic_s": start, "warmup_s": args.warmup,
        "measurement_s": args.measure}, indent=2) + "\n")

    def cpu_sample(label):
        begin = time.monotonic()
        raw = device.run_as("cat", stat_path, timeout=20)
        cpu.append({"label": label, "host_begin_s": begin,
                    "host_end_s": time.monotonic(), "raw": raw})

    cpu_sample("warmup_start")
    with (out / "surface-samples.jsonl").open("w") as samples, \
            (out / "health.jsonl").open("w") as health:
        measure_marked = False
        while True:
            elapsed = time.monotonic() - start
            if elapsed >= total:
                break
            if not measure_marked and elapsed >= args.warmup:
                cpu_sample("measure_start")
                measure_marked = True
            result = device.run(["shell", "dumpsys", "SurfaceFlinger", "--latency", layer],
                                timeout=20, check=False)
            samples.write(json.dumps({
                "elapsed_s": elapsed, "phase": "warmup" if elapsed < args.warmup else "measure",
                "wall_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "exit": result.returncode, "raw": result.stdout}) + "\n")
            samples.flush()
            rows = [line.split() for line in result.stdout.splitlines()[1:]]
            if result.returncode or not any(
                    len(row) == 3 and row[1] not in ("0", "9223372036854775807") for row in rows):
                raise TrialError("no valid presentation history for the app layer; "
                                 "rejecting the capture instead of inferring performance")
            if elapsed >= next_health:
                data = {}
                for cmd in (("dumpsys", "battery"), ("dumpsys", "thermalservice"),
                            ("dumpsys", "meminfo", device.package),
                            ("dumpsys", "activity", "activities")):
                    data[cmd[1]] = device.shell(*cmd, timeout=30, check=False)
                health.write(json.dumps({"elapsed_s": elapsed, "data": data}) + "\n")
                health.flush()
                next_health = elapsed + 30
                if not any("topResumedActivity=" in line and (device.package + "/") in line
                           for line in data["activity"].splitlines()):
                    raise TrialError("app left the foreground; rejecting the capture")
                if any(f"{source} powered: true" in data["battery"]
                       for source in ("AC", "USB", "Wireless", "Dock")):
                    raise TrialError("external power detected; rejecting the unplugged capture")
            time.sleep(max(0.0, 0.5 - (time.monotonic() - start - elapsed)))
    if not measure_marked:
        raise TrialError("measurement window never started")
    cpu_sample("measure_end")
    (out / "process-stat.json").write_text(json.dumps({
        "clk_tck": int(clock_ticks), "samples": cpu,
        "note": "whole-process CPU ticks including native engine and background "
                "threads; independent approximate host window, no exact "
                "compositor/CPU join"}, indent=2) + "\n")
    return {"elapsed_s": time.monotonic() - start,
            "warmup_s": args.warmup, "measurement_s": args.measure,
            "surface_samples_sha256": sha256_file(out / "surface-samples.jsonl"),
            "health_sha256": sha256_file(out / "health.jsonl")}


def finish_and_pull(device, out, before, fixture_bytes, request_created):
    """HOME (so the app saves and flushes), force-stop, then pull artifacts."""
    device.shell("input", "keyevent", "KEYCODE_HOME", timeout=20, check=False)
    time.sleep(3)
    device.shell("am", "force-stop", device.package, timeout=30, check=False)
    time.sleep(1)
    after = device.list_files()
    captures = sorted(name for name in after - before if name.startswith("frame-profile"))
    pulled = []
    for name in captures:
        target = out / name
        if target.exists():
            raise TrialError(f"refusing to overwrite existing capture {target}")
        data = device.pull_private(name, timeout=180)
        if data is None:
            raise TrialError(f"could not read capture files/{name}")
        target.write_bytes(data)
        pulled.append({"name": name, "sha256": hashlib.sha256(data).hexdigest(),
                       "bytes": len(data),
                       "rows": max(0, data.count(b"\n") - 2)})
    session = device.pull_private(FIXTURE_REMOTE_NAME)
    if session is None:
        raise TrialError("worker fixture disappeared during the trial")
    (out / "session-after.json").write_bytes(session)
    scene = check_fixture(session)  # same invariants: gen 2, six bodies, finite pose
    consumed = PROFILE_REQUEST_NAME not in after
    if request_created and not consumed:
        device.run_as("rm", "-f", "files/" + PROFILE_REQUEST_NAME, timeout=20, check=False)
    result = {
        "captures": pulled,
        "capture_count": len(pulled),
        "profile_request_consumed": consumed,
        "session_after": scene,
        "session_after_sha256": hashlib.sha256(session).hexdigest(),
        "session_unchanged_from_fixture": session == fixture_bytes,
        "files_after": sorted(after),
    }
    if len(pulled) != 1:
        raise TrialError(f"expected exactly one new frame capture, got {[p['name'] for p in pulled]}")
    if not consumed:
        raise TrialError("the app never consumed the profile capture request")
    if scene["body_count"] != EXPECTED_BODIES:
        raise TrialError("saved session does not hold the six frozen wetland bodies")
    return result


def run_trial(device, spec, apk, out, fixture_bytes, fixture_info, reference_row, args):
    out.mkdir(parents=True, exist_ok=False)
    (out / "trial-request.json").write_text(json.dumps({
        **spec, "apk": apk, "fixture": fixture_info,
        "purpose": args.purpose}, indent=2) + "\n")
    installed = install_and_verify(device, Path(apk["apk_path"]), apk["apk_sha256"], out, args)
    environment = record_environment(device, out)
    readiness, window = record_idle_window(device, out, reference_row, args)
    before, files_state = preflight_files(device, out, fixture_bytes, spec["trial"] == 1)
    request_created = False
    logcat = log_file = None
    completed = False
    loaded = layer = None
    try:
        device.run_as("sh", "-c", f"'echo {PROFILE_ROWS} > files/{PROFILE_REQUEST_NAME}'",
                      timeout=20)
        request_created = True
        pid, loaded, logcat, log_file = enter_wetland(device, out, args)
        layer = device.app_layer()
        collection = collect_window(device, out, pid, layer, args)
        completed = True
    finally:
        if logcat is not None:
            logcat.terminate()
            try:
                logcat.wait(timeout=15)
            except subprocess.TimeoutExpired:
                logcat.kill()
        if log_file is not None:
            log_file.close()
        if not completed:
            # Preserve the failed trial's artifacts; still leave the phone clean.
            device.shell("input", "keyevent", "KEYCODE_HOME", timeout=20, check=False)
            time.sleep(2)
            device.shell("am", "force-stop", device.package, timeout=30, check=False)
            if request_created:
                device.run_as("rm", "-f", "files/" + PROFILE_REQUEST_NAME,
                              timeout=20, check=False)
    finished = finish_and_pull(device, out, before, fixture_bytes, request_created)
    record = {
        **spec,
        "purpose": args.purpose,
        "apk": apk,
        "installed": installed,
        "fixture": fixture_info,
        "files": files_state,
        "environment": environment,
        "readiness": readiness,
        "scene_loaded": loaded,
        "layer": layer,
        "collection": collection,
        "result": finished,
        "claims": "raw collection only; no FPS, energy or speedup claim is made here",
    }
    (out / "trial.json").write_text(json.dumps(record, indent=2) + "\n")
    (out / "collection-complete.json").write_text(json.dumps({
        "trial": spec["trial"], "variant": spec["variant"],
        "completed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        **collection}, indent=2) + "\n")
    return record, window[-1]


def final_cleanup(device, out, fixture_bytes, log):
    """Remove ONLY files this run created; never touch other saves."""
    status = {"removed": [], "errors": []}
    try:
        current = device.pull_private(FIXTURE_REMOTE_NAME)
        if current is not None:
            device.run_as("rm", "-f", "files/" + FIXTURE_REMOTE_NAME, timeout=20, check=False)
            if device.pull_private(FIXTURE_REMOTE_NAME) is None:
                status["removed"].append("files/" + FIXTURE_REMOTE_NAME)
            else:
                status["errors"].append(f"could not remove files/{FIXTURE_REMOTE_NAME}")
        files = device.list_files()
        if PROFILE_REQUEST_NAME in files:
            device.run_as("rm", "-f", "files/" + PROFILE_REQUEST_NAME, timeout=20, check=False)
            status["removed"].append("files/" + PROFILE_REQUEST_NAME)
    except Exception as error:  # cleanup must never mask the original failure
        status["errors"].append(repr(error))
    status["note"] = ("only worker-created files were removed; user world.json, "
                      "wetland-session.json and other recovery slots untouched")
    (out / "cleanup.json").write_text(json.dumps(status, indent=2) + "\n")
    log(f"cleanup: removed={status['removed']} errors={status['errors']}")


def build_parser():
    parser = argparse.ArgumentParser(
        prog="collect_wetland_pair.py",
        description=__doc__.split("\n\n")[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Collection only. This script never decides which build is faster.")
    parser.add_argument("--adb", required=True, type=Path, help="path to the adb executable")
    parser.add_argument("--serial", required=True, help="adb device serial, e.g. 192.168.178.93:42373")
    parser.add_argument("--candidate-apk", required=True, type=Path)
    parser.add_argument("--reference-apk", required=True, type=Path)
    parser.add_argument("--fixture", required=True, type=Path,
                        help="lead-provided wetland session JSON written to the recovery slot")
    parser.add_argument("--out", required=True, type=Path, help="fresh output directory")
    parser.add_argument("--package", default=PACKAGE)
    parser.add_argument("--pairs", type=int, default=3, help="matched pairs (order alternates AB/BA/AB)")
    parser.add_argument("--warmup", type=float, default=120.0)
    parser.add_argument("--measure", type=float, default=120.0)
    parser.add_argument("--max-observations", type=int, default=31,
                        help="idle observations per trial before failing closed")
    parser.add_argument("--sample-interval", type=float, default=30.05)
    parser.add_argument("--tap", type=int, nargs=2, default=(850, 780), metavar=("X", "Y"),
                        help="chooser entry tap in device pixels")
    parser.add_argument("--chooser-delay", type=float, default=6.0)
    parser.add_argument("--launch-timeout", type=float, default=60.0)
    parser.add_argument("--load-timeout", type=float, default=180.0)
    parser.add_argument("--settle-delay", type=float, default=5.0)
    parser.add_argument("--install-timeout", type=float, default=600.0)
    parser.add_argument("--purpose", default="paired optimization experiment; "
                        "not proof of a speedup")
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    if args.pairs < 1 or args.warmup <= 0 or args.measure <= 0 or args.max_observations < 1:
        raise SystemExit("invalid bounds: pairs/warmup/measure/max-observations must be positive")
    if not args.adb.exists():
        raise SystemExit(f"adb not found at {args.adb}")
    args.out.mkdir(parents=True, exist_ok=False)
    log_path = args.out / "run.log"

    def log(message):
        line = f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {message}"
        print(line, flush=True)
        with log_path.open("a") as handle:
            handle.write(line + "\n")

    fixture_bytes = args.fixture.read_bytes()
    fixture_info = {**check_fixture(fixture_bytes), "source": str(args.fixture)}
    apks = {
        "candidate": {**expected_apk_metadata(args.candidate_apk, "candidate"),
                      "apk_path": str(args.candidate_apk)},
        "reference": {**expected_apk_metadata(args.reference_apk, "reference"),
                      "apk_path": str(args.reference_apk)},
    }
    if apks["candidate"]["composition_hash"] != apks["reference"]["composition_hash"]:
        raise SystemExit("candidate and reference declare different scene composition hashes; "
                         "the comparison would not be matched")
    if apks["candidate"]["generator"] != EXPECTED_GENERATOR:
        raise SystemExit(f"builds are not generator {EXPECTED_GENERATOR}")
    plan = trial_plan(args.pairs)
    (args.out / "manifest.json").write_text(json.dumps({
        "purpose": args.purpose,
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "adb": str(args.adb), "serial": args.serial, "package": args.package,
        "apks": apks, "fixture": fixture_info, "plan": plan,
        "warmup_s": args.warmup, "measurement_s": args.measure,
        "readiness_validator": "tools/performance/validate_conditions.validate_readiness",
        "readiness_validator_sha256": sha256_file(Path(__file__).parent / "validate_conditions.py"),
        "supervisor_sha256": sha256_file(Path(__file__)),
        "claims": "collection only; analysis and any comparison are the lead's",
    }, indent=2) + "\n")
    device = Device(args.adb, args.serial, args.package)
    reference_row = None
    records = []
    try:
        for spec in plan:
            log(f"trial {spec['trial']}/{len(plan)}: {spec['name']}")
            record, last_row = run_trial(
                device, spec, apks[spec["variant"]], args.out / spec["name"],
                fixture_bytes, fixture_info, reference_row, args)
            records.append(record)
            if reference_row is None:
                reference_row = last_row
                log("trial 1 idle window is now the thermal matching reference "
                    "(battery +-1 C, skin +-2 C for all later trials)")
            log(f"trial {spec['trial']} collected: {record['collection']}")
    except BaseException as error:
        (args.out / "failure.json").write_text(json.dumps({
            "error": repr(error), "completed_trials": [r["name"] for r in records],
            "note": "failed trial artifacts preserved; no partial result is a measurement",
        }, indent=2) + "\n")
        log(f"FAILED: {error!r}")
        final_cleanup(device, args.out, fixture_bytes, log)
        raise
    final_cleanup(device, args.out, fixture_bytes, log)
    (args.out / "run-complete.json").write_text(json.dumps({
        "finished_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "trials": [{"name": r["name"], "variant": r["variant"], "pair": r["pair"],
                    "collection": r["collection"],
                    "captures": r["result"]["captures"]} for r in records],
        "claims": "raw paired collection complete; no analysis, verdict or speedup claim",
    }, indent=2) + "\n")
    log(f"collection complete: {len(records)} trials in {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
