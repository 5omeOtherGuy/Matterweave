//! Voxel creature models. Each species is a compact box-list built from a body
//! plan, palette and feature set; evolved forms add mass and features rather
//! than a palette swap. Meshes feed the battle arena through the renderer's
//! ordinary mesh path.
use crate::roster::{species, Affinity, SpeciesId};
use matterweave_core::Mesh;

/// One axis-aligned voxel box in model space (one unit is one voxel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelBox {
    pub min: [i16; 3],
    pub size: [u8; 3],
    /// Palette slot: 0 base, 1 dark, 2 accent, 3 eye.
    pub slot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyPlan {
    Quadruped,
    Biped,
    Bird,
    Fish,
    Moth,
    Wisp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarKind {
    None,
    Short,
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailKind {
    None,
    Short,
    Bush,
    Fan,
    Plume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    pub ears: EarKind,
    pub tail: TailKind,
    pub horns: bool,
    pub wings: bool,
    pub fins: bool,
    pub spikes: bool,
    pub shell: bool,
}


const fn feat(
    ears: EarKind,
    tail: TailKind,
    horns: bool,
    wings: bool,
    fins: bool,
    spikes: bool,
    shell: bool,
) -> Features {
    Features {
        ears,
        tail,
        horns,
        wings,
        fins,
        spikes,
        shell,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModelSpec {
    pub plan: BodyPlan,
    pub scale: f32,
    pub body: [u8; 3],
    pub legs: u8,
    pub features: Features,
    pub palette: [[u8; 3]; 4],
}

/// Base hue per affinity; the species accent keeps family members distinct.
fn base_hue(affinity: Affinity) -> [u8; 3] {
    match affinity {
        Affinity::Ember => [196, 76, 48],
        Affinity::Tide => [56, 112, 196],
        Affinity::Verdant => [82, 158, 72],
        Affinity::Stone => [138, 122, 102],
        Affinity::Gale => [168, 206, 216],
        Affinity::Spark => [222, 196, 64],
        Affinity::Gloom => [104, 72, 148],
        Affinity::Lumen => [240, 224, 160],
        Affinity::Beast => [188, 150, 108],
    }
}

/// (plan, scale, body, legs, features, accent) in roster id order. The eye
/// colour is warm for Ember/Lumen lines and cool otherwise.
type SpecRow = (BodyPlan, f32, [u8; 3], u8, Features, [u8; 3]);

const ROWS: [SpecRow; 30] = [
    // Starters: Ember bipeds, Tide quadrupeds and a serpent, Verdant quadrupeds.
    (
        BodyPlan::Biped,
        0.90,
        [7, 8, 9],
        2,
        feat(
            EarKind::Short,
            TailKind::Plume,
            false,
            false,
            false,
            false,
            false,
        ),
        [255, 170, 40],
    ),
    (
        BodyPlan::Biped,
        1.15,
        [9, 10, 11],
        2,
        feat(
            EarKind::Short,
            TailKind::Plume,
            true,
            false,
            false,
            false,
            false,
        ),
        [255, 196, 80],
    ),
    (
        BodyPlan::Biped,
        1.40,
        [11, 12, 13],
        2,
        feat(
            EarKind::Short,
            TailKind::Plume,
            true,
            true,
            false,
            false,
            false,
        ),
        [255, 214, 120],
    ),
    (
        BodyPlan::Quadruped,
        0.90,
        [7, 7, 9],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            true,
            false,
            false,
        ),
        [230, 240, 250],
    ),
    (
        BodyPlan::Quadruped,
        1.15,
        [9, 8, 10],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            true,
            false,
            true,
        ),
        [210, 230, 250],
    ),
    (
        BodyPlan::Fish,
        1.40,
        [10, 9, 14],
        0,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            false,
            true,
            true,
            false,
        ),
        [180, 220, 255],
    ),
    (
        BodyPlan::Quadruped,
        0.90,
        [7, 7, 9],
        4,
        feat(
            EarKind::Long,
            TailKind::Bush,
            false,
            false,
            false,
            true,
            false,
        ),
        [240, 240, 200],
    ),
    (
        BodyPlan::Biped,
        1.15,
        [9, 10, 11],
        2,
        feat(
            EarKind::Long,
            TailKind::Plume,
            true,
            false,
            false,
            true,
            false,
        ),
        [220, 235, 180],
    ),
    (
        BodyPlan::Quadruped,
        1.40,
        [11, 10, 13],
        4,
        feat(
            EarKind::Long,
            TailKind::Bush,
            true,
            false,
            false,
            true,
            true,
        ),
        [200, 230, 160],
    ),
    // Three-stage wild families.
    (
        BodyPlan::Quadruped,
        0.70,
        [6, 5, 7],
        4,
        feat(
            EarKind::Long,
            TailKind::Short,
            false,
            false,
            false,
            false,
            false,
        ),
        [210, 170, 120],
    ),
    (
        BodyPlan::Quadruped,
        0.95,
        [8, 7, 9],
        4,
        feat(
            EarKind::Long,
            TailKind::Bush,
            false,
            false,
            false,
            true,
            false,
        ),
        [170, 130, 90],
    ),
    (
        BodyPlan::Quadruped,
        1.25,
        [10, 9, 12],
        4,
        feat(
            EarKind::Long,
            TailKind::Short,
            false,
            false,
            false,
            true,
            false,
        ),
        [140, 105, 70],
    ),
    (
        BodyPlan::Fish,
        0.80,
        [7, 6, 10],
        0,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            false,
            true,
            false,
            false,
        ),
        [90, 200, 210],
    ),
    (
        BodyPlan::Fish,
        1.10,
        [9, 8, 13],
        0,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            false,
            true,
            true,
            false,
        ),
        [70, 180, 200],
    ),
    (
        BodyPlan::Fish,
        1.40,
        [11, 10, 16],
        0,
        feat(EarKind::None, TailKind::Fan, false, true, true, true, false),
        [50, 160, 190],
    ),
    (
        BodyPlan::Quadruped,
        0.80,
        [7, 6, 8],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            false,
            true,
            false,
        ),
        [190, 170, 140],
    ),
    (
        BodyPlan::Quadruped,
        1.10,
        [9, 8, 10],
        4,
        feat(
            EarKind::Short,
            TailKind::Bush,
            true,
            false,
            false,
            true,
            false,
        ),
        [165, 145, 120],
    ),
    (
        BodyPlan::Quadruped,
        1.45,
        [12, 11, 14],
        4,
        feat(
            EarKind::Short,
            TailKind::Bush,
            true,
            false,
            false,
            true,
            true,
        ),
        [140, 120, 100],
    ),
    (
        BodyPlan::Wisp,
        0.80,
        [6, 6, 6],
        0,
        feat(
            EarKind::None,
            TailKind::None,
            false,
            false,
            false,
            true,
            false,
        ),
        [250, 240, 180],
    ),
    (
        BodyPlan::Wisp,
        1.10,
        [7, 7, 7],
        0,
        feat(
            EarKind::None,
            TailKind::None,
            false,
            false,
            false,
            true,
            false,
        ),
        [250, 230, 150],
    ),
    (
        BodyPlan::Wisp,
        1.45,
        [8, 8, 8],
        0,
        feat(
            EarKind::None,
            TailKind::None,
            false,
            true,
            false,
            true,
            false,
        ),
        [255, 220, 120],
    ),
    // Two-stage wild families.
    (
        BodyPlan::Bird,
        0.70,
        [6, 5, 7],
        2,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            false,
            false,
            false,
            false,
        ),
        [220, 235, 245],
    ),
    (
        BodyPlan::Bird,
        1.05,
        [8, 6, 10],
        2,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            true,
            false,
            false,
            false,
        ),
        [200, 225, 240],
    ),
    (
        BodyPlan::Quadruped,
        0.75,
        [6, 5, 8],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            false,
            false,
            true,
        ),
        [110, 170, 90],
    ),
    (
        BodyPlan::Quadruped,
        1.20,
        [9, 8, 11],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            false,
            true,
            true,
        ),
        [95, 150, 80],
    ),
    (
        BodyPlan::Moth,
        0.85,
        [6, 5, 8],
        0,
        feat(
            EarKind::None,
            TailKind::Plume,
            false,
            true,
            false,
            false,
            false,
        ),
        [230, 120, 60],
    ),
    (
        BodyPlan::Moth,
        1.25,
        [8, 6, 10],
        0,
        feat(
            EarKind::None,
            TailKind::Plume,
            false,
            true,
            false,
            true,
            false,
        ),
        [210, 90, 40],
    ),
    (
        BodyPlan::Bird,
        0.80,
        [6, 5, 8],
        2,
        feat(
            EarKind::None,
            TailKind::Fan,
            false,
            true,
            false,
            false,
            false,
        ),
        [170, 160, 145],
    ),
    (
        BodyPlan::Bird,
        1.25,
        [9, 7, 11],
        2,
        feat(
            EarKind::None,
            TailKind::Fan,
            true,
            true,
            false,
            false,
            false,
        ),
        [150, 140, 125],
    ),
    // Single non-evolving form.
    (
        BodyPlan::Quadruped,
        0.90,
        [8, 6, 9],
        4,
        feat(
            EarKind::Short,
            TailKind::Short,
            false,
            false,
            false,
            true,
            false,
        ),
        [205, 185, 150],
    ),
];

