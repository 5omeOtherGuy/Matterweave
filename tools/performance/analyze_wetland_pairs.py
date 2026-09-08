#!/usr/bin/env python3
"""Offline trial summaries. Frames are correlated observations, not replicates.

Presentation selection follows the previously executed v0.3 evidence method:
column2, globally deduplicated stamps, first-observation host window membership,
and consecutive endpoints co-observed in at least one actual dump. Missing
history is reported separately. App CSV has no absolute clock anchor, so its
statistics cover the entire capture, including startup and warmup.
"""
import argparse
import collections
import csv
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

from validate_frame_profile import validate_profile


def percentile(values, probability):
    """Linear interpolation at (n-1)*p, equivalent to common type-7 quantiles."""
    ordered = sorted(values)
    if not ordered:
        return None
    index = (len(ordered) - 1) * probability
    low, high = math.floor(index), math.ceil(index)
    return ordered[low] + (ordered[high] - ordered[low]) * (index - low)

def interval_stats(pairs):
    values = [(b - a) / 1e6 for a, b in pairs]
    if not values:
        return {"interval_count": 0}
    assert all(value > 0 for value in values)
    return {
        "interval_count": len(values),
        "total_interval_s": sum(b - a for a, b in pairs) / 1e9,
        "mean_ms": statistics.mean(values),
        "median_ms": statistics.median(values),
        "p95_ms": percentile(values, 0.95),
        "p99_ms": percentile(values, 0.99),
        "min_ms": min(values),
        "max_ms": max(values),
        "gaps_strictly_greater_than_ms": {
            str(threshold): {"count": sum(v > threshold for v in values),
                             "percent": 100 * sum(v > threshold for v in values) / len(values)}
            for threshold in [33.3, 50, 100]
        },
    }

def match_number(pattern, text):
    match = re.search(pattern, text, re.MULTILINE)
    return float(match.group(1)) if match else None

def health_point(sample):
    data = sample["data"]
    battery, thermal, memory = (data.get(name, "") for name in ["battery", "thermalservice", "meminfo"])
    current_hal = thermal.split("Current temperatures from HAL:", 1)
    current_hal = current_hal[1].split("Current cooling devices from HAL:", 1)[0] if len(current_hal) == 2 else ""
    sensors = {}
    for value, kind, name, status in re.findall(
        r"Temperature\{mValue=([^,]+), mType=(\d+), mName=([^,]+), mStatus=(\d+)\}", current_hal
    ):
        sensors[name] = {"value": float(value), "type": int(kind), "status": int(status)}
    cpus = [entry["value"] for entry in sensors.values() if entry["type"] == 0]
    gpus = [entry["value"] for entry in sensors.values() if entry["type"] == 1]
    temperature = match_number(r"^\s*temperature:\s*(\d+)", battery)
    return {
        "elapsed_s": sample["elapsed_s"],
        "battery_percent": match_number(r"^\s*level:\s*(\d+)", battery),
        "battery_temperature_c": temperature / 10 if temperature is not None else None,
        "usb_powered": "USB powered: true" in battery,
        "thermal_status": match_number(r"^Thermal Status:\s*(\d+)", thermal),
        "thermal_status_override": "IsStatusOverride: true" in thermal,
        "skin_current_hal_c": sensors.get("skin", {}).get("value"),
        "cpu_hottest_current_hal_c": max(cpus) if cpus else None,
        "gpu_hottest_current_hal_c": max(gpus) if gpus else None,
        "pss_reported_kb": match_number(r"TOTAL PSS:\s*(\d+)", memory),
        "rss_reported_kb": match_number(r"TOTAL RSS:\s*(\d+)", memory),
        "swap_pss_reported_kb": match_number(r"TOTAL SWAP PSS:\s*(\d+)", memory),
        "graphics_pss_reported_kb": match_number(r"^\s*Graphics:\s*(\d+)", memory),
        "pid": match_number(r"MEMINFO in pid (\d+)", memory),
        "current_hal_sensors": sensors,
    }

