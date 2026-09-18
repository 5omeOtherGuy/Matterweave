//! Mesh and scene builders for the app: the player avatar, town NPCs and the
//! battle arena. All geometry is original voxel work built through the game
//! crate's box-mesh helper and uploaded through the renderer's ordinary paths.
use matterweave_core::Mesh;
use matterweave_monsters::models::{mesh_from_boxes, VoxelBox};
use matterweave_monsters::roster::SpeciesId;
use matterweave_render::StaticInstance;

pub const PLAYER_UNIT: f32 = 0.055;

fn palette(base: [u8; 3], accent: [u8; 3], eye: [u8; 3]) -> [[u8; 3]; 4] {
    let dark = [base[0] / 2, base[1] / 2, base[2] / 2];
    [base, dark, accent, eye]
}

/// The player: a compact humanoid with a bright scarf so the avatar reads at
/// the close camera. Facing +Z.
pub fn player_mesh() -> Mesh {
    let boxes = &[
        VoxelBox {
            min: [-3, 6, -2],
            size: [6, 7, 5],
            slot: 0,
        }, // torso
        VoxelBox {
            min: [-2, 13, -2],
            size: [5, 5, 5],
            slot: 0,
        }, // head
        VoxelBox {
            min: [-3, 14, -3],
            size: [7, 2, 2],
            slot: 2,
        }, // cap brim
        VoxelBox {
            min: [-2, 11, -2],
            size: [5, 2, 5],
            slot: 2,
        }, // scarf
        VoxelBox {
            min: [-3, 0, -2],
            size: [2, 6, 3],
            slot: 1,
        }, // left leg
        VoxelBox {
            min: [1, 0, -2],
            size: [2, 6, 3],
            slot: 1,
        }, // right leg
        VoxelBox {
            min: [-4, 7, -1],
            size: [1, 5, 3],
            slot: 0,
        }, // left arm
        VoxelBox {
            min: [3, 7, -1],
            size: [1, 5, 3],
            slot: 0,
        }, // right arm
        VoxelBox {
            min: [-5, 8, -3],
            size: [2, 3, 2],
            slot: 3,
        }, // satchel
    ];
    mesh_from_boxes(
        boxes,
        &palette([86, 122, 196], [226, 92, 72], [42, 34, 30]),
        PLAYER_UNIT,
    )
}

