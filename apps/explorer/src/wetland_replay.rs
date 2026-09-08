//! Opt-in normal-runtime route diagnostic. No simulation or persistence fixtures.
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request { parse_request(br#"{"version":1,"route":"ground"}"#).unwrap() }
    const ROUTE: [[f32; 3]; 2] = [[0., 0., 0.], [2., 0., 0.]];
    #[test]
    fn strict_bounded_request() {
        assert!(parse_request(br#"{"version":1,"route":"elevated"}"#).is_ok());
        for bytes in [br#"{"version":2,"route":"ground"}"#.as_slice(), br#"{"version":1,"route":"fly"}"#, br#"{"version":1,"route":"ground","extra":0}"#, br#"{"version":1}"#, br#"{"version":1,"version":1,"route":"ground"}"#, b"null"] { assert!(parse_request(bytes).is_err()); }
        assert!(parse_request(&vec![b' '; 4097]).is_err());
    }
    #[test]
    fn offroute_and_unsettled_fail() {
        let mut run = Run::new(request(), &ROUTE, [2., 1.7, 0.]);
        run.observe(&ROUTE, [2., 1.7, 0.], true, 0., 0., 0, false);
        assert_eq!(run.report.outcome, "FAIL");
        let mut run = Run::new(request(), &ROUTE, [0., 1.7, 0.]);
        run.observe(&ROUTE, [0., 1.7, 0.], false, 3.1, 0.1, 6, false);
        assert_eq!(run.report.reason, "not grounded after settle");
    }
    #[test]
    fn stall_fall_respawn_and_cancel_are_terminal() {
        for (wall, eye, respawn, reason) in [(21., [0., 1.7, 0.], false, "waypoint timeout"), (1., [0., -5., 0.], false, "fell below route"), (1., [0., 1.7, 0.], true, "normal app respawn")] {
            let mut run = Run::new(request(), &ROUTE, [0., 1.7, 0.]);
            run.observe(&ROUTE, [0., 1.7, 0.], true, 0., 0., 0, false);
            run.observe(&ROUTE, eye, true, wall, 0.1, 6, respawn);
            assert_eq!(run.report.reason, reason);
        }
        let mut run = Run::new(request(), &ROUTE, [0., 1.7, 0.]);
        run.finish("CANCEL", "input", 1.);
        run.observe(&ROUTE, [2., 1.7, 0.], true, 2., 0.1, 6, false);
        assert_eq!(run.report.outcome, "CANCEL");
    }
    #[test]
    fn completion_is_once_and_speed_is_bounded() {
        let mut run = Run::new(request(), &ROUTE, [0., 1.7, 0.]);
        run.observe(&ROUTE, [0., 1.7, 0.], true, 0., 0., 0, false);
        assert!(run.velocity(&ROUTE, [1.7, 1.7, 0.], 6., 0.1).length() <= 3.001);
        assert!(run.velocity(&ROUTE, [0., 1.7, 0.], 2., 0.1).length() <= 2.);
        run.observe(&ROUTE, [2., 1.7, 0.], true, 1., 0.1, 6, false);
        assert_eq!(run.report.outcome, "PASS");
        assert_eq!(run.report.next_waypoint, 2);
        run.observe(&ROUTE, [0., -10., 0.], true, 99., 0.1, 6, true);
        assert_eq!(run.report.duration_wall_seconds, 1.);
        assert_eq!(run.velocity(&ROUTE, [0.; 3], 6., 0.1), glam::Vec3::ZERO);
    }
}
