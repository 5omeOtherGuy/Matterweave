"""Regression checks for clock boundaries, repeated histories and paired controls."""
import copy
import unittest

from analyze_wetland_pairs import (pair_members, pair_mismatches, percentile,
                                   presentation_intervals, proc_stat, process_cpu)


def surface(elapsed, stamps):
    return {'elapsed_s': elapsed, 'exit': 0,
            'raw': '16666667\n' + ''.join(f'1 {stamp} 1\n' for stamp in stamps)}


def stat(cpu=100, start=50, pid=9):
    fields = ['0'] * 22
    fields[0], fields[11], fields[12], fields[19] = 'S', str(cpu), '20', str(start)
    return f'{pid} (worker (renderer) thread) ' + ' '.join(fields)


def trial():
    return {'variant': 'candidate', 'capture_state': 'on',
            'scene': {'instances': 8324}, 'environment': {'resolution': '3168x1440'},
            'end_camera': {'yaw': 0., 'pitch': -.08, 'shadows': True},
            'fixture': {'sha256': 'fixture'},
            'apk': {'generator': 2, 'composition_hash': 'composition', 'apk_sha256': 'a' * 64},
            'end_eye': [46., 15.6, 66.],
            'pre_launch_gate': {'battery_c': 32., 'skin_c': 32.},
            'app_whole_capture': {'value_counts': {
                'gpu_prev_shadows': {'': 1, '1': 40},
                'gpu_prev_shadow_map_size': {'': 1, '1024': 40},
                'voxel_bodies_total': {'6': 41}}}}


class PresentationTests(unittest.TestCase):
    def test_duplicate_history_counts_once_and_excludes_warmup(self):
        samples = [surface(119., [10_000_000, 20_000_000]),
                   surface(120., [10_000_000, 20_000_000, 30_000_000, 40_000_000]),
                   surface(121., [20_000_000, 30_000_000, 40_000_000, 60_000_000]),
                   surface(240., [30_000_000, 40_000_000, 60_000_000, 90_000_000])]
        result = presentation_intervals(samples, 120., 240.)
        self.assertEqual(result['selected_timestamps'], 3)
        self.assertEqual(result['supported']['interval_count'], 2)
        self.assertEqual(result['supported']['mean_ms'], 15.)
        self.assertEqual(result['unsupported_gaps'], [])
        self.assertAlmostEqual(result['verified_interval_duration_fraction_of_selected_span'], 1.)

    def test_missing_history_is_not_invented_as_a_slow_frame(self):
        result = presentation_intervals([surface(120., [10, 20]), surface(121., [100, 110])], 120., 240.)
        self.assertEqual(result['supported']['interval_count'], 2)
        self.assertEqual(result['unsupported_gaps'], [{'from_ns': 20, 'to_ns': 100, 'duration_ms': .00008}])
        self.assertAlmostEqual(result['verified_interval_duration_fraction_of_selected_span'], .2)
        self.assertAlmostEqual(result['whole_selected_span_mean_upper_bound_ms'], .0001 / 3)

    def test_pending_and_zero_timestamps_are_excluded(self):
        result = presentation_intervals([surface(120., [0, 10, 20, 2**63-1])], 120., 240.)
        self.assertEqual(result['selected_timestamps'], 2)

    def test_invalid_dump_or_clock_is_rejected(self):
        for samples in ([surface(120., [10, 20]), surface(119., [20, 30])],
                        [dict(surface(120., [10, 20]), exit=1)],
                        [surface(float('nan'), [10, 20])]):
            with self.subTest(samples=samples), self.assertRaises(ValueError):
                presentation_intervals(samples, 120., 240.)

    def test_type7_quantile_does_not_drop_outliers(self):
        self.assertEqual(percentile([1., 2., 3., 100.], .5), 2.5)
        self.assertAlmostEqual(percentile([1., 2., 3., 100.], .95), 85.45)


class ProcessTests(unittest.TestCase):
    def record(self):
        return {'clk_tck': 100, 'samples': [
            {'label': 'measure_start', 'raw': stat(), 'host_begin_s': 119., 'host_end_s': 121.},
            {'label': 'measure_end', 'raw': stat(cpu=6100), 'host_begin_s': 239., 'host_end_s': 241.}]}

    def test_comm_with_nested_parentheses_and_midpoint_clock(self):
        self.assertEqual(proc_stat(stat()), {'pid': 9, 'start_ticks': 50, 'cpu_ticks': 120})
        result = process_cpu(self.record())
        self.assertEqual(result['cpu_seconds'], 60.)
        self.assertEqual(result['logical_core_equivalent_utilization'], .5)

    def test_restart_and_reused_pid_are_rejected(self):
        for raw in [stat(cpu=6100, start=51), stat(cpu=6100, pid=10)]:
            record = self.record()
            record['samples'][1]['raw'] = raw
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                process_cpu(record)


def off_trial():
    """A capture-OFF trial: same build, no capture CSV at all."""
    row = trial()
    row.update(variant='same-build', capture_state='off', app_whole_capture=None)
    return row


class CaptureStatePairTests(unittest.TestCase):
    def test_build_pair_still_compares_variants(self):
        a, b = trial(), trial()
        a['variant'], b['variant'] = 'reference', 'candidate'
        self.assertEqual(pair_members([a, b]), ('candidate_minus_reference', a, b))

    def test_same_build_pair_compares_capture_states(self):
        off = off_trial()
        on = trial()
        on['variant'] = 'same-build'
        self.assertEqual(pair_members([on, off]), ('capture_on_minus_off', off, on))

    def test_two_states_of_two_builds_is_not_a_pair(self):
        off = off_trial()
        on = trial()
        on['variant'] = 'other-build'
        self.assertIsNone(pair_members([on, off]))
        self.assertIsNone(pair_members([trial()]))

    def test_absent_off_capture_is_not_compared_against_the_on_csv(self):
        on = trial()
        on['variant'] = 'same-build'
        self.assertEqual(pair_mismatches(off_trial(), on), [])

    def test_capture_pair_must_share_one_build(self):
        on = trial()
        on['variant'] = 'same-build'
        on['apk'] = dict(on['apk'], apk_sha256='b' * 64)
        self.assertIn('capture on/off pair does not share one build',
                      pair_mismatches(off_trial(), on))


class MatchTests(unittest.TestCase):
    def test_same_controls_allow_comparison(self):
        self.assertEqual(pair_mismatches(trial(), trial()), [])

    def test_quality_camera_content_or_fixture_change_rejects_pair(self):
        a = trial()
        for key, value in [('scene', {}), ('environment', {}), ('end_camera', {}),
                           ('fixture', {'sha256': 'different'}),
                           ('apk', {'generator': 3, 'composition_hash': 'changed'}),
                           ('end_eye', [46., 15.62, 66.])]:
            b = copy.deepcopy(a)
            b[key] = value
            with self.subTest(key=key):
                self.assertTrue(pair_mismatches(a, b))
        for field in ['gpu_prev_shadows', 'gpu_prev_shadow_map_size', 'voxel_bodies_total']:
            b = copy.deepcopy(a)
            b['app_whole_capture']['value_counts'][field]['different'] = 1
            with self.subTest(field=field):
                self.assertTrue(pair_mismatches(a, b))

    def test_mutual_temperature_match_required_even_if_shared_reference_matches(self):
        a, b = trial(), trial()
        a['pre_launch_gate']['battery_c'] = 31.1
        b['pre_launch_gate']['battery_c'] = 32.9
        self.assertTrue(pair_mismatches(a, b))


if __name__ == '__main__':
    unittest.main()
