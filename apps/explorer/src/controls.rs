use glam::{Mat4, Vec2, Vec3};
use matterweave_core::{InputService, VirtualKey};
use std::collections::HashSet;
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
    Shadows,
    Sun,
    ShadowQuality,
}

impl Action {
    fn to_u32(self) -> u32 {
        self as u32
    }

    fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Action::Remove),
            1 => Some(Action::Place),
            2 => Some(Action::Save),
            3 => Some(Action::Swap),
            4 => Some(Action::Size),
            5 => Some(Action::Grab),
            6 => Some(Action::Throw),
            7 => Some(Action::Break),
            8 => Some(Action::Flight),
            9 => Some(Action::Home),
            10 => Some(Action::ResetObjects),
            11 => Some(Action::Shadows),
            12 => Some(Action::Sun),
            13 => Some(Action::ShadowQuality),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Controls {
    pub service: InputService,
    pub keys: HashSet<KeyCode>,
    pub swapped: bool,
    pub large: bool,
    pub mouse_look: bool,
    pub cursor: Option<Vec2>,
}
impl Controls {
    pub fn clear(&mut self) {
        self.service.clear();
        self.keys.clear();
        self.mouse_look = false;
        self.cursor = None;
    }
    pub fn buttons(&self) -> [([f32; 4], &'static str, Action); 14] {
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
                "RESET PLAYGROUND",
                Action::ResetObjects,
            ),
            ([720., 164., 124., 40.], "SHADOWS", Action::Shadows),
            ([852., 164., 128., 40.], "SUN", Action::Sun),
            (
                [720., 212., 260., 40.],
                "SHADOW DETAIL",
                Action::ShadowQuality,
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
        self.service.clear_button_zones();
        for (rect, _, action) in self.buttons() {
            self.service.add_button_zone(rect, action.to_u32());
        }
        self.service.clear_motion_zones();
        let elevations = self.elevation_zones();
        self.service
            .add_motion_zone(elevations[0], [0., 1., 0.], true);
        self.service
            .add_motion_zone(elevations[1], [0., -1., 0.], false);
        self.service.set_move_zone(self.move_zone(), 70.0);
        self.service.clear_look_zones();

        self.service
            .pointer_down(id, [p.x, p.y])
            .and_then(Action::from_u32)
    }
    /// Gesture roles for the wetland HUD, independent of sandbox buttons.
    pub fn start_wetland(&mut self, id: u64, p: Vec2) {
        self.service.clear_button_zones();
        self.service.clear_motion_zones();
        self.service
            .add_motion_zone([910., 367., 64., 55.], [0., 1., 0.], true);
        self.service.set_move_zone([60., 410., 142., 142.], 70.0);
        self.service.clear_look_zones();

        self.service.pointer_down(id, [p.x, p.y]);
    }
    pub fn moved(&mut self, id: u64, p: Vec2) {
        self.service.pointer_move(id, [p.x, p.y]);
    }
    pub fn end(&mut self, id: u64) {
        self.service.pointer_up(id);
    }
    pub fn mouse(&mut self, p: Vec2) {
        if self.mouse_look {
            if let Some(previous) = self.cursor {
                let delta = p - previous;
                self.service.add_look_delta([delta.x, delta.y]);
            }
        }
        self.cursor = Some(p);
    }
    pub fn consume(&mut self) -> (Vec3, Vec2) {
        for (key, vk) in [
            (KeyCode::KeyW, VirtualKey::W),
            (KeyCode::KeyS, VirtualKey::S),
            (KeyCode::KeyA, VirtualKey::A),
            (KeyCode::KeyD, VirtualKey::D),
            (KeyCode::Space, VirtualKey::Space),
            (KeyCode::ShiftLeft, VirtualKey::Shift),
        ] {
            if self.keys.contains(&key) {
                self.service.key_down(vk);
            } else {
                self.service.key_up(vk);
            }
        }
        let motion = self.service.consume_motion();
        let delta = self.service.consume_look();
        (Vec3::from_array(motion), Vec2::from_array(delta))
    }
}
pub fn contains([x, y, w, h]: [f32; 4], p: Vec2) -> bool {
    matterweave_core::input::contains([x, y, w, h], [p.x, p.y])
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
