//! Opt-in normal-runtime route diagnostic. No simulation or persistence fixtures.
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Ground,
    Elevated,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    pub route: Route,
}
fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    if bytes.len() > 4096 {
        return Err("request exceeds 4KiB".into());
    }
    let request: Request = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if request.version != 1 {
        return Err("unsupported version".into());
    }
    Ok(request)
}
#[derive(Serialize)]
struct Report {
    request: Request,
    route_point_count: usize,
    next_waypoint: usize,
    max_index: Option<usize>,
    actual_eye: [f32; 3],
    start_eye: [f32; 3],
    start_unix_seconds: f64,
    end_unix_seconds: Option<f64>,
    duration_wall_seconds: f64,
    accumulated_frame_dt_seconds: f64,
    physics_step_count: u64,
    outcome: &'static str,
    reason: &'static str,
}
fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
struct Run {
    report: Report,
    settled: bool,
    point_since: f64,
}
impl Run {
    fn new(request: Request, route: &[[f32; 3]], eye: [f32; 3]) -> Self {
        Self {
            report: Report {
                request,
                route_point_count: route.len(),
                next_waypoint: 0,
                max_index: None,
                actual_eye: eye,
                start_eye: eye,
                start_unix_seconds: unix_seconds(),
                end_unix_seconds: None,
                duration_wall_seconds: 0.,
                accumulated_frame_dt_seconds: 0.,
                physics_step_count: 0,
                outcome: "RUNNING",
                reason: "settling",
            },
            settled: false,
            point_since: 0.,
        }
    }
    fn active(&self) -> bool {
        self.report.outcome == "RUNNING"
    }
    fn finish(&mut self, outcome: &'static str, reason: &'static str, wall: f64) {
        if !self.active() {
            return;
        }
        self.report.outcome = outcome;
        self.report.reason = reason;
        self.report.duration_wall_seconds = wall;
        self.report.end_unix_seconds = Some(unix_seconds());
    }
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        route: &[[f32; 3]],
        eye: [f32; 3],
        grounded: bool,
        wall: f64,
        dt: f32,
        steps: u32,
        respawn: bool,
    ) {
        if !self.active() {
            return;
        }
        self.report.actual_eye = eye;
        self.report.duration_wall_seconds = wall;
        self.report.accumulated_frame_dt_seconds += f64::from(dt);
        self.report.physics_step_count += u64::from(steps);
        let fail = if respawn {
            Some("normal app respawn")
        } else if !Vec3::from_array(eye).is_finite() || route.is_empty() {
            Some("invalid pose or empty route")
        } else if wall
            >= match self.report.request.route {
                Route::Ground => 600.,
                Route::Elevated => 120.,
            }
        {
            Some("total wall timeout")
        } else if !self.settled && horizontal(eye, route[0]).length() > 1. {
            Some("start more than 1m off route")
        } else if eye[1] < route[self.report.next_waypoint][1] + 1.7 - 4. {
            Some("fell below route")
        } else if !self.settled && !grounded && wall >= 3. {
            Some("not grounded after settle")
        } else if wall - self.point_since >= 20. {
            Some("waypoint timeout")
        } else {
            None
        };
        if let Some(reason) = fail {
            self.finish("FAIL", reason, wall);
            return;
        }
        if !self.settled {
            if !grounded {
                return;
            }
            self.settled = true;
            self.report.reason = "traversing";
        }
        if horizontal(eye, route[self.report.next_waypoint]).length() <= 0.2
            && grounded
            && (eye[1] - route[self.report.next_waypoint][1] - 1.7).abs() <= 1.
        {
            self.report.max_index = Some(self.report.next_waypoint);
            self.report.next_waypoint += 1;
            self.point_since = wall;
            if self.report.next_waypoint == route.len() {
                self.finish("PASS", "route complete", wall);
            }
        }
    }
    fn velocity(&self, route: &[[f32; 3]], eye: [f32; 3], speed: f32, dt: f32) -> Vec3 {
        if !self.active() || !self.settled {
            return Vec3::ZERO;
        }
        let delta = horizontal(eye, route[self.report.next_waypoint]);
        // Include possible accumulator remainder without changing its fixed timestep.
        delta.normalize_or_zero()
            * speed.min(
                delta.length()
                    / (dt + matterweave_physics::FIXED_DT).max(matterweave_physics::FIXED_DT),
            )
    }
}
fn horizontal(eye: [f32; 3], target: [f32; 3]) -> Vec3 {
    Vec3::new(target[0] - eye[0], 0., target[2] - eye[2])
}

