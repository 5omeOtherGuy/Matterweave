//! Deterministic landmark terrain: things worth walking towards.
//!
//! The landscape generator on its own has no destination: every hill looks like
//! the next, and there is nothing on the horizon that says "go there". This
//! module adds a sparse lattice of recognisable features - a spire, a mesa, a
//! stepped pyramid and a crater - that are part of the terrain and not props
//! placed on top of it. A landmark is a shaping function applied to the ground
//! the generator already made, so it is walkable-to, it never floats above the
//! surface and it never punches a hole in it.
//!
//! # Lattice
//!
//! One cell of [`LANDMARK_CELL_M`] holds at most one site. A cell's hash decides
//! whether it has one, which kind it is, and where inside the cell its centre
//! falls; the centre is kept inside the middle half of the cell
//! ([`CENTRE_MIN_OFFSET`]`..`[`CENTRE_MAX_OFFSET`]), which holds any two centres
//! at least [`MIN_CENTRE_SEPARATION_M`] apart - more than twice
//! [`MAX_LANDMARK_RADIUS_M`] - so two footprints can never overlap and a column
//! is shaped by at most one landmark. The roll, the kind and the position are
//! all pure integer functions of `(seed, cell)`: landmarks are world data, the
//! same on every target, with no storage and no pass.
//!
//! # Shape
//!
//! Every kind is a profile in metres from its centre. It reaches zero with zero
//! slope at its footprint radius, and the ground beyond is untouched; inside, the
//! lift is *added to the local ground*, so a landmark follows the terrain
//! instead of sitting on a plinth. Three rules keep it honest:
//!
//! - **No site under water, none near the ceiling.** A site is only shaped when
//!   its own centre's ground can carry its full height: above the wave base and
//!   far enough below [`MAX_SURFACE_Y`](crate::landscape::MAX_SURFACE_Y) that the
//!   range clamp never beheads a summit into a mesa. The test is a function of
//!   the site alone, so the world a player walks and the list a test reads are
//!   the same set of landmarks.
//! - **Slope budget.** The profile's outermost fifth - the apron a player walks
//!   on to reach the landmark - stays under [`APRON_SLOPE_PERMILLE`], and the
//!   flanks inside it stay under [`WALK_SLOPE_PERMILLE`] for every kind, so a
//!   ziggurat's summit, a mesa's top and a crater's rim are all reachable on
//!   foot and the path there is never a cliff.
//!
//! A crater is the one kind that cuts instead of raising; its floor is held
//! above sea level, so it is a dry bowl rather than a pond the water pass would
//! have to explain.

use crate::landscape::{self, MAX_SURFACE_Y, SEA_LEVEL};

/// Lattice cell edge of one landmark site, in metres.
pub const LANDMARK_CELL_M: i32 = 720;
/// Lowest centre offset inside a cell: the centre never sits closer than this
/// to a cell edge, which is what keeps footprints from overlapping.
pub const CENTRE_MIN_OFFSET: i32 = LANDMARK_CELL_M / 4;
/// One past the highest centre offset.
pub const CENTRE_MAX_OFFSET: i32 = LANDMARK_CELL_M * 3 / 4;
/// Distance two neighbouring centres cannot be closer than, in metres: twice
/// the centre offset span.
pub const MIN_CENTRE_SEPARATION_M: i32 = CENTRE_MAX_OFFSET - CENTRE_MIN_OFFSET;
/// Largest footprint radius any kind uses, in metres. Held below half
/// [`MIN_CENTRE_SEPARATION_M`], so a column is shaped by at most one landmark.
pub const MAX_LANDMARK_RADIUS_M: i32 = 160;
/// Sites per thousand cells; sparse enough that a landmark reads as an event.
pub const SITE_RATE_PER_MILLE: i32 = 640;
/// Lift above its own ground at which a landmark's surface is bare rock rather
/// than the biome's cover, in metres.
///
/// A spire wearing the plains' grass is a green hill with a point on it: at two
/// kilometres the cover is most of what a player sees, and the biome's own
/// materials are the only thing that would make a landmark unrecognisable from
/// the ridge beside it. Rock above this lift is what the mountain biome shows
/// anyway, so a landmark in the mountains is unchanged and one on a plain reads
/// as stone rising out of grass.
pub const ROCK_FACE_M: i32 = 10;

