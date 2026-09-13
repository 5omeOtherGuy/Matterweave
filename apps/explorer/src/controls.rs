use crate::settings::{Handedness, SettingRow, SharedSettings};
use glam::{Mat4, Vec2, Vec3};
use matterweave_core::{InputService, VirtualKey};
use matterweave_render::Hud;
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
    /// Gesture roles for the wetland HUD, independent of sandbox buttons. The
    /// move and jump zones come from the same shared layout the HUD draws, so a
    /// preference change can never leave a stale hit region behind.
    pub fn start_wetland(&mut self, layout: &WetlandLayout, id: u64, p: Vec2) {
        self.service.clear_button_zones();
        self.service.clear_motion_zones();
        self.service
            .add_motion_zone(layout.jump_zone, [0., 1., 0.], true);
        self.service.set_move_zone(layout.move_zone, 70.0);
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

/// Map a window-relative pixel position into the 1000x600 HUD space used by
/// every sample. Draw and touch hit-testing share this projection, so a drawn
/// rectangle and its hit region cannot drift apart when the window is resized.
pub fn virtual_point(x: f64, y: f64, width: u32, height: u32) -> Vec2 {
    Vec2::new(
        x as f32 / width.max(1) as f32 * 1000.,
        y as f32 / height.max(1) as f32 * 600.,
    )
}

#[cfg(test)]
fn centered(rect: [f32; 4]) -> Vec2 {
    Vec2::new(rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.)
}

#[cfg(test)]
fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// One settings overlay shared by the chooser, the wetland and Voxel Relay.
/// The panel rect, the three preference rows and the DONE control are one
/// calculation used for both drawing and hit-testing.
pub struct SettingsPanel {
    pub panel: [f32; 4],
    pub title: [f32; 2],
    pub rows: [([f32; 4], SettingRow); 3],
    pub done: [f32; 4],
}

pub fn settings_panel() -> SettingsPanel {
    let row = |index: usize| [350., 196. + index as f32 * 56., 300., 46.];
    SettingsPanel {
        panel: [330., 130., 340., 320.],
        title: [350., 150.],
        rows: [
            (row(0), SettingRow::Handedness),
            (row(1), SettingRow::LargeControls),
            (row(2), SettingRow::Mute),
        ],
        done: [410., 372., 180., 44.],
    }
}

/// Outcome of one click inside an open settings overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPanelClick {
    /// A preference row toggled. The caller must persist the change and clear
    /// active contacts so a held touch cannot activate the previous layout.
    Toggled,
    /// DONE was pressed; the caller closes the overlay and clears contacts.
    Close,
    /// The click was outside every panel element.
    Outside,
}

pub fn settings_panel_click(settings: &mut SharedSettings, p: Vec2) -> SettingsPanelClick {
    let panel = settings_panel();
    if contains(panel.done, p) {
        return SettingsPanelClick::Close;
    }
    for (rect, row) in panel.rows {
        if contains(rect, p) {
            settings.toggle(row);
            return SettingsPanelClick::Toggled;
        }
    }
    SettingsPanelClick::Outside
}

/// Draw the shared settings overlay with the unchanged HUD palette. It only
/// uses user-facing labels; the rows and values come from the same
/// [`settings_panel`] geometry the hit-testing uses.
pub fn draw_settings_panel(hud: &mut Hud, settings: &SharedSettings) {
    let panel = settings_panel();
    let ink = [0.87, 0.91, 0.82, 1.];
    let gold = [0.82, 0.65, 0.37, 1.];
    let backdrop = [0.02, 0.035, 0.04, 0.96];
    let row_fill = [0.07, 0.13, 0.13, 1.];
    hud.rect(panel.panel, backdrop);
    hud.text(panel.title[0], panel.title[1], "SETTINGS", 1.8, gold);
    for (rect, row) in panel.rows {
        hud.rect(rect, row_fill);
        hud.text(rect[0] + 12., rect[1] + 8., row.title(), 1.3, ink);
        let value = settings.value_label(row);
        let value_width = value.chars().count() as f32 * 8. * 1.2;
        hud.text(
            rect[0] + rect[2] - 12. - value_width,
            rect[1] + 8.,
            value,
            1.2,
            gold,
        );
    }
    hud.rect(panel.done, row_fill);
    hud.text(panel.done[0] + 12., panel.done[1] + 8., "DONE", 1.5, ink);
}