pub struct Replay {
    run: Run,
    started: Instant,
    path: PathBuf,
    last_progress: f64,
    finalized: bool,
}
impl Replay {
    pub fn requested(
        directory: &Path,
        ground: &[[f32; 3]],
        elevated: &[[f32; 3]],
        eye: [f32; 3],
    ) -> Result<Option<Self>, String> {
        let request_path = directory.join("wetland-replay.json");
        let file = match File::open(&request_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let request = parse_request(&bytes)?;
        let route = match request.route {
            Route::Ground => ground,
            Route::Elevated => elevated,
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = directory.join(format!(
            "wetland-replay-result-{unique}-{}.json",
            std::process::id()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let replay = Self {
            run: Run::new(request, route, eye),
            started: Instant::now(),
            path,
            last_progress: 0.,
            finalized: false,
        };
        replay.persist()?;
        fs::remove_file(request_path).map_err(|e| e.to_string())?;
        Ok(Some(replay))
    }
    pub fn route(&self) -> Route {
        self.run.report.request.route
    }
    pub fn active(&self) -> bool {
        self.run.active()
    }
    pub fn velocity(&self, route: &[[f32; 3]], eye: [f32; 3], speed: f32, dt: f32) -> Vec3 {
        self.run.velocity(route, eye, speed, dt)
    }
    pub fn observe(
        &mut self,
        route: &[[f32; 3]],
        eye: [f32; 3],
        grounded: bool,
        dt: f32,
        steps: u32,
        respawn: bool,
    ) {
        self.run.observe(
            route,
            eye,
            grounded,
            self.started.elapsed().as_secs_f64(),
            dt,
            steps,
            respawn,
        );
        let wall = self.run.report.duration_wall_seconds;
        if self.active() && wall - self.last_progress >= 5. {
            self.last_progress = wall;
            self.write_progress();
        }
    }
    pub fn cancel(&mut self, reason: &'static str) {
        self.run
            .finish("CANCEL", reason, self.started.elapsed().as_secs_f64());
    }
    pub fn take_finished(&mut self) -> bool {
        if self.active() || self.finalized {
            return false;
        }
        self.finalized = true;
        self.write_progress();
        true
    }
    fn write_progress(&self) {
        log::info!(
            "WETLAND REPLAY {} next={}/{} eye={:?} wall={:.3}s {}",
            self.run.report.outcome,
            self.run.report.next_waypoint,
            self.run.report.route_point_count,
            self.run.report.actual_eye,
            self.run.report.duration_wall_seconds,
            self.run.report.reason
        );
        if let Err(e) = self.persist() {
            log::error!("Wetland replay report: {e}");
        }
    }
    fn persist(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.run.report).map_err(|e| e.to_string())?;
        let temp = self.path.with_extension("json.tmp");
        let mut file = File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(temp, &self.path).map_err(|e| e.to_string())?;
        File::open(self.path.parent().unwrap())
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        parse_request(br#"{"version":1,"route":"ground"}"#).unwrap()
    }
    const ROUTE: [[f32; 3]; 2] = [[0., 0., 0.], [2., 0., 0.]];
    #[test]
    fn files_consume_only_valid_and_keep_terminal_report() {
        let directory = std::env::temp_dir().join(format!(
            "wetland-replay-test-{}-{}",
            std::process::id(),
            unix_seconds()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("wetland-replay.json");
        fs::write(&path, b"invalid").unwrap();
        assert!(Replay::requested(&directory, &ROUTE, &ROUTE, [0., 1.7, 0.]).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
        fs::write(&path, br#"{"version":1,"route":"ground"}"#).unwrap();
        let mut replay = Replay::requested(&directory, &ROUTE, &ROUTE, [0., 1.7, 0.])
            .unwrap()
            .unwrap();
        assert!(!path.exists());
        assert!(Replay::requested(&directory, &ROUTE, &ROUTE, [0., 1.7, 0.])
            .unwrap()
            .is_none());
        replay.cancel("lifecycle suspended");
        assert!(replay.take_finished());
        assert!(!replay.take_finished());
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&replay.path).unwrap()).unwrap();
        assert_eq!(report["outcome"], "CANCEL");
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn strict_bounded_request() {
        assert!(parse_request(br#"{"version":1,"route":"elevated"}"#).is_ok());
        for bytes in [
            br#"{"version":2,"route":"ground"}"#.as_slice(),
            br#"{"version":1,"route":"fly"}"#,
            br#"{"version":1,"route":"ground","extra":0}"#,
            br#"{"version":1}"#,
            br#"{"version":1,"version":1,"route":"ground"}"#,
            b"null",
        ] {
            assert!(parse_request(bytes).is_err());
        }
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
        for (wall, eye, respawn, reason) in [
            (21., [0., 1.7, 0.], false, "waypoint timeout"),
            (1., [0., -5., 0.], false, "fell below route"),
            (1., [0., 1.7, 0.], true, "normal app respawn"),
        ] {
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
