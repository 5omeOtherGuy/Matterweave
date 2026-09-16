//! The `world` sample mode: walk the landscape with solid terrain.
//!
//! The renderer draws exactly the scene `landscape` mode draws - the resident
//! fine window, the distance rings, water, flora, sky and clouds - but the
//! camera is a player instead of a machine on rails: a capsule moved by the
//! shared [`matterweave_physics`] character controller with gravity, walking
//! and a jump. Flying is deliberately unavailable here; the point of the mode
//! is that the world is solid.
//!
//! # Collision and the analytic fallback
//!
//! Inside the resident fine chunk window, collision is the authoritative voxel
//! geometry published by `Physics::sync_world`: exact solid cells, never a
//! visual mesh or a ring LOD. Outside that window only the ring cascade and
//! the analytic generator exist, and both a hole and an invisible wall at the
//! boundary would be simulation errors. So a single probe answers the
//! generator's walkable surface - [`matterweave_core::landscape::height_at`]
//! plus the one-metre cell's top face - for every column the window does not
//! cover. The physics adapter consults it only where residency says there is
//! no voxel collider, so published geometry always wins inside the window and
//! the two sources cannot compete at the edge.
//!
//! The window's voxels are cut from that same generator surface, so the
//! handover agrees to the cell, not merely approximately. The tests below pin
//! the heights on both sides of the boundary, walk the player across it with
//! the window frozen, and prove the fallback refuses a cliff face.
//!
//! Collision reads the world and the generator; it never edits either. A
//! height is never changed to make the player fit.

use crate::landscape::SEED;
use glam::Vec3;
use matterweave_core::landscape;
use matterweave_core::World;
use matterweave_physics::{AnalyticGround, CharacterProfile, Physics, FIXED_DT};
use std::path::Path;
use std::time::Instant;

/// Marker beside the save selecting this mode on a device, exactly like
/// `landscape.txt`: `adb shell run-as dev.matterweave.explorer touch files/world.txt`.
/// A device run then walks the landscape instead of flying over it.
pub const MARKER_FILE: &str = "world.txt";

/// Walking speed, m/s. A person walks at about 1.4 m/s; a first-person demo
/// with a six kilometre landscape to cross walks several times that.
pub const WALK_SPEED_M_S: f32 = 4.5;
/// Running speed, m/s, while Shift is held on the desktop.
pub const RUN_SPEED_M_S: f32 = 7.0;
/// Camera height above the feet, m.
pub const EYE_HEIGHT_M: f32 = 1.7;
/// Design apex of a standing jump, m. The physics profile derives the launch
/// speed from this and the fixed gravity.
pub const JUMP_HEIGHT_M: f32 = 1.2;
/// Largest step the character climbs: one landscape voxel plus the contact
/// margin the controller needs. A two-voxel face is a cliff, not a stair.
pub const STEP_UP_M: f32 = 1.05;
/// Horizontal half-extent the character is clamped to, m. The eye stays inside
/// the range the landscape renderer and ring planner document.
const BOUNDS_M: f32 = (matterweave_core::LANDSCAPE_WORLD_LIMIT - 66) as f32;
/// Height above the chosen spawn surface the capsule is placed at before the
/// construction settle, m. Enough for `teleport`'s intersection test to accept
/// the pose without a visible drop on the first frame.
const SPAWN_LIFT_M: f32 = 0.05;

/// Where a world-mode run starts: the eye pose and the look direction.
#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// The walkable top of the generator's surface for one metre column: the top
/// face of the column's highest solid cell, in metres. The resident chunks are
/// filled from this same function, which is why the analytic handover at the
/// window edge is exact rather than approximate.
pub fn surface_at(x: i32, z: i32) -> f32 {
    landscape::height_at(SEED, x, z) as f32 + 1.0
}

/// The analytic ground probe this mode installs: the generator surface for any
/// column, because the physics adapter only ever asks for columns the resident
/// window does not cover.
fn analytic_ground() -> AnalyticGround {
    Box::new(|x, z| Some(surface_at(x, z)))
}

/// Whether this run selects world mode: the desktop flag, or the marker beside
/// the save (the only entry a phone has, since NativeActivity passes no
/// command line).
pub fn requested(flag: bool, directory: &Path) -> bool {
    flag || marker_present(directory)
}

