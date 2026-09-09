//! Reusable, platform-independent multi-touch, pointer, and keyboard input service.
use std::collections::{HashMap, HashSet};

/// Virtual keys recognized by the input service with standard or configurable axis mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualKey {
    W,
    A,
    S,
    D,
    Space,
    Shift,
    Up,
    Down,
    Left,
    Right,
    Action(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PointerRole {
    Button(u32),
    Move { origin: [f32; 2], current: [f32; 2] },
    Look { previous: [f32; 2] },
    MotionAxis { axis: [f32; 3] },
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MotionZone {
    rect: [f32; 4],
    axis: [f32; 3],
    latch: bool,
}

/// Platform-independent touch, pointer, and keyboard input tracker.
#[derive(Debug, Clone)]
pub struct InputService {
    pointers: HashMap<u64, PointerRole>,
    pressed_keys: HashSet<VirtualKey>,
    key_mappings: HashMap<VirtualKey, [f32; 3]>,
    move_zone: Option<([f32; 4], f32)>,
    look_zones: Vec<[f32; 4]>,
    button_zones: Vec<([f32; 4], u32)>,
    motion_zones: Vec<MotionZone>,
    look_delta: [f32; 2],
    latched_motion: [f32; 3],
    pending_actions: HashSet<u32>,
}

impl Default for InputService {
    fn default() -> Self {
        Self::new()
    }
}

pub fn contains([x, y, w, h]: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= x && p[0] < x + w && p[1] >= y && p[1] < y + h
}

impl InputService {
    /// Creates a new input service with standard WASD, Arrow, Space, and Shift key mappings.
    pub fn new() -> Self {
        let mut key_mappings = HashMap::new();
        key_mappings.insert(VirtualKey::W, [0.0, 0.0, 1.0]);
        key_mappings.insert(VirtualKey::S, [0.0, 0.0, -1.0]);
        key_mappings.insert(VirtualKey::A, [-1.0, 0.0, 0.0]);
        key_mappings.insert(VirtualKey::D, [1.0, 0.0, 0.0]);
        key_mappings.insert(VirtualKey::Up, [0.0, 0.0, 1.0]);
        key_mappings.insert(VirtualKey::Down, [0.0, 0.0, -1.0]);
        key_mappings.insert(VirtualKey::Left, [-1.0, 0.0, 0.0]);
        key_mappings.insert(VirtualKey::Right, [1.0, 0.0, 0.0]);
        key_mappings.insert(VirtualKey::Space, [0.0, 1.0, 0.0]);
        key_mappings.insert(VirtualKey::Shift, [0.0, -1.0, 0.0]);

        Self {
            pointers: HashMap::new(),
            pressed_keys: HashSet::new(),
            key_mappings,
            move_zone: None,
            look_zones: Vec::new(),
            button_zones: Vec::new(),
            motion_zones: Vec::new(),
            look_delta: [0.0, 0.0],
            latched_motion: [0.0, 0.0, 0.0],
            pending_actions: HashSet::new(),
        }
    }

    /// Sets or replaces the virtual joystick/movement zone with a bounding rect and max drag radius.
    pub fn set_move_zone(&mut self, rect: [f32; 4], radius: f32) {
        self.move_zone = Some((rect, radius));
    }

    /// Returns the currently configured move zone rect and radius if any.
    pub fn move_zone(&self) -> Option<([f32; 4], f32)> {
        self.move_zone
    }

    /// Sets a single look zone (or clears all look zones if None).
    pub fn set_look_zone(&mut self, rect: Option<[f32; 4]>) {
        self.look_zones.clear();
        if let Some(r) = rect {
            self.look_zones.push(r);
        }
    }

    /// Adds a look zone. When any look zones are configured, look touches must originate within one.
    pub fn add_look_zone(&mut self, rect: [f32; 4]) {
        self.look_zones.push(rect);
    }

    /// Clears all configured look zones, returning to unrestricted screen look fallback.
    pub fn clear_look_zones(&mut self) {
        self.look_zones.clear();
    }

    /// Adds a touch button zone associated with an action identifier.
    pub fn add_button_zone(&mut self, rect: [f32; 4], action_id: u32) {
        self.button_zones.push((rect, action_id));
    }

    /// Clears all registered touch button zones.
    pub fn clear_button_zones(&mut self) {
        self.button_zones.clear();
    }

    /// Adds a directional motion zone (e.g. elevation jump or descent buttons).
    pub fn add_motion_zone(&mut self, rect: [f32; 4], axis: [f32; 3], latch_on_tap: bool) {
        self.motion_zones.push(MotionZone {
            rect,
            axis,
            latch: latch_on_tap,
        });
    }

    /// Clears all registered motion zones.
    pub fn clear_motion_zones(&mut self) {
        self.motion_zones.clear();
    }

    /// Maps a virtual key to a 3D motion axis.
    pub fn map_key(&mut self, key: VirtualKey, axis: [f32; 3]) {
        self.key_mappings.insert(key, axis);
    }

    /// Records a key press.
    pub fn key_down(&mut self, key: VirtualKey) {
        if let VirtualKey::Action(id) = key {
            self.pending_actions.insert(id);
        } else {
            self.pressed_keys.insert(key);
        }
    }

    /// Records a key release.
    pub fn key_up(&mut self, key: VirtualKey) {
        self.pressed_keys.remove(&key);
    }

    /// Whether a key is currently tracked as pressed.
    pub fn is_key_down(&self, key: VirtualKey) -> bool {
        self.pressed_keys.contains(&key)
    }

    /// Processes a pointer/finger down event. Returns `Some(action_id)` if a button zone was hit.
    pub fn pointer_down(&mut self, id: u64, pos: [f32; 2]) -> Option<u32> {
        for &(rect, action_id) in &self.button_zones {
            if contains(rect, pos) {
                self.pointers.insert(id, PointerRole::Button(action_id));
                self.pending_actions.insert(action_id);
                return Some(action_id);
            }
        }

        for &zone in &self.motion_zones {
            if contains(zone.rect, pos) {
                if zone.latch {
                    self.latched_motion[0] += zone.axis[0];
                    self.latched_motion[1] += zone.axis[1];
                    self.latched_motion[2] += zone.axis[2];
                }
                self.pointers
                    .insert(id, PointerRole::MotionAxis { axis: zone.axis });
                return None;
            }
        }

        if let Some((rect, _)) = self.move_zone {
            if contains(rect, pos)
                && !self
                    .pointers
                    .values()
                    .any(|p| matches!(p, PointerRole::Move { .. }))
            {
                self.pointers.insert(
                    id,
                    PointerRole::Move {
                        origin: pos,
                        current: pos,
                    },
                );
                return None;
            }
        }

        let is_in_look = if self.look_zones.is_empty() {
            true
        } else {
            self.look_zones.iter().any(|&r| contains(r, pos))
        };

        if is_in_look
            && !self
                .pointers
                .values()
                .any(|p| matches!(p, PointerRole::Look { .. }))
        {
            self.pointers
                .insert(id, PointerRole::Look { previous: pos });
            return None;
        }

        self.pointers.insert(id, PointerRole::Ignored);
        None
    }

    /// Processes a pointer/finger move event.
    pub fn pointer_move(&mut self, id: u64, pos: [f32; 2]) {
        if let Some(role) = self.pointers.get_mut(&id) {
            match role {
                PointerRole::Move { current, .. } => {
                    *current = pos;
                }
                PointerRole::Look { previous } => {
                    self.look_delta[0] += pos[0] - previous[0];
                    self.look_delta[1] += pos[1] - previous[1];
                    *previous = pos;
                }
                _ => {}
            }
        }
    }

    /// Processes a pointer/finger up event.
    pub fn pointer_up(&mut self, id: u64) {
        self.pointers.remove(&id);
    }

    /// Adds external look delta (e.g. from mouse movement).
    pub fn add_look_delta(&mut self, delta: [f32; 2]) {
        self.look_delta[0] += delta[0];
        self.look_delta[1] += delta[1];
    }

    /// Consumes the accumulated motion vector, clamped to length <= 1.0.
    pub fn consume_motion(&mut self) -> [f32; 3] {
        let mut motion = [0.0f32; 3];

        for role in self.pointers.values() {
            match *role {
                PointerRole::Move { origin, current } => {
                    let radius = self.move_zone.map_or(70.0, |(_, r)| r).max(1.0);
                    let mut dx = (current[0] - origin[0]) / radius;
                    let mut dy = (current[1] - origin[1]) / radius;
                    let len_sq = dx * dx + dy * dy;
                    if len_sq > 1.0 {
                        let len = len_sq.sqrt();
                        dx /= len;
                        dy /= len;
                    }
                    motion[0] += dx;
                    motion[2] -= dy;
                }
                PointerRole::MotionAxis { axis } => {
                    motion[0] += axis[0];
                    motion[1] += axis[1];
                    motion[2] += axis[2];
                }
                _ => {}
            }
        }

        for key in &self.pressed_keys {
            if let Some(axis) = self.key_mappings.get(key) {
                motion[0] += axis[0];
                motion[1] += axis[1];
                motion[2] += axis[2];
            }
        }

        let latched = std::mem::take(&mut self.latched_motion);
        if latched[1] > 0.0 {
            motion[1] = motion[1].max(latched[1]);
        }
        motion[0] += latched[0];
        motion[2] += latched[2];

        let total_sq = motion[0] * motion[0] + motion[1] * motion[1] + motion[2] * motion[2];
        if total_sq > 1.0 {
            let total_len = total_sq.sqrt();
            motion[0] /= total_len;
            motion[1] /= total_len;
            motion[2] /= total_len;
        }

        motion
    }

    /// Consumes and resets the accumulated look delta.
    pub fn consume_look(&mut self) -> [f32; 2] {
        std::mem::take(&mut self.look_delta)
    }

    /// Returns true if the action was triggered, clearing its pending state.
    pub fn take_action(&mut self, id: u32) -> bool {
        self.pending_actions.remove(&id)
    }

    /// Drains and returns all pending triggered action identifiers in sorted order.
    pub fn take_actions(&mut self) -> Vec<u32> {
        let mut list: Vec<u32> = self.pending_actions.drain().collect();
        list.sort_unstable();
        list
    }

    /// Resets all pointers, keys, deltas, and latches to prevent stuck input.
    pub fn clear(&mut self) {
        self.pointers.clear();
        self.pressed_keys.clear();
        self.look_delta = [0.0, 0.0];
        self.latched_motion = [0.0, 0.0, 0.0];
        self.pending_actions.clear();
    }

    /// Returns the origin and current position of the active virtual joystick, if one is held.
    pub fn joystick_state(&self) -> Option<([f32; 2], [f32; 2])> {
        for role in self.pointers.values() {
            if let PointerRole::Move { origin, current } = *role {
                return Some((origin, current));
            }
        }
        None
    }

    /// Number of actively tracked pointers.
    pub fn active_pointers(&self) -> usize {
        self.pointers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_move_look_action_and_release() {
        let mut input = InputService::new();
        input.set_move_zone([20.0, 390.0, 190.0, 190.0], 70.0);
        input.add_button_zone([390.0, 520.0, 105.0, 58.0], 7);

        assert_eq!(input.pointer_down(1, [100.0, 450.0]), None);
        input.pointer_move(1, [100.0, 380.0]);

        assert_eq!(input.pointer_down(2, [700.0, 250.0]), None);
        input.pointer_move(2, [720.0, 260.0]);

        assert_eq!(input.pointer_down(3, [410.0, 540.0]), Some(7));
        assert!(input.take_action(7));
        assert!(!input.take_action(7));

        let motion = input.consume_motion();
        let look = input.consume_look();
        assert!((motion[0] - 0.0).abs() < 1e-5);
        assert!((motion[1] - 0.0).abs() < 1e-5);
        assert!((motion[2] - 1.0).abs() < 1e-5);
        assert_eq!(look, [20.0, 10.0]);

        input.pointer_up(2);
        input.pointer_up(3);
        let motion2 = input.consume_motion();
        assert!((motion2[2] - 1.0).abs() < 1e-5);

        input.pointer_up(1);
        let motion3 = input.consume_motion();
        assert_eq!(motion3, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn keyboard_motion_and_diagonal_clamping() {
        let mut input = InputService::new();
        input.key_down(VirtualKey::W);
        input.key_down(VirtualKey::D);
        let motion = input.consume_motion();
        let len = (motion[0] * motion[0] + motion[1] * motion[1] + motion[2] * motion[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-5);
        assert!(motion[0] > 0.7 && motion[0] < 0.72);
        assert!(motion[2] > 0.7 && motion[2] < 0.72);

        input.key_up(VirtualKey::W);
        input.key_up(VirtualKey::D);
        assert_eq!(input.consume_motion(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn quick_jump_tap_is_not_lost_between_frames() {
        let mut input = InputService::new();
        input.add_motion_zone([905.0, 390.0, 70.0, 48.0], [0.0, 1.0, 0.0], true);
        input.pointer_down(1, [940.0, 410.0]);
        input.pointer_up(1);
        let motion = input.consume_motion();
        assert_eq!(motion, [0.0, 1.0, 0.0]);
        assert_eq!(input.consume_motion(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn focus_loss_and_cancellation_clears_all_state() {
        let mut input = InputService::new();
        input.set_move_zone([20.0, 390.0, 190.0, 190.0], 70.0);
        input.pointer_down(1, [100.0, 450.0]);
        input.pointer_move(1, [120.0, 390.0]);
        input.key_down(VirtualKey::Space);
        input.add_look_delta([15.0, -10.0]);

        input.clear();

        assert_eq!(input.consume_motion(), [0.0, 0.0, 0.0]);
        assert_eq!(input.consume_look(), [0.0, 0.0]);
        assert_eq!(input.active_pointers(), 0);
        assert!(!input.is_key_down(VirtualKey::Space));
    }

    #[test]
    fn custom_look_zone_constrains_touch() {
        let mut input = InputService::new();
        input.add_look_zone([500.0, 100.0, 300.0, 300.0]);

        assert_eq!(input.pointer_down(1, [100.0, 100.0]), None);
        input.pointer_move(1, [150.0, 150.0]);
        assert_eq!(input.consume_look(), [0.0, 0.0]);

        assert_eq!(input.pointer_down(2, [600.0, 200.0]), None);
        input.pointer_move(2, [620.0, 210.0]);
        assert_eq!(input.consume_look(), [20.0, 10.0]);
    }
}