/// Build the full spec for one species; panics only on an unknown id, which
/// save validation already excludes.
pub fn spec_for(id: SpeciesId) -> ModelSpec {
    let data = species(id).unwrap_or_else(|| panic!("unknown species {}", id.0));
    let row = ROWS[id.0 as usize - 1];
    let base = base_hue(data.affinity);
    let dark = [base[0] / 2, base[1] / 2, base[2] / 2];
    let eye = if matches!(
        data.affinity,
        Affinity::Ember | Affinity::Lumen | Affinity::Spark
    ) {
        [250, 220, 120]
    } else {
        [24, 28, 40]
    };
    ModelSpec {
        plan: row.0,
        scale: row.1,
        body: row.2,
        legs: row.3,
        features: row.4,
        palette: [base, dark, row.5, eye],
    }
}

/// Build the box list for one species.
pub fn model_boxes(spec: &ModelSpec) -> Vec<VoxelBox> {
    let mut boxes = Vec::new();
    let b = spec.body;
    let (w, h, d) = (b[0] as i16, b[1] as i16, b[2] as i16);
    match spec.plan {
        BodyPlan::Quadruped => {
            boxes.push(VoxelBox {
                min: [-w / 2, h / 2, -d / 2],
                size: [w as u8, h as u8, d as u8],
                slot: 0,
            });
            let head = (w as u8 / 2 + 2).max(4);
            boxes.push(VoxelBox {
                min: [w / 2 - 1, h / 2 + h / 3, d / 2 - head as i16],
                size: [head, head, head],
                slot: 0,
            });
            let pair = spec.legs.max(2) / 2;
            for side in [-1i16, 1] {
                for j in 0..pair {
                    boxes.push(VoxelBox {
                        min: [
                            -w / 2 + j as i16 * (w - 2).max(2) / pair.max(1) as i16,
                            -h / 2,
                            side * (d / 2 - 2),
                        ],
                        size: [2, h as u8 / 2 + 1, 2],
                        slot: 1,
                    });
                }
            }
            if spec.features.ears != EarKind::None {
                let ear = if spec.features.ears == EarKind::Long {
                    4
                } else {
                    2
                };
                boxes.push(VoxelBox {
                    min: [w / 2 - 1, h + h / 3, d / 2 - 3],
                    size: [ear, ear, 2],
                    slot: 2,
                });
            }
            if spec.features.horns {
                boxes.push(VoxelBox {
                    min: [w / 2, h + h / 2, d / 2 - 1],
                    size: [2, 4, 2],
                    slot: 2,
                });
            }
            if spec.features.tail != TailKind::None {
                let len = match spec.features.tail {
                    TailKind::Bush => 4,
                    TailKind::Fan => 5,
                    _ => 3,
                };
                boxes.push(VoxelBox {
                    min: [-w / 2 - len, h / 2, -2],
                    size: [len as u8, 3, 4],
                    slot: if spec.features.tail == TailKind::Bush {
                        2
                    } else {
                        0
                    },
                });
            }
            if spec.features.shell {
                boxes.push(VoxelBox {
                    min: [-w / 2, h + h / 2, -d / 2],
                    size: [w as u8, 3, d as u8],
                    slot: 1,
                });
            }
            if spec.features.spikes {
                for k in 0..3 {
                    boxes.push(VoxelBox {
                        min: [-w / 2 + 1 + k * 3, h + h / 2, -1],
                        size: [1, 3, 1],
                        slot: 2,
                    });
                }
            }
            if spec.features.fins {
                boxes.push(VoxelBox {
                    min: [-2, h + h / 2, -d / 2 - 3],
                    size: [4, 2, 3],
                    slot: 2,
                });
            }
        }
        BodyPlan::Biped => {
            let legs = spec.legs.max(2) as i16;
            boxes.push(VoxelBox {
                min: [-w / 2, legs, -d / 2],
                size: [w as u8, (h + 2) as u8, d as u8],
                slot: 0,
            });
            let head = (w as u8 / 2 + 2).max(4);
            boxes.push(VoxelBox {
                min: [-(head as i16) / 2, legs + h + 2, 0],
                size: [head, head, head],
                slot: 0,
            });
            for side in [-1i16, 1] {
                boxes.push(VoxelBox {
                    min: [side * (w / 2 - 1), 1, -2],
                    size: [3, legs as u8, 4],
                    slot: 1,
                });
                boxes.push(VoxelBox {
                    min: [side * (w / 2), legs + 1, -1],
                    size: [3, h as u8 / 2, 3],
                    slot: 1,
                });
            }
            if spec.features.ears != EarKind::None {
                let ear = if spec.features.ears == EarKind::Long {
                    4
                } else {
                    2
                };
                boxes.push(VoxelBox {
                    min: [-(head as i16) / 2, legs + h + 2 + head as i16, 0],
                    size: [ear, ear, 2],
                    slot: 2,
                });
            }
            if spec.features.horns {
                boxes.push(VoxelBox {
                    min: [-(head as i16) / 2 - 1, legs + head as i16 + h + 2, 0],
                    size: [head + 2, 3, 2],
                    slot: 2,
                });
            }
            if spec.features.tail != TailKind::None {
                let len = if spec.features.tail == TailKind::Plume {
                    4
                } else {
                    3
                };
                boxes.push(VoxelBox {
                    min: [-w / 2 - len, legs + 1, -2],
                    size: [len as u8, 2, 3],
                    slot: 2,
                });
            }
            if spec.features.wings {
                for side in [-1i16, 1] {
                    boxes.push(VoxelBox {
                        min: [side * (w / 2 + 2), legs + h / 2, -1],
                        size: [4, h as u8 + 2, 2],
                        slot: 2,
                    });
                }
            }
            if spec.features.spikes {
                boxes.push(VoxelBox {
                    min: [-w / 2 + 1, legs + h + 2, -d / 2],
                    size: [(w - 2) as u8, 2, 1],
                    slot: 2,
                });
            }
        }
        BodyPlan::Bird => {
            boxes.push(VoxelBox {
                min: [-w / 2, 4, -d / 2 - 1],
                size: [w as u8, h as u8, (d + 2) as u8],
                slot: 0,
            });
            boxes.push(VoxelBox {
                min: [-2, 4 + h - 1, d / 2],
                size: [5, 5, 4],
                slot: 0,
            });
            boxes.push(VoxelBox {
                min: [-1, 4 + h, d / 2 + 4],
                size: [2, 2, 2],
                slot: 2,
            });
            for side in [-1i16, 1] {
                boxes.push(VoxelBox {
                    min: [side * 2 - 1, 0, 0],
                    size: [2, 4, 2],
                    slot: 2,
                });
                boxes.push(VoxelBox {
                    min: [side * (w / 2 + 3), 5, -2],
                    size: [5, if spec.features.wings { 3 } else { 2 }, 6],
                    slot: 1,
                });
            }
            boxes.push(VoxelBox {
                min: [-3, 5, -d / 2 - 4],
                size: [6, 2, 4],
                slot: 2,
            });
            if spec.features.horns {
                boxes.push(VoxelBox {
                    min: [-2, 4 + h + 4, d / 2 + 1],
                    size: [4, 2, 2],
                    slot: 2,
                });
            }
        }
        BodyPlan::Fish => {
            boxes.push(VoxelBox {
                min: [-w / 2, 2, -d / 2 - 2],
                size: [w as u8, h as u8, (d + 4) as u8],
                slot: 0,
            });
            boxes.push(VoxelBox {
                min: [-w / 2 + 1, 2 + h / 2, d / 2 + 2],
                size: [2, 3, 3],
                slot: 3,
            });
            if spec.features.fins {
                boxes.push(VoxelBox {
                    min: [-1, 2 + h, -1],
                    size: [2, 4, 5],
                    slot: 2,
                });
                for side in [-1i16, 1] {
                    boxes.push(VoxelBox {
                        min: [side * (w / 2 + 2), 3, -2],
                        size: [3, 2, 4],
                        slot: 1,
                    });
                }
            }
            boxes.push(VoxelBox {
                min: [-3, 2, -d / 2 - 6],
                size: [6, h as u8 + 1, 4],
                slot: 2,
            });
            if spec.features.spikes {
                boxes.push(VoxelBox {
                    min: [-w / 2, 2 + h, -2],
                    size: [w as u8, 2, 1],
                    slot: 2,
                });
            }
        }
        BodyPlan::Moth => {
            boxes.push(VoxelBox {
                min: [-2, 5, -4],
                size: [5, 4, 9],
                slot: 0,
            });
            boxes.push(VoxelBox {
                min: [-2, 5, 5],
                size: [4, 4, 4],
                slot: 0,
            });
            const WING: u8 = 10;
            for side in [-1i16, 1] {
                boxes.push(VoxelBox {
                    min: [side * 3 - (if side < 0 { WING as i16 } else { 0 }), 6, -3],
                    size: [WING, 1, 7],
                    slot: 2,
                });
                boxes.push(VoxelBox {
                    min: [side * 4 - 1, 5, 9],
                    size: [2, 2, 4],
                    slot: 2,
                });
                boxes.push(VoxelBox {
                    min: [side * 3 - 1, 0, -3],
                    size: [2, 5, 2],
                    slot: 1,
                });
            }
        }
        BodyPlan::Wisp => {
            boxes.push(VoxelBox {
                min: [-3, 6, -3],
                size: [6, 6, 6],
                slot: 0,
            });
            boxes.push(VoxelBox {
                min: [-2, 7, -2],
                size: [4, 4, 4],
                slot: 3,
            });
            for (dx, dy, dz) in [
                (-6i16, 3i16, 0i16),
                (6, 3, 0),
                (0, 3, -6),
                (0, 3, 6),
                (0, 13, 0),
            ] {
                boxes.push(VoxelBox {
                    min: [dx, dy, dz],
                    size: [2, 2, 2],
                    slot: 2,
                });
            }
            if spec.features.spikes {
                for k in 0..4 {
                    boxes.push(VoxelBox {
                        min: [-5 + k * 3, 12, -1],
                        size: [1, 3, 1],
                        slot: 2,
                    });
                }
            }
        }
    }
    boxes
}

