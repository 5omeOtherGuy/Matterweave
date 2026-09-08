import copy
import unittest

from check_phone_route import validate_report


class RouteReportTests(unittest.TestCase):
    def setUp(self):
        self.points = [[0., 0., 0.], [1., 2., 3.]]
        self.report = {'request': {'version': 1, 'route': 'ground'},
                       'route_point_count': 2, 'outcome': 'PASS',
                       'next_waypoint': 2, 'max_index': 1,
                       'physics_step_count': 120, 'actual_eye': [1., 3.7, 3.]}

    def test_complete_actual_endpoint_is_required(self):
        validate_report(self.report, 'ground', self.points, 'PASS', False)
        for field, value in [('next_waypoint', 1), ('max_index', 0),
                             ('route_point_count', 3), ('physics_step_count', 0),
                             ('actual_eye', [1.3, 3.7, 3.]),
                             ('actual_eye', [1., 5., 3.])]:
            report = copy.deepcopy(self.report)
            report[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                validate_report(report, 'ground', self.points, 'PASS', False)

    def test_wrong_route_or_nonterminal_report_is_rejected(self):
        for field, value in [('request', {'version': 1, 'route': 'elevated'}),
                             ('outcome', 'RUNNING'), ('outcome', 'FAIL')]:
            report = copy.deepcopy(self.report)
            report[field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                validate_report(report, 'ground', self.points, 'PASS', False)

    def test_accidental_enter_cancellation_cannot_pass_interruption_check(self):
        self.report['outcome'] = 'CANCEL'
        self.report['next_waypoint'] = 1
        with self.assertRaises(AssertionError):
            validate_report(self.report, 'ground', self.points, 'CANCEL', False)
        validate_report(self.report, 'ground', self.points, 'CANCEL', True)


if __name__ == '__main__':
    unittest.main()