def summarize_health(points):
    metrics = {}
    for key in ["battery_percent", "battery_temperature_c", "thermal_status", "skin_current_hal_c",
                "cpu_hottest_current_hal_c", "gpu_hottest_current_hal_c", "pss_reported_kb",
                "rss_reported_kb", "swap_pss_reported_kb", "graphics_pss_reported_kb"]:
        present = [point for point in points if point[key] is not None]
        values = [point[key] for point in present]
        metrics[key] = {"samples": len(values), "missing": len(points) - len(values),
                        "early": values[0] if values else None, "late": values[-1] if values else None,
                        "min": min(values) if values else None, "max": max(values) if values else None,
                        "mean": statistics.mean(values) if values else None}
    return {"samples": len(points), "first_elapsed_s": points[0]["elapsed_s"],
            "last_elapsed_s": points[-1]["elapsed_s"], "metrics": metrics,
            "usb_powered_all_samples": all(p["usb_powered"] for p in points),
            "thermal_status_override_any_sample": any(p["thermal_status_override"] for p in points),
            "observed_pids": sorted({int(p["pid"]) for p in points if p["pid"] is not None}),
            "thermal_status_counts": dict(collections.Counter(str(p["thermal_status"]) for p in points))}

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def presentation_intervals(samples, start, end):
    first, coobserved, last_elapsed = {}, set(), -1.0
    headers = collections.Counter()
    for sample in samples:
        elapsed = sample['elapsed_s']
        if not math.isfinite(elapsed) or elapsed <= last_elapsed or sample['exit'] != 0:
            raise ValueError('invalid or nonmonotonic compositor observation')
        last_elapsed = elapsed
        lines = sample['raw'].splitlines()
        headers[str(int(lines[0]))] += 1
        stamps = set()
        for line in lines[1:]:
            if not line.strip():
                continue
            row = list(map(int, line.split()))
            if len(row) != 3:
                raise ValueError('malformed compositor timestamp row')
            if 0 < row[1] < 2**63 - 1:
                stamps.add(row[1])
        if not stamps:
            raise ValueError('empty presentation history')
        for stamp in stamps:
            first.setdefault(stamp, elapsed)
        ordered = sorted(stamps)
        coobserved.update(zip(ordered, ordered[1:]))
    selected = sorted(stamp for stamp, elapsed in first.items() if start <= elapsed < end)
    if len(selected) < 2:
        raise ValueError('insufficient measurement presentation history')
    adjacent = list(zip(selected, selected[1:]))
    supported = [pair for pair in adjacent if pair in coobserved]
    gaps = [pair for pair in adjacent if pair not in coobserved]
    span = (selected[-1] - selected[0]) / 1e9
    stats = interval_stats(supported)
    values = [(b-a)/1e6 for a,b in supported]
    stats['iqr_ms'] = percentile(values, .75) - percentile(values, .25) if values else None
    return {'dumps': len(samples), 'selected_timestamps': len(selected),
            'selected_span_s': span, 'supported': stats,
            # Each unsupported gap contains at least one actual interval.
            # Hidden intermediate presentations only increase the denominator.
            'whole_selected_span_mean_upper_bound_ms': span * 1000 / len(adjacent),
            'mean_bound_note': 'Conservative mean bound over selected span, not an imputed frame distribution.',
            'unsupported_gaps': [{'from_ns': a, 'to_ns': b, 'duration_ms': (b-a)/1e6}
                                 for a,b in gaps],
            'verified_interval_duration_fraction_of_selected_span':
                sum(b-a for a,b in supported)/1e9/span,
            'header_period_ns_counts': dict(headers),
            'boundary_note': 'First-observation boundaries are approximate; selected span is not requested-window coverage.'}


def proc_stat(raw):
    # comm can contain spaces and parentheses. Fields following its final ')' are stable.
    prefix, tail = raw.rsplit(') ', 1)
    fields = tail.split()
    return {'pid': int(prefix.split('(', 1)[0].strip()),
            'start_ticks': int(fields[19]), 'cpu_ticks': int(fields[11]) + int(fields[12])}