/// Room a raised kind keeps below the surface clamp for its own summit, so the
/// clamp never flattens it.
pub const SUMMIT_HEADROOM_M: i32 = 16;
/// Ground above sea level a site's own centre needs before it is built, in
/// metres: a site under water or in the swash is not a landmark, it is a wreck.
pub const BASE_CLEARANCE_M: i32 = 6;
/// Slope the demo's walking player is expected to climb, in per-mille (35
/// degrees). The physics player owns the real number; every landmark flank and
/// apron stays at or below it, so a player that can walk a 35 degree slope can
/// reach every footprint and every summit.
pub const WALK_SLOPE_PERMILLE: i32 = 700;
/// Highest slope any landmark apron reaches, in per-mille (21 degrees): the
/// part of the footprint a player stands on to look up at the landmark.
pub const APRON_SLOPE_PERMILLE: i32 = 400;
/// Where the apron starts, as a fraction of the radius in 256ths.
const APRON_START: i32 = 205;

/// The four kinds of landmark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LandmarkKind {
    /// A tall rock tower: the silhouette a player picks out first.
    Spire,
    /// A flat-topped table with walkable flanks.
    Mesa,
    /// A terraced pyramid whose summit is reachable on foot.
    Ziggurat,
    /// A ring wall around a dry bowl.
    Crater,
}