/// Build a world-unit triangle mesh for one species. `unit` is metres per
/// voxel; battle presentation uses a small unit so even large forms stay
/// arena-scale.
pub fn mesh_for(id: SpeciesId, unit: f32) -> Mesh {
    let spec = spec_for(id);
    let boxes = model_boxes(&spec);
    let scale = unit * spec.scale;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for b in &boxes {
        let c = spec.palette[b.slot as usize % 4];
        let color = [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ];
        let min = [
            b.min[0] as f32 * scale,
            b.min[1] as f32 * scale,
            b.min[2] as f32 * scale,
        ];
        let max = [
            (b.min[0] + b.size[0] as i16) as f32 * scale,
            (b.min[1] + b.size[1] as i16) as f32 * scale,
            (b.min[2] + b.size[2] as i16) as f32 * scale,
        ];
        push_box(&mut vertices, &mut indices, min, max, color);
    }
    Mesh {
        vertices,
        indices,
        revision: 0,
    }
}

fn push_box(
    vertices: &mut Vec<matterweave_core::Vertex>,
    indices: &mut Vec<u32>,
    min: [f32; 3],
    max: [f32; 3],
    color: [f32; 3],
) {
    const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let center = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let half = [
        (max[0] - min[0]) / 2.0,
        (max[1] - min[1]) / 2.0,
        (max[2] - min[2]) / 2.0,
    ];
    for (normal, u, v) in FACES {
        let base = vertices.len() as u32;
        let face_center = [
            center[0] + normal[0] * half[0],
            center[1] + normal[1] * half[1],
            center[2] + normal[2] * half[2],
        ];
        for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let half_u = u[0].abs() * half[0] + u[1].abs() * half[1] + u[2].abs() * half[2];
            let half_v = v[0].abs() * half[0] + v[1].abs() * half[1] + v[2].abs() * half[2];
            vertices.push(matterweave_core::Vertex {
                position: [
                    face_center[0] + u[0] * su * half_u + v[0] * sv * half_v,
                    face_center[1] + u[1] * su * half_u + v[1] * sv * half_v,
                    face_center[2] + u[2] * su * half_u + v[2] * sv * half_v,
                ],
                normal,
                color,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::ROSTER;
    use std::collections::HashSet;

    #[test]
    fn every_species_has_a_spec_and_a_mesh() {
        assert_eq!(ROWS.len(), ROSTER.len(), "one row per species");
        for entry in ROSTER {
            let spec = spec_for(entry.id);
            assert!(spec.scale > 0.0);
            let mesh = mesh_for(entry.id, 0.05);
            assert!(!mesh.vertices.is_empty(), "{} has no geometry", entry.name);
            assert_eq!(mesh.indices.len() % 6, 0);
            assert!(mesh
                .indices
                .iter()
                .all(|i| (*i as usize) < mesh.vertices.len()));
            assert!(mesh
                .vertices
                .iter()
                .all(|v| v.position.iter().all(|c| c.is_finite())));
            let normal_len = mesh.vertices[0].normal.iter().map(|c| c * c).sum::<f32>();
            assert!((normal_len - 1.0).abs() < 1e-5, "normals are unit length");
        }
    }

    #[test]
    fn evolved_forms_are_larger_or_more_detailed() {
        use crate::roster::species;
        for entry in ROSTER {
            let Some((to, _)) = entry.evolves_into else {
                continue;
            };
            let from = spec_for(entry.id);
            let into = spec_for(to);
            assert!(
                into.scale > from.scale,
                "{} -> {}: evolution must add mass",
                species(entry.id).unwrap().name,
                species(to).unwrap().name
            );
            // Occupied volume (box volume x cube of scale) must grow: a
            // silhouette change may trade legs for body, not lose mass.
            let volume = |spec: &ModelSpec| {
                model_boxes(spec)
                    .iter()
                    .map(|b| b.size.iter().map(|s| *s as f32).product::<f32>())
                    .sum::<f32>()
                    * spec.scale.powi(3)
            };
            assert!(
                volume(&into) > volume(&from),
                "{} -> {}: evolved form must occupy more space",
                species(entry.id).unwrap().name,
                species(to).unwrap().name
            );
        }
    }

    #[test]
    fn family_members_keep_distinct_accents() {
        // Within one family, at least one of scale or accent differs at every
        // step; the base hue is shared by affinity by design.
        for entry in ROSTER {
            let Some((to, _)) = entry.evolves_into else {
                continue;
            };
            let from = spec_for(entry.id);
            let into = spec_for(to);
            assert_ne!(from.palette[2], into.palette[2], "accent must develop");
        }
    }

    #[test]
    fn plans_cover_the_roster_variety() {
        let plans: HashSet<BodyPlan> = ROSTER.iter().map(|s| spec_for(s.id).plan).collect();
        assert!(
            plans.len() >= 4,
            "the roster reads as more than one silhouette"
        );
    }
}
