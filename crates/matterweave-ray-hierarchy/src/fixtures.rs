//! Deterministic fixtures shared by the occupancy, traversal and differential tests.
//!
//! Every ray input is a dyadic rational (integer, half, quarter or sixteenth), so the
//! `f32` inputs of `World::raycast` and this crate's `f64` inputs describe exactly the
//! same geometry. That makes the oracle comparison a test of traversal semantics rather
//! than of float conversion.

use crate::{BlockShape, HierarchyVolume};
use matterweave_core::World;
use matterweave_render::ray_reference::RayVolume;

/// Deterministic 64-bit mixer (SplitMix64). The standard library has no reproducible
/// value generator: `DefaultHasher` is explicitly not stable across releases, and the
/// fixture stream must be reproducible from a seed. The physics destruction tests use
/// the same approach with a local LCG.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `lo..hi`; `lo` is returned when the range is empty.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo) as u64) as i32
    }

    pub fn below(&mut self, limit: u32) -> u32 {
        if limit == 0 {
            0
        } else {
            (self.next_u64() % u64::from(limit)) as u32
        }
    }

    pub fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        values[(self.next_u64() % values.len() as u64) as usize]
    }
}

pub fn palette() -> [[f32; 3]; 256] {
    std::array::from_fn(|index| [index as f32 / 255.0, 0.25, 0.5])
}

/// Reference pack of a crop; panics only on an invalid fixture request.
pub fn pack(world: &World, epoch: u64, origin: [i32; 3], dimensions: [u32; 3]) -> RayVolume {
    RayVolume::pack(world, epoch, origin, dimensions, palette()).expect("valid fixture crop")
}

/// Hierarchy snapshot built from the reference pack of the same crop.
pub fn snapshot(
    world: &World,
    shape: BlockShape,
    origin: [i32; 3],
    dimensions: [u32; 3],
) -> HierarchyVolume {
    HierarchyVolume::from_reference(&pack(world, 1, origin, dimensions), shape)
        .expect("valid fixture snapshot")
}

/// Normalizes exactly as `World::raycast` does, so both sides share one direction.
pub fn oracle_direction(direction: [f64; 3]) -> [f64; 3] {
    let norm = direction.iter().map(|&v| v.powi(2)).sum::<f64>().sqrt();
    direction.map(|v| v / norm)
}

/// Calls the authoritative CPU oracle with the same geometry.
///
/// Fixture inputs are f32-exact dyadic values, so the f32 conversion the oracle's API
/// requires loses nothing and both sides see one identical ray.
pub fn oracle(
    world: &World,
    origin: [f64; 3],
    raw_direction: [f64; 3],
    max_distance: f64,
) -> Option<matterweave_core::RayHit> {
    world.raycast(
        origin.map(|v| v as f32),
        raw_direction.map(|v| v as f32),
        max_distance as f32,
    )
}

/// Bit-for-bit hit comparison between traversal modes: cell, material, normal and the
/// exact `f32` distance.
pub fn assert_same_hit(
    a: Option<matterweave_core::RayHit>,
    b: Option<matterweave_core::RayHit>,
    context: &str,
) {
    match (a, b) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            assert_eq!(x.cell, y.cell, "{context}: cell {x:?} vs {y:?}");
            assert_eq!(x.material, y.material, "{context}: material {x:?} vs {y:?}");
            assert_eq!(x.normal, y.normal, "{context}: normal {x:?} vs {y:?}");
            assert_eq!(
                x.distance.to_bits(),
                y.distance.to_bits(),
                "{context}: distance {x:?} vs {y:?}"
            );
        }
        _ => panic!("{context}: hit/miss mismatch {a:?} vs {b:?}"),
    }
}

/// A world whose content is entirely inside `origin..origin + dimensions`.
pub struct Fixture {
    pub world: World,
    pub origin: [i32; 3],
    pub dimensions: [u32; 3],
}

impl Fixture {
    pub fn right(&self) -> [f64; 3] {
        [
            f64::from(self.origin[0] + self.dimensions[0] as i32),
            f64::from(self.origin[1] + self.dimensions[1] as i32),
            f64::from(self.origin[2] + self.dimensions[2] as i32),
        ]
    }