impl LandmarkKind {
    /// Every kind, for coverage checks and tools.
    pub const ALL: [LandmarkKind; 4] = [
        LandmarkKind::Spire,
        LandmarkKind::Mesa,
        LandmarkKind::Ziggurat,
        LandmarkKind::Crater,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LandmarkKind::Spire => "spire",
            LandmarkKind::Mesa => "mesa",
            LandmarkKind::Ziggurat => "ziggurat",
            LandmarkKind::Crater => "crater",
        }
    }

    /// Footprint radius in metres: how far from the centre the ground is
    /// touched. Beyond it the profile is exactly zero.
    pub fn radius_m(self) -> i32 {
        match self {
            // 80 m for the tower, not 160: a tower as wide as it is tall is a
            // hill, and the height is what carries its silhouette.
            LandmarkKind::Spire => 80,
            LandmarkKind::Mesa => 160,
            LandmarkKind::Ziggurat => 160,
            LandmarkKind::Crater => 160,
        }
    }

    /// Silhouette height in metres: summit above the ground it stands on, or
    /// rim above the ground for a crater.
    ///
    /// Sized so the kind is a silhouette rather than a bump at two kilometres:
    /// at the reference viewport a spire's 110 m subtends 62 pixels there, a
    /// mesa's 80 m forty-five, and a crater's 30 m seventeen.
    pub fn height_m(self) -> i32 {
        match self {
            LandmarkKind::Spire => 110,
            LandmarkKind::Mesa => 80,
            LandmarkKind::Ziggurat => 70,
            LandmarkKind::Crater => 30,
        }
    }

    /// Depth a crater cuts below the ground it stands on, in metres.
    pub fn depth_m(self) -> i32 {
        match self {
            LandmarkKind::Crater => 20,
            _ => 0,
        }
    }

    /// Highest ground a site may stand on, in metres, for the shape to keep its
    /// full height below the surface clamp.
    pub fn max_base_m(self) -> i32 {
        match self {
            LandmarkKind::Crater => MAX_SURFACE_Y - self.height_m(),
            _ => MAX_SURFACE_Y - self.height_m() - SUMMIT_HEADROOM_M,
        }
    }

    /// Lowest ground a site may stand on, in metres.
    pub fn min_base_m(self) -> i32 {
        SEA_LEVEL + BASE_CLEARANCE_M
    }

    /// Largest ground slope the shape ever adds, in per-mille, as an upper
    /// bound the tests check against the generated terrain.
    pub fn max_slope_per_mille(self) -> i32 {
        WALK_SLOPE_PERMILLE
    }

    /// Signed lift at distance `d` from the centre, in metres, before the fade:
    /// the profile with its apron taper.
    ///
    /// The core profile never reaches further than [`APRON_START`] of the
    /// radius, and [`apron`] takes whatever is left there to zero, so the shape
    /// the player walks on is gentle without flattening the landmark behind it.
    fn lift(self, d: i32) -> i32 {
        let r = self.radius_m();
        if d >= r {
            return 0;
        }
        (self.core(d) * apron(d, r)) >> 8
    }

    /// The profile before the apron taper, in metres.
    fn core(self, d: i32) -> i32 {
        match self {
            LandmarkKind::Spire => {
                // A tower: a 12 m summit plateau, one straight flank at a 2.1
                // grade, and a 3 m shoulder where the apron takes over.
                const PLATEAU_M: i32 = 12;
                const SHOULDER_M: i32 = 3;
                let shoulder_at = self.radius_m() * APRON_START / 256;
                if d <= PLATEAU_M {
                    self.height_m()
                } else if d <= shoulder_at {
                    let h = self.height_m();
                    h - (h - SHOULDER_M) * (d - PLATEAU_M) / (shoulder_at - PLATEAU_M)
                } else {
                    // Flat shoulder: the apron taper owns everything past here.
                    SHOULDER_M
                }
            }
            LandmarkKind::Mesa => {
                // A table: flat top over the inner 40 m, then a 0.7-grade flank
                // onto an 8 m shoulder.
                const PLATEAU_M: i32 = 40;
                const SHOULDER_M: i32 = 8;
                let shoulder_at = self.radius_m() * APRON_START / 256;
                if d <= PLATEAU_M {
                    self.height_m()
                } else if d <= shoulder_at {
                    let h = self.height_m();
                    h - (h - SHOULDER_M) * (d - PLATEAU_M) / (shoulder_at - PLATEAU_M)
                } else {
                    SHOULDER_M
                }
            }
            LandmarkKind::Ziggurat => {
                // Five terraces of 14 m: a 7 m tread and a 21 m riser at a 0.67
                // grade, so every step is a walk rather than a climb and the
                // summit is a destination a player can stand on.
                const SUMMIT_R: i32 = 20;
                const TREAD_M: i32 = 7;
                const RISER_M: i32 = 21;
                const RISE_M: i32 = 14;
                const STEPS: i32 = 5;
                let terrace = TREAD_M + RISER_M;
                if d <= SUMMIT_R {
                    return self.height_m();
                }
                if d >= SUMMIT_R + STEPS * terrace {
                    return 0;
                }
                let step = (d - SUMMIT_R) / terrace;
                let within = (d - SUMMIT_R) % terrace;
                let top = self.height_m() - step * RISE_M;
                if within <= TREAD_M {
                    top
                } else {
                    top - (within - TREAD_M) * RISE_M / RISER_M
                }
            }
            LandmarkKind::Crater => {
                // Dry bowl: floor, inner wall at 0.69, rim just inside 0.6r,
                // outer wall at 0.67 onto an 8 m shoulder.
                const FLOOR_R: i32 = 22;
                const SHOULDER_M: i32 = 8;
                let rim_r = self.radius_m() * 19 / 32;
                let shoulder_at = self.radius_m() * APRON_START / 256;
                let depth = self.depth_m();
                let h = self.height_m();
                if d <= FLOOR_R {
                    -depth
                } else if d <= rim_r {
                    -depth + (h + depth) * (d - FLOOR_R) / (rim_r - FLOOR_R)
                } else if d <= shoulder_at {
                    h - (h - SHOULDER_M) * (d - rim_r) / (shoulder_at - rim_r)
                } else {
                    SHOULDER_M
                }
            }
        }
    }
}

/// Smooth `1` at `APRON_START` of the radius, `0` with zero slope at the rim,
/// in 256ths. A cubic rather than a quadratic: a quadratic foot is steepest
/// where it meets the flank, which is the opposite of what a walk-in needs.
fn apron(d: i32, r: i32) -> i32 {
    let start = r * APRON_START / 256;
    if d <= start {
        return 256;
    }
    let run = (r - start).max(1) as i64;
    let u = (((d - start) as i64 * 256) / run).min(256);
    let square = u * u;
    let cube = square * u;
    (256 - 3 * square / 256 + 2 * cube / 65536) as i32
}

/// One landmark site: a kind and a centre, before any ground is consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landmark {
    pub kind: LandmarkKind,
    pub centre: [i32; 2],
}

impl Landmark {
    pub fn radius_m(&self) -> i32 {
        self.kind.radius_m()
    }