/// Town or route NPCs. The role picks the palette so a haven keeper reads
/// differently from a trainer at a glance. Facing +Z.
pub fn npc_mesh(role: NpcRole) -> Mesh {
    let (base, accent) = match role {
        NpcRole::Healer => ([236, 238, 240], [214, 76, 96]),
        NpcRole::Merchant => ([224, 190, 120], [92, 132, 84]),
        NpcRole::Trainer => ([120, 96, 160], [240, 200, 90]),
        NpcRole::Leader => ([80, 70, 66], [210, 180, 90]),
        NpcRole::Resident => ([150, 140, 128], [110, 140, 190]),
    };
    let boxes = &[
        VoxelBox {
            min: [-3, 6, -2],
            size: [6, 7, 5],
            slot: 0,
        },
        VoxelBox {
            min: [-2, 13, -2],
            size: [5, 5, 5],
            slot: 0,
        },
        VoxelBox {
            min: [-2, 11, -2],
            size: [5, 2, 5],
            slot: 2,
        },
        VoxelBox {
            min: [-3, 0, -2],
            size: [2, 6, 3],
            slot: 1,
        },
        VoxelBox {
            min: [1, 0, -2],
            size: [2, 6, 3],
            slot: 1,
        },
        VoxelBox {
            min: [-4, 7, -1],
            size: [1, 5, 3],
            slot: 0,
        },
        VoxelBox {
            min: [3, 7, -1],
            size: [1, 5, 3],
            slot: 0,
        },
    ];
    mesh_from_boxes(boxes, &palette(base, accent, [40, 36, 32]), PLAYER_UNIT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcRole {
    Healer,
    Merchant,
    Trainer,
    Leader,
    Resident,
}

pub fn role_of(id: &str) -> NpcRole {
    match id {
        "haven" => NpcRole::Healer,
        "shop" => NpcRole::Merchant,
        "leader-marwick" => NpcRole::Leader,
        id if id.starts_with("gym-")
            || id.contains("ranger")
            || id.contains("wayfarer")
            || id.contains("herbalist")
            || id.contains("prospector")
            || id.contains("scout") =>
        {
            NpcRole::Trainer
        }
        _ => NpcRole::Resident,
    }
}

/// One static scene for the overworld: every NPC in the current place.
pub fn npc_scene(npcs: &[(NpcRole, [f32; 3], u8)]) -> (Vec<Mesh>, Vec<StaticInstance>) {
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut cache: Vec<(NpcRole, usize)> = Vec::new();
    for (role, translation, yaw_quarters) in npcs {
        let prototype = if let Some((_, index)) = cache.iter().find(|(r, _)| r == role) {
            *index
        } else {
            let index = meshes.len();
            meshes.push(npc_mesh(*role));
            cache.push((*role, index));
            index
        };
        instances.push(StaticInstance {
            prototype,
            translation: *translation,
            yaw_quarters: *yaw_quarters,
        });
    }
    (meshes, instances)
}

/// The battle arena: the player's active monster facing the opponent across a
/// small clearing, tight enough that the fixed camera frames both. `unit`
/// scales the models to arena size.
pub fn arena_scene(
    player_species: SpeciesId,
    wild_species: SpeciesId,
    center: [f32; 3],
    unit: f32,
) -> (Vec<Mesh>, Vec<StaticInstance>) {
    let meshes = vec![
        matterweave_monsters::models::mesh_for(player_species, unit),
        matterweave_monsters::models::mesh_for(wild_species, unit),
    ];
    // Both combatants sit in front of the fixed camera, which looks along +Z
    // with screen-left at +X: the opponent stands far-left, the player's
    // creature near-right, and they face each other across the X axis
    // (quarter 1 faces +X, quarter 3 faces -X).
    let instances = vec![
        StaticInstance {
            prototype: 0,
            translation: [center[0] - 1.5, center[1], center[2] + 0.8],
            yaw_quarters: 1,
        },
        StaticInstance {
            prototype: 1,
            translation: [center[0] + 1.5, center[1], center[2] + 3.0],
            yaw_quarters: 3,
        },
    ];
    (meshes, instances)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_and_npc_meshes_are_valid_geometry() {
        for mesh in [
            player_mesh(),
            npc_mesh(NpcRole::Healer),
            npc_mesh(NpcRole::Leader),
        ] {
            assert!(!mesh.vertices.is_empty());
            assert_eq!(mesh.indices.len() % 6, 0);
            assert!(mesh
                .indices
                .iter()
                .all(|i| (*i as usize) < mesh.vertices.len()));
        }
    }

    #[test]
    fn npc_scene_shares_one_prototype_per_role() {
        let (meshes, instances) = npc_scene(&[
            (NpcRole::Healer, [0.0, 2.0, 0.0], 0),
            (NpcRole::Trainer, [4.0, 2.0, 0.0], 2),
            (NpcRole::Trainer, [8.0, 2.0, 0.0], 1),
        ]);
        assert_eq!(meshes.len(), 2, "one mesh per distinct role");
        assert_eq!(instances.len(), 3);
        assert_eq!(instances[1].prototype, instances[2].prototype);
    }

    #[test]
    fn arena_places_both_sides_on_the_ground_facing_each_other() {
        let (meshes, instances) = arena_scene(
            matterweave_monsters::roster::ids::CINDERUB,
            matterweave_monsters::roster::ids::NIBBIT,
            [10.0, 2.0, 10.0],
            0.05,
        );
        assert_eq!(meshes.len(), 2);
        // Player near-right facing +X, opponent far-left facing -X.
        assert_eq!(instances[0].yaw_quarters, 1);
        assert_eq!(instances[1].yaw_quarters, 3);
        assert!(instances[0].translation[2] < instances[1].translation[2]);
        assert!(instances[0].translation[0] < instances[1].translation[0]);
        assert_eq!(instances[0].translation[1], 2.0);
    }
}