def process_cpu(record):
    by_label = {s['label']: s for s in record['samples']}
    before, after = by_label['measure_start'], by_label['measure_end']
    a, b = proc_stat(before['raw']), proc_stat(after['raw'])
    if (a['pid'], a['start_ticks']) != (b['pid'], b['start_ticks']):
        raise ValueError('process identity changed during CPU observation')
    elapsed = ((after['host_begin_s'] + after['host_end_s']) -
               (before['host_begin_s'] + before['host_end_s'])) / 2
    ticks = b['cpu_ticks'] - a['cpu_ticks']
    if elapsed <= 0 or ticks < 0 or record['clk_tck'] <= 0:
        raise ValueError('invalid CPU interval')
    busy = ticks / record['clk_tck']
    return {'cpu_seconds': busy, 'host_midpoint_elapsed_seconds': elapsed,
            'logical_core_equivalent_utilization': busy/elapsed,
            'note': 'Whole process, independently timed approximate measurement window; not single-thread utilization or power.'}


def app_profile(path):
    valid = validate_profile(path)
    with path.open(newline='') as source:
        next(source)
        rows = list(csv.DictReader(source))
    fields = ['draw_interval_wall_ms', 'main_wall_ms', 'main_cpu_busy_ms',
              'physics_wall_ms', 'dynamic_mesh_build_wall_ms', 'dynamic_upload_wall_ms',
              'render_wall_ms', 'gpu_prev_render_ms', 'gpu_prev_shadow_ms']
    stats = {}
    for field in fields:
        values = [float(row[field]) for row in rows if row[field] != '']
        stats[field] = {'count': len(values), 'missing': len(rows)-len(values),
                        'mean': statistics.mean(values) if values else None,
                        'median': statistics.median(values) if values else None,
                        'p95': percentile(values, .95), 'p99': percentile(values, .99),
                        'iqr': percentile(values, .75)-percentile(values, .25) if values else None}
    flags = ['gpu_prev_shadows', 'gpu_prev_shadow_map_size', 'voxel_bodies_total',
             'voxel_bodies_active', 'voxel_bodies_sleeping', 'voxel_bodies_not_simulated']
    return {'validation': valid, 'scope': 'Entire capture includes startup/warmup; no exact host/compositor window join.',
            'distributions': stats,
            'value_counts': {key: dict(collections.Counter(row[key] for row in rows)) for key in flags},
            'dynamic_mesh_builds': sum(int(row['dynamic_mesh_builds']) for row in rows),
            'dynamic_mesh_uploads': sum(int(row['dynamic_mesh_uploads']) for row in rows)}


def analyze_trial(directory):
    trial = json.loads((directory/'trial.json').read_text())
    completion = json.loads((directory/'collection-complete.json').read_text())
    for file, key in [('surface-samples.jsonl','surface_samples_sha256'), ('health.jsonl','health_sha256')]:
        if digest(directory/file) != completion[key]:
            raise ValueError(f'{directory.name}: {file} hash mismatch')
    warm, measure = completion['warmup_s'], completion['measurement_s']
    if completion['elapsed_s'] < warm + measure:
        raise ValueError('incomplete measurement duration')
    health = read_jsonl(directory/'health.jsonl')
    for row in health:
        if not all(f'{power} powered: false' in row['data']['battery'] for power in ['AC','USB','Wireless','Dock']):
            raise ValueError('unplugged state not established')
        if not any('topResumedActivity=' in line and 'dev.matterweave.explorer/' in line
                   for line in row['data']['activity'].splitlines()):
            raise ValueError('foreground state not established')
    capture = trial['result']['captures'][0]
    if digest(directory/capture['name']) != capture['sha256']:
        raise ValueError('app capture hash mismatch')
    session = json.loads((directory/'session-after.json').read_text())
    if digest(directory/'session-after.json') != trial['result']['session_after_sha256']:
        raise ValueError('saved session hash mismatch')
    points = [health_point(row) for row in health]
    return {'name': directory.name, 'pair': trial['pair'], 'variant': trial['variant'],
            'input_trial_sha256': digest(directory/'trial.json'),
            'apk': trial['apk'], 'fixture': trial['fixture'], 'scene': trial['scene_loaded'],
            'environment': trial['environment'], 'pre_launch_gate': trial['pre_launch_gate'],
            'end_camera': {key: session[key] for key in ['yaw','pitch','shadows']},
            'end_eye': session['physics']['eye'],
            'presentation': presentation_intervals(read_jsonl(directory/'surface-samples.jsonl'), warm, warm+measure),
            'process_cpu': process_cpu(json.loads((directory/'process-stat.json').read_text())),
            'app_whole_capture': app_profile(directory/capture['name']),
            'health_all': summarize_health(points),
            'health_measurement': summarize_health([p for p in points if warm <= p['elapsed_s'] < warm+measure])}