/// Whether the marker file is present beside the save, read with the same
/// tolerance as every landscape marker: unreadable or oversized is absent,
/// never fatal.
pub fn marker_present(directory: &Path) -> bool {
    crate::landscape::bounded_marker_text(directory, MARKER_FILE).is_some()
}

/// A coastal spawn worth opening the demo on: low ground with open water 64 m
/// away and a flat enough neighbourhood to stand on. It is the same
/// deterministic scan the landscape sample's fly spawn uses, but the eye
/// stands on the surface and a sparsely covered shore (beach, dune, rock) is
/// preferred, because the fly camera looks from eight metres up while the
/// walking eye looks from inside the ground cover.
pub fn spawn() -> Spawn {
    let mut coast = None;
    let mut sparse = None;
    let mut flat = None;
    for gz in (-24..24).rev() {
        for gx in -24..24 {
            let (x, z) = (gx * 64, gz * 64);
            let ground = landscape::height_at(SEED, x, z);
            if !(2..=14).contains(&ground) {
                continue;
            }
            let mut facing = None;
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
            ] {
                if landscape::height_at(SEED, x + dx * 64, z + dz * 64) < -2 {
                    facing = Some((dx, dz));
                    break;
                }
            }
            let Some((dx, dz)) = facing else {
                continue;
            };
            let spawn = standing_spawn(x, z, (dx as f32).atan2(dz as f32));
            if coast.is_none() {
                coast = Some(spawn);
            }
            let open = landscape::biome_at(SEED, x, z).grass_density() <= 16;
            if open && sparse.is_none() {
                sparse = Some(spawn);
            }
            if flat_enough(x, z) {
                if open {
                    return spawn;
                }
                if flat.is_none() {
                    flat = Some(spawn);
                }
            }
        }
    }
    // No open shore in the probed square: a flat one, then any coast, then a
    // start on the origin rather than failing to start.
    sparse
        .or(flat)
        .or(coast)
        .unwrap_or_else(|| standing_spawn(0, 0, 0.6))
}

/// A spawn with the eye a hair above the column's surface, so the settle can
/// land it without an intersection rejection.
fn standing_spawn(x: i32, z: i32, yaw: f32) -> Spawn {
    Spawn {
        eye: [
            x as f32 + 0.5,
            surface_at(x, z) + EYE_HEIGHT_M + SPAWN_LIFT_M,
            z as f32 + 0.5,
        ],
        yaw,
        pitch: -0.05,
    }
}

/// Whether the ground around a site is flat enough to stand and walk on:
/// adjacent columns within one voxel of each other over a three-metre square.
fn flat_enough(x: i32, z: i32) -> bool {
    let mut low = i32::MAX;
    let mut high = i32::MIN;
    for dz in -1..=1 {
        for dx in -1..=1 {
            let height = landscape::height_at(SEED, x + dx, z + dz);
            low = low.min(height);
            high = high.max(height);
        }
    }
    high - low <= 1
}

/// One frame of movement input in world axes: `move_x` strafes right, `move_z`
/// walks forward, both in `-1..=1`; `jump` is the held jump control and `run`
/// selects the run speed.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    pub move_x: f32,
    pub move_z: f32,
    pub jump: bool,
    pub run: bool,
}

impl PlayerInput {
    /// Idle input, for tests that need a base to spread the rest over.
    #[cfg(test)]
    pub const IDLE: Self = Self {
        move_x: 0.0,
        move_z: 0.0,
        jump: false,
        run: false,
    };
}

/// The movement profile this mode installs: a 1.7 m eye, a 1.2 m jump, a
/// one-voxel autostep and the landscape's travel bounds. Exposed for tests and
/// for the HUD, which must state what the phone actually runs.
pub fn world_profile() -> CharacterProfile {
    CharacterProfile {
        eye_height_m: EYE_HEIGHT_M,
        jump_height_m: JUMP_HEIGHT_M,
        autostep_height_m: STEP_UP_M,
        autostep_min_width_m: 0.20,
        bounds_m: BOUNDS_M,
    }
}

