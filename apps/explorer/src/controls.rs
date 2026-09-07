use glam::{Mat4, Vec2, Vec3};
use std::collections::{HashMap, HashSet};
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Remove,
    Place,
    Save,
    Swap,
    Size,
    Grab,
    Throw,
    Break,
    Flight,
    Home,
    ResetObjects,
}
#[derive(Debug, Clone, Copy)]
enum Finger {
    Move { origin: Vec2, current: Vec2 },
    Look { previous: Vec2 },
    Up,
    Down,
    Button,
}
#[derive(Default)]
pub struct Controls {
    fingers: HashMap<u64, Finger>,
    pub keys: HashSet<KeyCode>,
    pub swapped: bool,
    pub large: bool,
    look_delta: Vec2,
    jump_pressed: bool,
    pub mouse_look: bool,
    pub cursor: Option<Vec2>,
}
impl Controls {
    pub fn clear(&mut self) {
        self.fingers.clear();
        self.keys.clear();
        self.look_delta = Vec2::ZERO;
        self.jump_pressed = false;
        self.mouse_look = false;
        self.cursor = None;
    }
    pub fn buttons(&self) -> [([f32; 4], &'static str, Action); 11] {
        [
            ([390., 520., 105., 58.], "REMOVE", Action::Remove),
            ([505., 520., 105., 58.], "PLACE", Action::Place),
            ([720., 20., 76., 38.], "SAVE", Action::Save),
            ([804., 20., 76., 38.], "SWAP", Action::Swap),
            ([888., 20., 92., 38.], "SIZE", Action::Size),
            ([282., 454., 138., 52.], "GRAB/DROP", Action::Grab),
            ([430., 454., 138., 52.], "THROW", Action::Throw),
            ([578., 454., 138., 52.], "BREAK", Action::Break),
            ([720., 68., 124., 40.], "WALK/FLY", Action::Flight),
            ([852., 68., 128., 40.], "HOME", Action::Home),
            (
                [720., 116., 260., 40.],
                "RESET OBJECTS",
                Action::ResetObjects,
            ),
        ]
    }
    pub fn move_zone(&self) -> [f32; 4] {
        let side = if self.large { 250. } else { 190. };
        [
            if self.swapped { 980. - side } else { 20. },
            580. - side,
            side,
            side,
        ]
    }
    pub fn elevation_zones(&self) -> [[f32; 4]; 2] {
        let x = if self.swapped { 25. } else { 905. };
        [[x, 390., 70., 48.], [x, 450., 70., 48.]]
    }
    pub fn start(&mut self, id: u64, p: Vec2) -> Option<Action> {
        for (rect, _, action) in self.buttons() {
            if contains(rect, p) {
                self.fingers.insert(id, Finger::Button);
                return Some(action);
            }
        }
        let elevations = self.elevation_zones();
        let role = if contains(elevations[0], p) {
            self.jump_pressed = true;
            Finger::Up
        } else if contains(elevations[1], p) {
            Finger::Down
        } else if contains(self.move_zone(), p)
            && !self
                .fingers
                .values()
                .any(|f| matches!(f, Finger::Move { .. }))
        {
            Finger::Move {
                origin: p,
                current: p,
            }
        } else if !self
            .fingers
            .values()
            .any(|f| matches!(f, Finger::Look { .. }))
        {
            Finger::Look { previous: p }
        } else {
            Finger::Button
        };
        self.fingers.insert(id, role);
        None
    }
    pub fn moved(&mut self, id: u64, p: Vec2) {
        if let Some(f) = self.fingers.get_mut(&id) {
            match f {
                Finger::Move { current, .. } => *current = p,
                Finger::Look { previous } => {
                    self.look_delta += p - *previous;
                    *previous = p;
                }
                _ => {}
            }
        }
    }
    pub fn end(&mut self, id: u64) {
        self.fingers.remove(&id);
    }
    pub fn mouse(&mut self, p: Vec2) {
        if self.mouse_look {
            if let Some(previous) = self.cursor {
                self.look_delta += p - previous;
            }
        }
        self.cursor = Some(p);
    }
    pub fn consume(&mut self) -> (Vec3, Vec2) {
        let mut motion = Vec3::ZERO;
        for f in self.fingers.values() {
            match *f {
                Finger::Move { origin, current } => {
                    let delta = ((current - origin) / 70.).clamp_length_max(1.);
                    motion.x += delta.x;
                    motion.z -= delta.y;
                }
                Finger::Up => motion.y += 1.,
                Finger::Down => motion.y -= 1.,
                _ => {}
            }
        }
        for (key, axis) in [
            (KeyCode::KeyW, Vec3::Z),
            (KeyCode::KeyS, Vec3::NEG_Z),
            (KeyCode::KeyA, Vec3::NEG_X),
            (KeyCode::KeyD, Vec3::X),
            (KeyCode::Space, Vec3::Y),
            (KeyCode::ShiftLeft, Vec3::NEG_Y),
        ] {
            if self.keys.contains(&key) {
                motion += axis;
            }
        }
        if std::mem::take(&mut self.jump_pressed) {
            motion.y = motion.y.max(1.);
        }
        let delta = std::mem::take(&mut self.look_delta);
        (motion.clamp_length_max(1.), delta)
    }
}
pub fn contains([x, y, w, h]: [f32; 4], p: Vec2) -> bool {
    p.x >= x && p.x < x + w && p.y >= y && p.y < y + h
}

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(12., 10., 23.),
            yaw: -2.82,
            pitch: -0.18,
        }
    }
}
impl Camera {
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        )
    }
    pub fn update(&mut self, motion: Vec3, look: Vec2, dt: f32) {
        self.yaw -= look.x * 0.004;
        self.pitch = (self.pitch - look.y * 0.004).clamp(-1.50, 1.50);
        let forward = Vec3::new(self.yaw.sin(), 0., self.yaw.cos());
        let right = Vec3::new(-self.yaw.cos(), 0., self.yaw.sin());
        self.position +=
            (right * motion.x + Vec3::Y * motion.y + forward * motion.z) * 10. * dt.clamp(0., 0.05);
        self.position = self
            .position
            .clamp(Vec3::new(-248., -8., -248.), Vec3::new(248., 70., 248.));
    }
    pub fn view_projection(&self, aspect: f32) -> [[f32; 4]; 4] {
        (Mat4::perspective_rh(65_f32.to_radians(), aspect.max(0.01), 0.1, 240.)
            * Mat4::look_at_rh(self.position, self.position + self.forward(), Vec3::Y))
        .to_cols_array_2d()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simultaneous_move_look_action_and_release() {
        let mut c = Controls::default();
        assert_eq!(c.start(1, Vec2::new(100., 450.)), None);
        c.moved(1, Vec2::new(100., 380.));
        c.start(2, Vec2::new(700., 250.));
        c.moved(2, Vec2::new(720., 260.));
        assert_eq!(c.start(3, Vec2::new(410., 540.)), Some(Action::Remove));
        let (m, l) = c.consume();
        assert_eq!(m, Vec3::Z);
        assert_eq!(l, Vec2::new(20., 10.));
        c.end(2);
        c.end(3);
        assert_eq!(c.consume().0, Vec3::Z);
        c.end(1);
        assert_eq!(c.consume().0, Vec3::ZERO);
    }
    #[test]
    fn cancel_focus_clear_prevents_stuck_input() {
        let mut c = Controls::default();
        c.start(4, Vec2::new(100., 450.));
        c.moved(4, Vec2::new(120., 390.));
        c.keys.insert(KeyCode::Space);
        c.mouse_look = true;
        c.clear();
        assert_eq!(c.consume(), (Vec3::ZERO, Vec2::ZERO));
        assert!(!c.mouse_look);
    }
    #[test]
    fn swap_and_resize_keep_action_targets_separate() {
        let mut c = Controls {
            swapped: true,
            large: true,
            ..Default::default()
        };
        assert_eq!(c.start(1, Vec2::new(830., 500.)), None);
        c.moved(1, Vec2::new(830., 430.));
        assert_eq!(c.consume().0, Vec3::Z);
        assert_eq!(c.start(2, Vec2::new(530., 540.)), Some(Action::Place));
    }
    #[test]
    fn stalled_frame_and_diagonal_motion_are_bounded() {
        let mut camera = Camera::default();
        let before = camera.position;
        camera.update(
            Vec3::new(1., 1., 1.).normalize(),
            Vec2::new(0., -1e6),
            1000.,
        );
        assert!((camera.position - before).length() <= 0.501);
        assert!(camera.forward().is_finite());
        assert!(camera.pitch <= 1.5);
        for _ in 0..10000 {
            camera.update(Vec3::Z, Vec2::ZERO, 0.05);
        }
        assert!(camera.position.abs().max_element() <= 248.);
    }
    #[test]
    fn quick_jump_tap_is_not_lost_between_frames() {
        let mut c = Controls::default();
        c.start(1, Vec2::new(940., 410.));
        c.end(1);
        assert_eq!(c.consume().0, Vec3::Y);
        assert_eq!(c.consume().0, Vec3::ZERO);
    }
}