    pub fn diagonal(&self) -> f64 {
        let [dx, dy, dz] = self.dimensions.map(f64::from);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    pub fn snapshot(&self, shape: BlockShape) -> HierarchyVolume {
        snapshot(&self.world, shape, self.origin, self.dimensions)
    }
}

/// Sparse fixture: plate boxes, isolated voxels and one thin wall with a single opening.
pub fn sparse_fixture(seed: u64) -> Fixture {
    let mut rng = Rng::new(seed);
    let dimensions = [rng.below(19) + 6, rng.below(19) + 6, rng.below(19) + 6];
    let origin = [
        -rng.range(0, 24) + rng.range(0, 4),
        -rng.range(0, 8) + rng.range(0, 4),
        -rng.range(0, 24) + rng.range(0, 4),
    ];
    let mut world = World::new(rng.next_u64());
    // One-cell-thick wall across the crop with a one-cell opening.
    let wall_axis = rng.below(3) as usize;
    let wall_at = rng.below(dimensions[wall_axis]);
    let hole_y = rng.below(dimensions[(wall_axis + 1) % 3]);
    let hole_z = rng.below(dimensions[(wall_axis + 2) % 3]);
    let wall_material = rng.range(1, 256) as u8;
    for a in 0..dimensions[0] {
        for b in 0..dimensions[1] {
            for c in 0..dimensions[2] {
                let cell = [a, b, c];
                if cell[wall_axis] != wall_at {
                    continue;
                }
                if cell[(wall_axis + 1) % 3] == hole_y && cell[(wall_axis + 2) % 3] == hole_z {
                    continue;
                }
                let world_cell = [
                    origin[0] + a as i32,
                    origin[1] + b as i32,
                    origin[2] + c as i32,
                ];
                assert!(world.set(world_cell, wall_material), "wall write");
            }
        }
    }
    // Thin plates.
    for _ in 0..rng.below(3) + 1 {
        let thickness_axis = rng.below(3);
        let mut extent = dimensions.map(|d| rng.below(d / 2 + 1) + 2);
        extent[thickness_axis as usize] = rng.below(3) + 1;
        let start = [
            rng.below(dimensions[0].saturating_sub(extent[0]) + 1),
            rng.below(dimensions[1].saturating_sub(extent[1]) + 1),
            rng.below(dimensions[2].saturating_sub(extent[2]) + 1),
        ];
        let material = rng.range(1, 256) as u8;
        for a in 0..extent[0] {
            for b in 0..extent[1] {
                for c in 0..extent[2] {
                    let cell = [start[0] + a, start[1] + b, start[2] + c];
                    let world_cell = [
                        origin[0] + cell[0] as i32,
                        origin[1] + cell[1] as i32,
                        origin[2] + cell[2] as i32,
                    ];
                    let _ = world.set(world_cell, material);
                }
            }
        }
    }
    // Isolated voxels, including single-cell thin walls.
    for _ in 0..rng.below(7) {
        let world_cell = [
            origin[0] + rng.below(dimensions[0]) as i32,
            origin[1] + rng.below(dimensions[1]) as i32,
            origin[2] + rng.below(dimensions[2]) as i32,
        ];
        let material = rng.range(1, 256) as u8;
        let _ = world.set(world_cell, material);
    }
    Fixture {
        world,
        origin,
        dimensions,
    }
}

/// A fully solid crop; every visited cell reads a nonzero material.
pub fn solid_fixture(dimensions: [u32; 3]) -> Fixture {
    let mut world = World::new(3);
    for x in 0..dimensions[0] {
        for y in 0..dimensions[1] {
            for z in 0..dimensions[2] {
                assert!(world.set([x as i32, y as i32, z as i32], 7), "solid write");
            }
        }
    }
    Fixture {
        world,
        origin: [0, 0, 0],
        dimensions,
    }
}

/// Where a ray starts relative to the cell grid.
#[derive(Clone, Copy, Debug)]
pub enum StartKind {
    /// Cell centers, strictly inside the crop.
    Center,
    /// Exact integer coordinates: on grid planes.
    Integer,
    /// Quarter offsets: exactly representable, close to but off a face.
    Quarter,
    /// Cell centers of a span that reaches outside the crop.
    Outside,
}

pub fn random_ray(rng: &mut Rng, fixture: &Fixture, start: StartKind) -> ([f64; 3], [f64; 3], f64) {
    let offset: f64 = match start {
        StartKind::Center | StartKind::Outside => 0.5,
        StartKind::Integer => 0.0,
        StartKind::Quarter => 0.25,
    };
    let outside = matches!(start, StartKind::Outside);
    let mut origin = [0.0f64; 3];
    for (axis, slot) in origin.iter_mut().enumerate() {
        let span = fixture.dimensions[axis] as i32;
        let high = if outside { span + 3 } else { span };
        let low = if outside { -3 } else { 0 };
        *slot = f64::from(fixture.origin[axis] + rng.range(low, high)) + offset;
    }
    // Dyadic components, including exact ties and zero components.
    let choices = [0.0, 1.0, -1.0, 0.5, -0.5, 0.25, -0.25, 2.0, -3.0, 0.125];
    let raw = [rng.pick(&choices), rng.pick(&choices), rng.pick(&choices)];
    let raw = if raw == [0.0; 3] {
        [1.0, 0.0, 0.0]
    } else {
        raw
    };
    let max_distance = fixture.diagonal() * 1.5;
    (origin, raw, max_distance)
}