    pub fn height_m(&self) -> i32 {
        self.kind.height_m()
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    /// Euclidean distance from the centre, in metres.
    pub fn range_from(&self, x: i32, z: i32) -> i32 {
        let dx = (self.centre[0] - x) as i64;
        let dz = (self.centre[1] - z) as i64;
        ((dx * dx + dz * dz) as u64).isqrt() as i32
    }
}

/// The site a lattice cell holds, or `None`.
///
/// Pure integer function of `(seed, cell)`: no ground is sampled, so this stays
/// cheap enough to ask of nine cells for every column the generator produces.
pub fn site(seed: u64, cell_x: i32, cell_z: i32) -> Option<Landmark> {
    let roll = landscape::hash3(seed ^ SALT_LANDMARK, cell_x, cell_z);
    if (roll & 0xFFFF) as i32 >= SITE_RATE_PER_MILLE * 65536 / 1000 {
        return None;
    }
    // Spires are the kind a player notices from furthest away, so they lead.
    let kind = match (roll >> 20) % 100 {
        0..=34 => LandmarkKind::Spire,
        35..=59 => LandmarkKind::Mesa,
        60..=79 => LandmarkKind::Ziggurat,
        _ => LandmarkKind::Crater,
    };
    let span = (CENTRE_MAX_OFFSET - CENTRE_MIN_OFFSET) as u64;
    let offset_x = CENTRE_MIN_OFFSET + (((roll >> 24) & 0xFF) * span / 256) as i32;
    let offset_z = CENTRE_MIN_OFFSET + (((roll >> 40) & 0xFF) * span / 256) as i32;
    Some(Landmark {
        kind,
        centre: [
            cell_x * LANDMARK_CELL_M + offset_x,
            cell_z * LANDMARK_CELL_M + offset_z,
        ],
    })
}

/// A site whose own ground can carry its full height, or `None`.
///
/// This is what "a landmark is here" means to a test, a tool or the demo's
/// spawn search: the site's centre is above the shore fade band, below the
/// height that would let the range clamp cut its summit, and on land.
/// [`shape`] is the same content found from the other side - it reads the
/// column's own ground instead of the centre's - so a site that fails this test
/// still shapes its columns, faded by the ground it finds there.
pub fn landmark_at_cell(seed: u64, cell_x: i32, cell_z: i32) -> Option<Landmark> {
    let landmark = site(seed, cell_x, cell_z)?;
    site_valid(seed, &landmark).then_some(landmark)
}

/// Whether a site's own centre can carry its full height.
///
/// The one ground sample the landmark system needs, taken at the site and not at
/// the column being shaped, so a footprint never depends on how the ground
/// happens to wander under it.
pub fn site_valid(seed: u64, landmark: &Landmark) -> bool {
    let ground = landscape::base_height(seed, landmark.centre[0], landmark.centre[1]);
    (landmark.kind.min_base_m()..=landmark.kind.max_base_m()).contains(&ground)
}

/// The lattice cell containing a coordinate.
fn cell_of(value: i32) -> i32 {
    value.div_euclid(LANDMARK_CELL_M)
}

/// The landmark whose footprint covers `(x, z)`, with its distance, if any.
///
/// At most one site can, but the lattice means the candidate sits in the cell
/// containing the column or one of its eight neighbours, and each candidate is
/// rejected by arithmetic before its hash is even rolled.
pub fn covering(seed: u64, x: i32, z: i32) -> Option<(Landmark, i32)> {
    let (cx, cz) = (cell_of(x), cell_of(z));
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (cell_x, cell_z) = (cx + dx, cz + dz);
            // Cheapest possible rejection: how far the column can be from any
            // centre this cell is allowed to hold, before rolling anything.
            let low_x = cell_x * LANDMARK_CELL_M + CENTRE_MIN_OFFSET;
            let high_x = cell_x * LANDMARK_CELL_M + CENTRE_MAX_OFFSET;
            let low_z = cell_z * LANDMARK_CELL_M + CENTRE_MIN_OFFSET;
            let high_z = cell_z * LANDMARK_CELL_M + CENTRE_MAX_OFFSET;
            let gap_x = (low_x - x).max(0).max(x - high_x) as i64;
            let gap_z = (low_z - z).max(0).max(z - high_z) as i64;
            if gap_x * gap_x + gap_z * gap_z > (MAX_LANDMARK_RADIUS_M as i64).pow(2) {
                continue;
            }
            let Some(landmark) = site(seed, cell_x, cell_z) else {
                continue;
            };
            let d = landmark.range_from(x, z);
            if d <= landmark.radius_m() {
                return Some((landmark, d));
            }
        }
    }
    None
}