/// The jump button drawn beside the landscape sample's move stick. The HUD and
/// the touch hit-testing share this rectangle, so they cannot drift apart.
pub fn jump_zone(move_zone: [f32; 4], swapped: bool) -> [f32; 4] {
    let x = if swapped {
        move_zone[0] - 128.0
    } else {
        move_zone[0] + move_zone[2] + 18.0
    };
    [x, move_zone[1] + move_zone[3] - 96.0, 110.0, 96.0]
}

pub struct WorldPlayer {
    physics: Physics,
    /// The world revision the published collision corresponds to. A standing
    /// player costs a compare per frame; only a window shift or an edit
    /// actually rebuilds colliders.
    synced_revision: u64,
    speed_m_s: f32,
    steps_simulated: u64,
    frames: u64,
    /// Collision publishes this player performed: one per world revision, i.e.
    /// per window shift or edit, never per frame.
    syncs: u64,
    step_ms: f64,
    worst_step_ms: f64,
    sync_ms: f64,
    worst_sync_ms: f64,
}

impl WorldPlayer {
    /// Builds the player on `world`, with the window already streamed around
    /// `spawn`. Publishes collision, places the capsule and settles it, so the
    /// first presented frame starts standing on the surface instead of falling
    /// towards it.
    pub fn new(world: &World, spawn: Spawn) -> Result<Self, String> {
        let mut physics = Physics::new(world);
        if !physics.set_character_profile(world_profile()) {
            return Err("world movement profile rejected".into());
        }
        physics.set_analytic_ground(Some(analytic_ground()));
        let mut placed = false;
        for step in 0..=16 {
            let lift = step as f32 * 0.25;
            let eye = [spawn.eye[0], spawn.eye[1] + lift, spawn.eye[2]];
            if physics.teleport(eye) {
                placed = true;
                break;
            }
        }
        if !placed {
            return Err(format!("spawn {:?} is inside solid geometry", spawn.eye));
        }
        let mut settled = false;
        for _ in 0..60 {
            physics.step(FIXED_DT, [0.0; 3], false);
            if physics.grounded() {
                settled = true;
                break;
            }
        }
        if !settled {
            return Err(format!("no ground under the spawn {:?}", spawn.eye));
        }
        Ok(Self {
            physics,
            synced_revision: world.revision(),
            speed_m_s: 0.0,
            steps_simulated: 0,
            frames: 0,
            syncs: 0,
            step_ms: 0.0,
            worst_step_ms: 0.0,
            sync_ms: 0.0,
            worst_sync_ms: 0.0,
        })
    }

    /// Publishes the world's collision to the character when it changed: a
    /// streaming window shift or an edit. Nothing is rebuilt otherwise, so a
    /// standing player's collision cost is a revision compare.
    pub fn sync_world(&mut self, world: &World) {
        let revision = world.revision();
        if revision == self.synced_revision {
            return;
        }
        let begin = Instant::now();
        self.physics.sync_world(world);
        self.sync_ms = begin.elapsed().as_secs_f64() * 1000.0;
        self.worst_sync_ms = self.worst_sync_ms.max(self.sync_ms);
        self.syncs += 1;
        self.synced_revision = revision;
    }

    /// One frame of movement at the camera's yaw. `dt` is the frame delta; the
    /// physics adapter simulates at most six fixed steps and discards stalled
    /// time rather than accelerating.
    pub fn step(&mut self, dt: f32, input: PlayerInput, yaw: f32) {
        let begin = Instant::now();
        let (sin, cos) = yaw.sin_cos();
        let direction = (Vec3::new(-cos, 0.0, sin) * input.move_x
            + Vec3::new(sin, 0.0, cos) * input.move_z)
            .normalize_or_zero();
        let speed = if input.run {
            RUN_SPEED_M_S
        } else {
            WALK_SPEED_M_S
        };
        let before = self.physics.character_eye();
        let substeps = self
            .physics
            .step(dt, (direction * speed).to_array(), input.jump);
        let after = self.physics.character_eye();
        if substeps > 0 && dt > 0.0 {
            let dx = after[0] - before[0];
            let dz = after[2] - before[2];
            self.speed_m_s = (dx * dx + dz * dz).sqrt() / dt;
        } else if direction.length_squared() == 0.0 {
            self.speed_m_s = 0.0;
        }
        self.steps_simulated += substeps as u64;
        self.frames += 1;
        self.step_ms = begin.elapsed().as_secs_f64() * 1000.0;
        self.worst_step_ms = self.worst_step_ms.max(self.step_ms);
    }