/// Wetland touch controls for one shared preference state. The action
/// rectangles are also the regions `click` test, so the HUD and touch agree by
/// construction. Defaults reproduce the layout the sample shipped with.
pub struct WetlandLayout {
    pub move_zone: [f32; 4],
    pub jump_zone: [f32; 4],
    pub actions: [([f32; 4], &'static str, Action); 5],
}

pub fn wetland_layout(settings: &SharedSettings) -> WetlandLayout {
    let (move_zone, jump_zone, remove, place, grab, throw, brake) = if settings.large_controls {
        (
            [60., 374., 178., 178.],
            [894., 320., 80., 70.],
            [548., 500., 128., 72.],
            [686., 500., 128., 72.],
            [824., 500., 150., 72.],
            [686., 416., 128., 68.],
            [824., 416., 150., 68.],
        )
    } else {
        (
            [60., 410., 142., 142.],
            [910., 367., 64., 55.],
            [630., 514., 102., 58.],
            [742., 514., 102., 58.],
            [854., 514., 120., 58.],
            [742., 444., 102., 54.],
            [854., 444., 120., 54.],
        )
    };
    let flipped = |rect: [f32; 4]| {
        if settings.handedness == Handedness::Left {
            rect
        } else {
            [1000. - rect[0] - rect[2], rect[1], rect[2], rect[3]]
        }
    };
    WetlandLayout {
        move_zone: flipped(move_zone),
        jump_zone: flipped(jump_zone),
        actions: [
            (flipped(remove), "REMOVE", Action::Remove),
            (flipped(place), "PLACE", Action::Place),
            (flipped(grab), "GRAB", Action::Grab),
            (flipped(brake), "BREAK", Action::Break),
            (flipped(throw), "THROW", Action::Throw),
        ],
    }
}

/// Voxel Relay touch controls for one shared preference state. The movement
/// stick mirrors with handedness; ACTION stays on the opposite side so the two
/// primary zones can never overlap. Defaults reproduce the shipped layout.
pub struct RelayLayout {
    pub move_zone: [f32; 4],
    pub action: [f32; 4],
    pub menu: [f32; 4],
    pub save: [f32; 4],
    pub reset: [f32; 4],
    pub settings: [f32; 4],
}

pub fn relay_layout(settings: &SharedSettings) -> RelayLayout {
    let (move_zone, action) = if settings.large_controls {
        ([40., 344., 216., 216.], [720., 436., 240., 84.])
    } else {
        ([40., 380., 180., 180.], [760., 450., 200., 70.])
    };
    let mirror = |rect: [f32; 4]| {
        if settings.handedness == Handedness::Left {
            rect
        } else {
            [1000. - rect[0] - rect[2], rect[1], rect[2], rect[3]]
        }
    };
    RelayLayout {
        move_zone: mirror(move_zone),
        action: mirror(action),
        menu: [650., 20., 90., 40.],
        save: [755., 20., 90., 40.],
        reset: [860., 20., 95., 40.],
        settings: [650., 70., 90., 36.],
    }
}

/// Whether a pointer position lies in an action rectangle; used by both the
/// sample click path and its tests. Kept beside the layout so they cannot
/// disagree.
pub fn action_at(layout: &WetlandLayout, p: Vec2) -> Option<Action> {
    layout
        .actions
        .iter()
        .find(|(rect, _, _)| contains(*rect, p))
        .map(|(_, _, action)| *action)
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

    #[test]
    fn wetland_default_layout_matches_the_shipped_rectangles() {
        let layout = wetland_layout(&SharedSettings::default());
        assert_eq!(layout.move_zone, [60., 410., 142., 142.]);
        assert_eq!(layout.jump_zone, [910., 367., 64., 55.]);
        let actions: Vec<([f32; 4], &str, Action)> = layout.actions.to_vec();
        assert_eq!(
            actions,
            vec![
                ([630., 514., 102., 58.], "REMOVE", Action::Remove),
                ([742., 514., 102., 58.], "PLACE", Action::Place),
                ([854., 514., 120., 58.], "GRAB", Action::Grab),
                ([854., 444., 120., 54.], "BREAK", Action::Break),
                ([742., 444., 102., 54.], "THROW", Action::Throw),
            ]
        );
        assert_eq!(
            action_at(&layout, Vec2::new(680., 540.)),
            Some(Action::Remove)
        );
        assert_eq!(action_at(&layout, Vec2::new(500., 540.)), None);
    }

    #[test]
    fn relay_default_layout_matches_the_shipped_rectangles() {
        let layout = relay_layout(&SharedSettings::default());
        assert_eq!(layout.move_zone, [40., 380., 180., 180.]);
        assert_eq!(layout.action, [760., 450., 200., 70.]);
        assert_eq!(layout.menu, [650., 20., 90., 40.]);
        assert_eq!(layout.save, [755., 20., 90., 40.]);
        assert_eq!(layout.reset, [860., 20., 95., 40.]);
        assert_eq!(layout.settings, [650., 70., 90., 36.]);
    }

    #[test]
    fn every_layout_combination_fits_the_canvas_without_overlapping_zones() {
        let settings_button = [880., 20., 100., 42.];
        for handedness in [Handedness::Left, Handedness::Right] {
            for large_controls in [false, true] {
                let settings = SharedSettings {
                    handedness,
                    large_controls,
                    ..SharedSettings::default()
                };
                let wetland = wetland_layout(&settings);
                let mut wetland_rects = vec![
                    ("move", wetland.move_zone),
                    ("jump", wetland.jump_zone),
                    ("settings button", settings_button),
                ];
                for (rect, label, _) in wetland.actions {
                    wetland_rects.push((label, rect));
                }
                for (name, rect) in &wetland_rects {
                    assert!(
                        rect[0] >= 0.
                            && rect[1] >= 0.
                            && rect[0] + rect[2] <= 1000.
                            && rect[1] + rect[3] <= 600.,
                        "wetland {name} {rect:?} leaves the 1000x600 canvas ({handedness:?}, large={large_controls})"
                    );
                }
                for (i, (a_name, a)) in wetland_rects.iter().enumerate() {
                    for (b_name, b) in wetland_rects.iter().skip(i + 1) {
                        assert!(
                            !overlaps(*a, *b),
                            "wetland {a_name} {a:?} overlaps {b_name} {b:?} ({handedness:?}, large={large_controls})"
                        );
                    }
                }

                let relay = relay_layout(&settings);
                let relay_rects = [
                    ("move", relay.move_zone),
                    ("action", relay.action),
                    ("menu", relay.menu),
                    ("save", relay.save),
                    ("reset", relay.reset),
                    ("settings", relay.settings),
                ];
                for (name, rect) in &relay_rects {
                    assert!(
                        rect[0] >= 0.
                            && rect[1] >= 0.
                            && rect[0] + rect[2] <= 1000.
                            && rect[1] + rect[3] <= 600.,
                        "relay {name} {rect:?} leaves the 1000x600 canvas ({handedness:?}, large={large_controls})"
                    );
                }
                for (i, (a_name, a)) in relay_rects.iter().enumerate() {
                    for (b_name, b) in relay_rects.iter().skip(i + 1) {
                        assert!(
                            !overlaps(*a, *b),
                            "relay {a_name} {a:?} overlaps {b_name} {b:?} ({handedness:?}, large={large_controls})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn handedness_mirrors_the_primary_zones_around_the_canvas_center() {
        let left = SharedSettings::default();
        let right = SharedSettings {
            handedness: Handedness::Right,
            ..SharedSettings::default()
        };
        let left_wetland = wetland_layout(&left);
        let right_wetland = wetland_layout(&right);
        assert_eq!(
            right_wetland.move_zone[0],
            1000. - left_wetland.move_zone[0] - left_wetland.move_zone[2]
        );
        assert_eq!(
            right_wetland.jump_zone[0],
            1000. - left_wetland.jump_zone[0] - left_wetland.jump_zone[2]
        );
        let left_relay = relay_layout(&left);
        let right_relay = relay_layout(&right);
        assert_eq!(
            right_relay.move_zone[0],
            1000. - left_relay.move_zone[0] - left_relay.move_zone[2]
        );
        assert_eq!(
            right_relay.action[0],
            1000. - left_relay.action[0] - left_relay.action[2]
        );
        assert_eq!(
            left_relay.move_zone[0], 40.,
            "the stick stays left by default"
        );
        assert_eq!(right_relay.move_zone[0], 780.);
    }

    #[test]
    fn settings_panel_geometry_is_disjoint_and_hit_testing_toggles() {
        let panel = settings_panel();
        assert!(!overlaps(panel.rows[0].0, panel.rows[1].0));
        assert!(!overlaps(panel.rows[1].0, panel.rows[2].0));
        assert!(!overlaps(panel.rows[2].0, panel.done));
        for (rect, _) in panel.rows {
            assert!(!overlaps(panel.done, rect));
            assert!(
                rect[0] >= panel.panel[0]
                    && rect[1] >= panel.panel[1]
                    && rect[0] + rect[2] <= panel.panel[0] + panel.panel[2]
                    && rect[1] + rect[3] <= panel.panel[1] + panel.panel[3]
            );
        }

        let mut settings = SharedSettings::default();
        assert_eq!(
            settings_panel_click(&mut settings, Vec2::new(10., 10.)),
            SettingsPanelClick::Outside
        );
        assert_eq!(settings, SharedSettings::default());
        assert_eq!(
            settings_panel_click(&mut settings, centered(panel.rows[0].0)),
            SettingsPanelClick::Toggled
        );
        assert_eq!(settings.handedness, Handedness::Right);
        assert_eq!(
            settings_panel_click(&mut settings, centered(panel.rows[1].0)),
            SettingsPanelClick::Toggled
        );
        assert!(settings.large_controls);
        assert_eq!(
            settings_panel_click(&mut settings, centered(panel.rows[2].0)),
            SettingsPanelClick::Toggled
        );
        assert!(settings.muted);
        assert_eq!(
            settings_panel_click(&mut settings, centered(panel.done)),
            SettingsPanelClick::Close
        );
        assert!(settings.muted, "DONE must not change a value");
    }

    #[test]
    fn settings_panel_draws_for_every_value_without_panicking() {
        for settings in [
            SharedSettings::default(),
            SharedSettings {
                handedness: Handedness::Right,
                large_controls: true,
                muted: true,
                ..SharedSettings::default()
            },
        ] {
            let mut hud = Hud::new(1000., 600.);
            draw_settings_panel(&mut hud, &settings);
        }
    }

    #[test]
    fn virtual_point_maps_narrow_and_wide_viewports_into_the_drawn_rectangles() {
        let layout = relay_layout(&SharedSettings {
            handedness: Handedness::Right,
            large_controls: true,
            ..SharedSettings::default()
        });
        for (width, height) in [(1000_u32, 600_u32), (480, 800), (1600, 720), (2400, 1080)] {
            for rect in [layout.move_zone, layout.action, layout.settings] {
                let center = centered(rect);
                let pixel_x = center.x / 1000. * width as f32;
                let pixel_y = center.y / 600. * height as f32;
                let mapped = virtual_point(pixel_x as f64, pixel_y as f64, width, height);
                assert!(
                    contains(rect, mapped),
                    "viewport {width}x{height} maps back into {rect:?}, got {mapped:?}"
                );
            }
        }
    }

    #[test]
    fn wetland_start_uses_the_shared_layout_and_swaps_cleanly() {
        let settings = SharedSettings {
            handedness: Handedness::Right,
            large_controls: true,
            ..SharedSettings::default()
        };
        let layout = wetland_layout(&settings);
        let mut c = Controls::default();
        c.start_wetland(&layout, 1, centered(layout.move_zone));
        assert_eq!(c.service.move_zone(), Some((layout.move_zone, 70.0)));
        c.moved(1, centered(layout.move_zone) + Vec2::new(70., 0.));
        assert!((c.consume().0.x - 1.0).abs() < 1e-4);
        c.end(1);

        c.start_wetland(&layout, 2, centered(layout.jump_zone));
        c.end(2);
        assert_eq!(c.consume().0, Vec3::Y);

        // A layout change clears the active contacts: the old stick position
        // must not drive motion or become a new button press afterwards.
        c.start_wetland(
            &wetland_layout(&SharedSettings::default()),
            3,
            Vec2::new(131., 481.),
        );
        c.moved(3, Vec2::new(131., 420.));
        assert!(c.consume().0.z > 0.5, "the old-default stick still moves");
        c.clear();
        assert_eq!(c.consume(), (Vec3::ZERO, Vec2::ZERO));
        c.start_wetland(&layout, 4, Vec2::new(131., 481.));
        c.moved(4, Vec2::new(131., 420.));
        assert_eq!(
            c.consume().0,
            Vec3::ZERO,
            "a held touch over the previous stick position must not move the right-hand layout"
        );
        c.end(4);
    }
}