/// Shape one column's ground with whatever landmark footprint covers it.
///
/// `ground` is the unshaped height from the climate and relief fields; the
/// return value is what the world stores. Called once per metre-column, so it
/// touches no other column and allocates nothing.
pub fn shape(seed: u64, x: i32, z: i32, ground: i32) -> i32 {
    let Some((landmark, d)) = covering(seed, x, z) else {
        return ground;
    };
    if !site_valid(seed, &landmark) {
        return ground;
    }
    let lift = landmark.kind.lift(d);
    if landmark.kind == LandmarkKind::Crater {
        // A bowl is held at one metre above sea level where its ground is dry,
        // so it stays a dry bowl and the water level stays the generator's
        // single sea plane. Ground that is already below sea level is left
        // alone: a crater that reaches the shore must not fill the sea in.
        let floor = (SEA_LEVEL + 1).min(ground);
        ground.saturating_add(lift).max(floor)
    } else {
        ground.saturating_add(lift)
    }
}

/// Whether `(x, z)` is on the walkable apron of a landmark: the outermost part
/// of its footprint, where a player stands to look at it.
pub fn on_apron(seed: u64, x: i32, z: i32) -> Option<Landmark> {
    let (landmark, d) = covering(seed, x, z)?;
    let start = landmark.radius_m() * APRON_START / 256;
    (d >= start).then_some(landmark)
}

/// Every site inside a square around a point, nearest first.
///
/// The demo and the acceptance tests use this to name what is on the horizon;
/// the generator itself never calls it.
pub fn sites_around(seed: u64, centre: [i32; 2], half_extent_m: i32) -> Vec<Landmark> {
    let cell_span = half_extent_m.div_euclid(LANDMARK_CELL_M) + 2;
    let (cx, cz) = (cell_of(centre[0]), cell_of(centre[1]));
    let mut found: Vec<(i32, Landmark)> = Vec::new();
    for dz in -cell_span..=cell_span {
        for dx in -cell_span..=cell_span {
            if let Some(landmark) = landmark_at_cell(seed, cx + dx, cz + dz) {
                let distance = landmark.range_from(centre[0], centre[1]);
                if distance <= half_extent_m {
                    found.push((distance, landmark));
                }
            }
        }
    }
    found.sort_by_key(|(distance, landmark)| (*distance, landmark.centre));
    found.into_iter().map(|(_, landmark)| landmark).collect()
}

/// Salt for the landmark lattice, distinct from every landscape field.
const SALT_LANDMARK: u64 = 0x6c62_272e_5d17_1d0f;

// -- The demo world ----------------------------------------------------------
//
// The explorer's landscape sample opens here. The spawn is a fixed point, not a
// search at run time: a named position is what makes a capture, a measurement
// and a report about the same place reproducible. The acceptance tests in
// `crates/matterweave-core/tests/landmarks.rs` re-derive every claim below from
// this seed, so a generator change that moves the framing fails a test instead
// of quietly shipping a different demo.

/// Seed of the demo world.
pub const DEMO_SEED: u64 = 20260913;

/// The demo's spawn, in world metres: `[x, ground y, z]`. It stands on plains
/// ground 77 m above sea level, with tundra ground 4 m away and stony mountain
/// ground 16 m away, so three visually distinct ground materials are inside a
/// short walk, and every step around it is one voxel or less.
pub const DEMO_SPAWN: [i32; 3] = [5632, 77, 1536];

/// The bearing the demo opens on, in degrees clockwise from +Z. It looks down a
/// coastline with the sea on its left and a mountain massif on its right, and it
/// frames a rock spire 447 m out and a ziggurat at 1114 m, each clearing the
/// ridge between it and the eye by nine metres or more, with the meadow-to-stone
/// transition beginning inside its first 20 m.
pub const DEMO_YAW_DEGREES: i32 = 220;

/// Eye height above the ground of the demo's spawn camera, in metres. The
/// sample's camera flies when it opens, eight metres above the ground it spawned
/// on, which is the eye every "seen from spawn" measurement in the acceptance
/// tests is taken from.
pub const DEMO_EYE_ABOVE_GROUND_M: i32 = 8;