    pub fn eye(&self) -> Vec3 {
        Vec3::from_array(self.physics.character_eye())
    }

    pub fn grounded(&self) -> bool {
        self.physics.grounded()
    }

    /// Whether the last fixed step rested on the analytic fallback rather than
    /// on resident voxel collision.
    pub fn on_analytic_ground(&self) -> bool {
        self.physics.on_analytic_ground()
    }

    /// The HUD's player line: ground contact, the surface that carried the last
    /// step, speed and position. Built here so the text the lead reads on the
    /// phone is the text the tests read on the host.
    pub fn hud_line(&self) -> String {
        let eye = self.eye();
        let ground = if !self.grounded() {
            "-"
        } else if self.on_analytic_ground() {
            "ANALYTIC"
        } else {
            "VOXEL"
        };
        format!(
            "PLAYER {} | GROUND {} | SPEED {:.1} M/S | POS {:.0} {:.0} {:.0}",
            if self.grounded() {
                "GROUNDED"
            } else {
                "AIRBORNE"
            },
            ground,
            self.speed_m_s,
            eye.x,
            eye.y,
            eye.z,
        )
    }

    /// The smoke run's summary: what the player did and what physics cost.
    pub fn summary_line(&self) -> String {
        format!(
            "{} | step {:.3} ms/frame worst {:.3} ({:.2} fixed steps/frame) | collision sync {:.3} ms worst {:.3} over {} publishes",
            self.hud_line(),
            self.step_ms,
            self.worst_step_ms,
            self.steps_simulated as f64 / self.frames.max(1) as f64,
            self.sync_ms,
            self.worst_sync_ms,
            self.syncs,
        )
    }
}