def pair_mismatches(a, b):
    issues = []
    for key in ['scene', 'environment', 'end_camera']:
        if a[key] != b[key]: issues.append(f'{key} differs')
    if a['fixture']['sha256'] != b['fixture']['sha256']: issues.append('fixture differs')
    for key in ['generator','composition_hash']:
        if a['apk'][key] != b['apk'][key]: issues.append(f'{key} differs')
    eye_delta = math.dist(a['end_eye'], b['end_eye'])
    if eye_delta > .01: issues.append(f'end-eye difference {eye_delta}m exceeds0.01m')
    for field, bound in [('battery_c',1.0), ('skin_c',2.0)]:
        if abs(a['pre_launch_gate'][field]-b['pre_launch_gate'][field]) > bound:
            issues.append(f'paired {field} difference exceeds {bound}')
    for field in ['gpu_prev_shadows','gpu_prev_shadow_map_size','voxel_bodies_total']:
        sets = [set(row['app_whole_capture']['value_counts'][field]) - {''} for row in [a,b]]
        if sets[0] != sets[1] or len(sets[0]) != 1: issues.append(f'{field} differs or changed during capture')
    return issues


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((args.input/'manifest.json').read_text())
    trials, pending = [], []
    for planned in manifest['plan']:
        directory = args.input/planned['name']
        if not (directory/'collection-complete.json').exists():
            pending.append(planned['name']); continue
        trials.append(analyze_trial(directory))
    pairs = []
    for number in sorted({t['pair'] for t in trials}):
        members = {t['variant']: t for t in trials if t['pair'] == number}
        if set(members) != {'reference','candidate'}: continue
        a, b = members['reference'], members['candidate']
        issues = pair_mismatches(a,b)
        result = {'pair':number, 'mismatches':issues, 'image_review':'lead-owned, not established by metadata'}
        if not issues:
            result['candidate_minus_reference'] = {
                key: b['presentation']['supported'][key]-a['presentation']['supported'][key]
                for key in ['mean_ms','median_ms','p95_ms','p99_ms']}
            result['cpu_core_equivalent_delta'] = (b['process_cpu']['logical_core_equivalent_utilization']-
                                                    a['process_cpu']['logical_core_equivalent_utilization'])
        pairs.append(result)
    summary = {'analysis_sha256':digest(Path(__file__)), 'trials':trials, 'pending':pending, 'pairs':pairs,
               'limits':['Frames are correlated; trial pairs are the replication unit.',
                         'At most3 pairs: no reliable confidence interval or significance verdict is claimed.',
                         'No exact CSV/compositor clock join, ambient/case measurement, energy or sustained-run inference.',
                         'Visual equivalence requires separate lead review; metadata matching alone is insufficient.']}
    (args.out/'summary.json').write_text(json.dumps(summary,indent=2,allow_nan=False)+'\n')
    lines=['# Wetland paired capture summaries','', 'Descriptive evidence only; no optimization verdict.','',
           '| Trial | Mean ms | Median ms | p95 ms | Unsupported gaps | CPU core equivalent |',
           '| --- | ---: | ---: | ---: | ---: | ---: |']
    for t in trials:
        p=t['presentation']; s=p['supported']; lines.append(
            f"| {t['name']} | {s['mean_ms']:.3f} | {s['median_ms']:.3f} | {s['p95_ms']:.3f} | {len(p['unsupported_gaps'])} | {t['process_cpu']['logical_core_equivalent_utilization']:.3f} |")
    lines += ['',f'Completed trials: {len(trials)}. Pending: {len(pending)}.', '',
              'Paired differences and all matching checks are recorded in summary.json.', '', *summary['limits']]
    (args.out/'summary.md').write_text('\n'.join(lines)+'\n')
    print('\n'.join(lines))


if __name__ == '__main__':
    main()