/// Writes one captured frame beside the save, so a host run can prove the
/// spawn pose and the HUD without a phone. Returns the captured extent and the
/// file path; the caller prints the player line rendered into it.
pub fn write_capture(
    renderer: &mut matterweave_render::Renderer,
    matrix: [[f32; 4]; 4],
    eye: [f32; 3],
    hud: &matterweave_render::Hud,
    lighting: &matterweave_render::LightingSettings,
    directory: &Path,
) -> Result<(u32, u32, std::path::PathBuf), String> {
    let frame = renderer.capture_frame(matrix, eye, hud, lighting)?;
    let path = directory.join("world-capture.ppm");
    crate::scale_check::write_ppm(&path, frame.width, frame.height, &frame.rgba)?;
    Ok((frame.width, frame.height, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-world-{}-{}-{name}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Walk in `input` for `seconds` of fixed steps.
    fn walk(player: &mut WorldPlayer, seconds: f32, input: PlayerInput, yaw: f32) {
        let frames = (seconds / FIXED_DT).round() as usize;
        for _ in 0..frames {
            player.step(FIXED_DT, input, yaw);
        }
    }

    /// The highest solid cell of a column in the published window, as the
    /// walkable surface its top face forms.
    fn voxel_surface(world: &World, x: i32, z: i32) -> Option<f32> {
        use matterweave_core::landscape::{MAX_SURFACE_Y, MIN_SURFACE_Y};
        (MIN_SURFACE_Y..=MAX_SURFACE_Y)
            .rev()
            .find(|&y| world.get([x, y, z]) != 0)
            .map(|y| y as f32 + 1.0)
    }

    /// A streamed landscape window with a flat 32x32 platform above the local
    /// terrain, so the movement tests measure the player rather than the
    /// generator's slope. Returns the world, the platform's cell layer and its
    /// walkable top.
    fn platform_world() -> (World, i32, f32) {
        let centre = [32.0, 120.0, 32.0];
        let mut world = World::landscape(SEED);
        world.stream_around(centre);
        let mut highest = -60;
        for x in 16..48 {
            for z in 16..48 {
                highest = highest.max(landscape::height_at(SEED, x, z));
            }
        }
        let layer = highest + 2;
        for x in 16..48 {
            for z in 16..48 {
                world.set([x, layer, z], 7);
            }
        }
        (world, layer, (layer + 1) as f32)
    }

    fn platform_spawn(top: f32) -> Spawn {
        Spawn {
            eye: [20.5, top + EYE_HEIGHT_M + SPAWN_LIFT_M, 32.5],
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -0.05,
        }
    }

    #[test]
    fn the_marker_selects_the_mode_and_the_desktop_flag_wins() {
        let dir = temp_dir("marker");
        assert!(
            !requested(false, &dir),
            "no flag and no marker is not an opt-in"
        );
        assert!(requested(true, &dir), "the desktop flag selects it");
        std::fs::write(dir.join(MARKER_FILE), "").unwrap();
        assert!(requested(false, &dir), "an empty marker selects it");
        // The marker is read with the landscape tolerance: oversized means
        // absent, and a broken marker never stops the app from starting.
        std::fs::write(
            dir.join(MARKER_FILE),
            "x".repeat(crate::landscape::MAX_MARKER_BYTES as usize + 1),
        )
        .unwrap();
        assert!(!requested(false, &dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_spawn_stands_on_the_surface_at_a_coast() {
        let mut world = World::landscape(SEED);
        let spawn = spawn();
        world.stream_around(spawn.eye);
        let player = WorldPlayer::new(&world, spawn).expect("spawn must be standable");
        assert!(player.grounded(), "the first frame must not be a fall");
        let eye = player.eye();
        let (x, z) = (eye.x.floor() as i32, eye.z.floor() as i32);
        let surface = surface_at(x, z);
        assert!(
            (eye.y - (surface + EYE_HEIGHT_M)).abs() < 0.08,
            "eye {} is not {} above the surface {surface}",
            eye.y,
            EYE_HEIGHT_M
        );
        // Worth looking at: low land with open water one 64 m step away.
        assert!(
            (2..=16).contains(&landscape::height_at(SEED, x, z)),
            "spawn ground {} is not low land",
            landscape::height_at(SEED, x, z)
        );
        assert!(
            [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, -1),
                (1, -1),
                (-1, 1)
            ]
            .iter()
            .any(|(dx, dz)| landscape::height_at(SEED, x + dx * 64, z + dz * 64) < -2),
            "spawn has no water within 64 m"
        );
        // Standing still for a second neither sinks nor drifts.
        let mut player = player;
        walk(&mut player, 1.0, PlayerInput::IDLE, spawn.yaw);
        assert!(player.grounded());
        assert!(
            (player.eye().y - eye.y).abs() < 0.05,
            "standing moved the eye from {} to {}",
            eye.y,
            player.eye().y
        );
    }

    #[test]
    fn a_drop_from_four_metres_rests_on_the_surface_without_sinking() {
        let (world, _, top) = platform_world();
        let mut player = WorldPlayer::new(&world, platform_spawn(top)).unwrap();
        assert!(player
            .physics
            .teleport([32.5, top + 4.0 + EYE_HEIGHT_M, 32.5]));
        walk(&mut player, 4.0, PlayerInput::IDLE, 0.0);
        assert!(player.grounded(), "the drop did not land");
        let eye = player.eye();
        let rest = top + EYE_HEIGHT_M;
        // The controller rests the capsule a centimetre above the surface; the
        // capsule must never be below it.
        assert!(
            (eye.y - rest).abs() < 0.02,
            "resting at {} not {rest}",
            eye.y
        );
        assert!(eye.y >= rest - 0.005, "penetrated to {}", eye.y);
    }

    #[test]
    fn a_jump_rises_within_ten_percent_of_the_design_height_and_lands() {
        let (world, _, top) = platform_world();
        let mut player = WorldPlayer::new(&world, platform_spawn(top)).unwrap();
        let stand = player.eye().y;
        player.step(
            FIXED_DT,
            PlayerInput {
                jump: true,
                ..PlayerInput::IDLE
            },
            0.0,
        );
        let mut apex = stand;
        for _ in 0..180 {
            player.step(FIXED_DT, PlayerInput::IDLE, 0.0);
            apex = apex.max(player.eye().y);
        }
        let rise = apex - stand;
        assert!(
            rise >= JUMP_HEIGHT_M * 0.9,
            "apex {rise} m is under 90% of the {JUMP_HEIGHT_M} m design"
        );
        assert!(
            rise <= JUMP_HEIGHT_M * 1.1,
            "apex {rise} m is over 110% of the {JUMP_HEIGHT_M} m design"
        );
        assert!(player.grounded(), "the jump did not land");
        assert!(
            (player.eye().y - stand).abs() < 0.02,
            "landed at {} not {stand}",
            player.eye().y
        );
    }

    /// The highest generator surface under the capsule's footprint: the height
    /// a capsule actually rests on when it overlaps a step edge, and the same
    /// footprint the analytic fallback samples.
    fn footprint_support(x: f32, z: f32) -> f32 {
        let margin = 0.32;
        [
            (x, z),
            (x - margin, z - margin),
            (x + margin, z - margin),
            (x - margin, z + margin),
            (x + margin, z + margin),
        ]
        .iter()
        .map(|&(px, pz)| surface_at(px.floor() as i32, pz.floor() as i32))
        .fold(f32::MIN, f32::max)
    }

    #[test]
    fn a_wall_stops_the_player_and_a_one_voxel_ledge_is_stepped() {
        let (mut world, layer, top) = platform_world();
        // A four-voxel wall: two voxels are already a face the controller can
        // never top from the platform, so walking into it must stop dead.
        for y in 0..4 {
            for z in 16..48 {
                world.set([40, layer + y, z], 7);
            }
        }
        let mut player = WorldPlayer::new(&world, platform_spawn(top)).unwrap();
        player.sync_world(&world);
        let stand = player.eye().y;
        walk(
            &mut player,
            6.0,
            PlayerInput {
                move_z: 1.0,
                ..PlayerInput::IDLE
            },
            std::f32::consts::FRAC_PI_2,
        );
        let eye = player.eye();
        assert!(
            eye.x < 40.0 - 0.25,
            "walked through the wall to x {}",
            eye.x
        );
        assert!(
            (eye.y - stand).abs() < 0.15,
            "climbed the wall from {stand} to {}",
            eye.y
        );
        assert!(player.grounded());

        // A one-voxel ledge on the same platform: one layer higher from x=40,
        // which is a step a person takes. The same controller carries the
        // player over it once the wall is gone.
        for y in 0..4 {
            for z in 16..48 {
                world.set([40, layer + y, z], 0);
            }
        }
        for x in 40..48 {
            for z in 16..48 {
                world.set([x, layer + 1, z], 7);
            }
        }
        let mut player = WorldPlayer::new(&world, platform_spawn(top)).unwrap();
        player.sync_world(&world);
        let stand = player.eye().y;
        walk(
            &mut player,
            5.2,
            PlayerInput {
                move_z: 1.0,
                ..PlayerInput::IDLE
            },
            std::f32::consts::FRAC_PI_2,
        );
        let eye = player.eye();
        assert!(eye.x > 42.0, "did not reach the ledge: x {}", eye.x);
        assert!(
            (eye.y - (stand + 1.0)).abs() < 0.1,
            "step-up ended at {} not {}",
            eye.y,
            stand + 1.0
        );
        assert!(player.grounded());
    }

    #[test]
    fn the_analytic_fallback_meets_the_voxel_surface_at_the_window_edge() {
        let mut world = World::landscape(SEED);
        let spawn = spawn();
        world.stream_around(spawn.eye);
        let centre = [
            (spawn.eye[0] as i32).div_euclid(16),
            (spawn.eye[2] as i32).div_euclid(16),
        ];
        // The +x edge of the window: the first column whose chunk is not
        // resident. The corridor is land with no step over one voxel, so the
        // walk measures the handover rather than a climb.
        let seam = (centre[0] + 4) * 16;
        let corridor = |z: i32| {
            (seam - 6..seam + 6).all(|x| {
                landscape::height_at(SEED, x, z) >= 2
                    && (landscape::height_at(SEED, x + 1, z) - landscape::height_at(SEED, x, z))
                        .abs()
                        <= 1
            })
        };
        let z = (spawn.eye[2] as i32 - 40..=spawn.eye[2] as i32 + 40)
            .find(|&z| corridor(z))
            .expect("the window edge has no walkable corridor");

        // The handover is exact: every resident column's voxel top is the
        // analytic surface the fallback answers for it.
        for x in seam - 4..seam {
            let voxel = voxel_surface(&world, x, z);
            assert_eq!(
                voxel,
                Some(surface_at(x, z)),
                "column ({x},{z}): voxel top {voxel:?} vs analytic {}",
                surface_at(x, z)
            );
        }
        let inside = surface_at(seam - 1, z);
        let outside = surface_at(seam, z);
        assert!(
            (outside - inside).abs() <= 1.0,
            "seam step: inside {inside} m -> outside {outside} m"
        );
        eprintln!(
            "WORLD SEAM: window edge at x {seam}, z {z} | inside column {} voxel top {:?} / analytic {inside} | outside column {seam} analytic {outside}",
            seam - 1,
            voxel_surface(&world, seam - 1, z),
        );

        // Walk across with the window frozen: past the edge only the analytic
        // fallback can carry the player.
        let start = Spawn {
            eye: [
                seam as f32 - 6.0,
                surface_at(seam - 6, z) + EYE_HEIGHT_M + SPAWN_LIFT_M,
                z as f32 + 0.5,
            ],
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -0.05,
        };
        let mut player = WorldPlayer::new(&world, start).unwrap();
        assert!(
            !player.on_analytic_ground(),
            "the walk must start on voxel geometry"
        );
        let mut handed_over = false;
        for _ in 0..360 {
            player.step(
                FIXED_DT,
                PlayerInput {
                    move_z: 1.0,
                    ..PlayerInput::IDLE
                },
                std::f32::consts::FRAC_PI_2,
            );
            handed_over |= player.on_analytic_ground();
            let eye = player.eye();
            let support = footprint_support(eye.x, eye.z);
            // Supported columns sit on the generator surface; a descending
            // voxel step is a short fall, which must never sink below the
            // highest ground under the capsule.
            assert!(
                eye.y >= support + EYE_HEIGHT_M - 0.06,
                "at x {} the eye {} sank under the surface {support}",
                eye.x,
                eye.y
            );
            if player.grounded() {
                assert!(
                    eye.y <= support + EYE_HEIGHT_M + 0.06,
                    "at x {} the supported eye {} floated over the surface {support}",
                    eye.x,
                    eye.y
                );
            }
        }
        assert!(
            handed_over,
            "the walk ended before the analytic fallback carried a step"
        );
        assert!(player.grounded());
        assert!(
            player.eye().x > seam as f32 + 2.0,
            "the walk stopped at x {}",
            player.eye().x
        );
        assert!(
            player.on_analytic_ground(),
            "the far side of the seam must be the analytic surface"
        );
    }

    #[test]
    fn the_fallback_refuses_a_cliff_and_the_eye_stays_in_the_world() {
        let mut world = World::landscape(SEED);
        let spawn = spawn();
        world.stream_around(spawn.eye);
        // A cliff outside the resident window: a column at least two voxels
        // above its neighbour, where only the analytic fallback exists.
        let base = (spawn.eye[0] as i32, spawn.eye[2] as i32);
        let mut cliff = None;
        'scan: for dx in 80..400 {
            for dz in -200..200 {
                let (x, z) = (base.0 + dx, base.1 + dz);
                if landscape::height_at(SEED, x, z) < 2 {
                    continue;
                }
                if landscape::height_at(SEED, x + 1, z) - landscape::height_at(SEED, x, z) >= 2 {
                    cliff = Some((x, z));
                    break 'scan;
                }
            }
        }
        let (x, z) = cliff.expect("the generator has no cliff east of the spawn");
        let low = surface_at(x, z);
        let start = Spawn {
            eye: [
                x as f32 + 0.5,
                low + EYE_HEIGHT_M + SPAWN_LIFT_M,
                z as f32 + 0.5,
            ],
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -0.05,
        };
        let mut player = WorldPlayer::new(&world, start).unwrap();
        assert!(player.grounded());
        assert!(
            player.on_analytic_ground(),
            "outside the window the ground is the analytic surface"
        );
        walk(
            &mut player,
            4.0,
            PlayerInput {
                move_z: 1.0,
                ..PlayerInput::IDLE
            },
            std::f32::consts::FRAC_PI_2,
        );
        let eye = player.eye();
        // The capsule's radius keeps its centre one radius plus the fallback's
        // footprint margin short of the cliff column boundary.
        assert!(
            eye.x < x as f32 + 0.75,
            "climbed or passed the cliff face at x {x}: ended at {}",
            eye.x
        );
        assert!(
            (eye.y - (low + EYE_HEIGHT_M)).abs() < 0.15,
            "the cliff lifted the eye from {} to {}",
            low + EYE_HEIGHT_M,
            eye.y
        );
        assert!(player.grounded());

        // The travel clamp is the landscape's domain, far beyond the legacy
        // ±256 m sandbox, and it keeps the eye inside the renderer's range.
        assert!(player.physics.teleport([BOUNDS_M + 5.0, 400.0, 0.5]));
        player.step(FIXED_DT, PlayerInput::IDLE, 0.0);
        assert!(
            player.eye().x <= BOUNDS_M + 0.85 + 1.0e-3,
            "the eye left the domain at {}",
            player.eye().x
        );
        assert!(BOUNDS_M < (matterweave_core::LANDSCAPE_WORLD_LIMIT - 64) as f32);
    }

    #[test]
    fn the_hud_line_names_ground_contact_speed_and_position() {
        let (world, _, top) = platform_world();
        let mut player = WorldPlayer::new(&world, platform_spawn(top)).unwrap();
        let line = player.hud_line();
        assert!(line.contains("GROUNDED"), "{line}");
        assert!(line.contains("VOXEL"), "{line}");
        assert!(line.contains("SPEED 0.0 M/S"), "{line}");
        assert!(line.contains("POS"), "{line}");
        player.step(
            FIXED_DT,
            PlayerInput {
                move_z: 1.0,
                ..PlayerInput::IDLE
            },
            std::f32::consts::FRAC_PI_2,
        );
        assert!(
            (player.speed_m_s - WALK_SPEED_M_S).abs() < 0.2,
            "walking speed {}",
            player.speed_m_s
        );
        let line = player.hud_line();
        assert!(line.contains("SPEED 4.5 M/S"), "{line}");
    }

    #[test]
    fn the_host_cost_of_a_walking_step_and_a_window_publish_is_measured() {
        // Not a gate: this prints the numbers the PR states, with the
        // conditions attached. The smoke run stands still, so the walk here
        // crosses chunk boundaries to exercise the fixed step and the
        // collision publish on the same path the sample uses.
        let mut world = World::landscape(SEED);
        let spawn = spawn();
        world.stream_around(spawn.eye);
        let started = Instant::now();
        let mut player = WorldPlayer::new(&world, spawn).unwrap();
        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
        let yaw = spawn.yaw + std::f32::consts::FRAC_PI_2;
        for _ in 0..900 {
            player.step(
                FIXED_DT,
                PlayerInput {
                    move_z: 1.0,
                    run: true,
                    ..PlayerInput::IDLE
                },
                yaw,
            );
            // The sample's order: the stream follows the new eye, then the
            // collision publish picks up whatever revision it published.
            world.stream_around(player.eye().to_array());
            player.sync_world(&world);
        }
        let eye = player.eye();
        let travelled = ((eye.x - spawn.eye[0]).hypot(eye.z - spawn.eye[2])) as f64;
        eprintln!(
            "WORLD HOST COST: construction+settle {build_ms:.1} ms | {} frames at run speed, {travelled:.0} m travelled, {} publishes | step {:.3} ms/frame worst {:.3} ({:.2} fixed steps/frame) | collision sync {:.3} ms worst {:.3}",
            player.frames,
            player.syncs,
            player.step_ms,
            player.worst_step_ms,
            player.steps_simulated as f64 / player.frames.max(1) as f64,
            player.sync_ms,
            player.worst_sync_ms,
        );
        assert!(travelled > 50.0, "the walk did not move: {eye:?}");
        assert!(player.grounded(), "the walk did not stay on the ground");
    }

    #[test]
    fn the_world_profile_is_the_one_the_mode_installs() {
        let profile = world_profile();
        assert!(profile.valid());
        assert_eq!(profile.eye_height_m, EYE_HEIGHT_M);
        assert_eq!(profile.jump_height_m, JUMP_HEIGHT_M);
        assert_eq!(profile.autostep_height_m, STEP_UP_M);
        assert!(profile.bounds_m < (matterweave_core::LANDSCAPE_WORLD_LIMIT - 64) as f32);
    }
}
