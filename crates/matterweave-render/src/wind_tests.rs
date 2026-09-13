//! Host tests for the wind-capable flora path.
//!
//! These cover exactly the parts that can be checked without a device: how a
//! flora placement is packed into the second per-instance attribute, that a
//! non-flora scene packs the zero record the shader early-outs on, that the
//! budgets reject rather than truncate, and that the shader and the Rust
//! uniform agree about what location 4 and the two new uniform fields are.

use crate::lighting::{PlayerPush, Wind};
use crate::static_scene::{
    plan_flora_scene, plan_static_scene, FloraInstance, StaticInstance, MAX_FLORA_BYTES,
    MAX_FLORA_HEIGHT_M, MAX_FLORA_INSTANCES, MAX_WIND_DISPLACEMENT_M, WIND_RECORD_SIZE, ZERO_WIND,
};
use crate::LightingSettings;
use matterweave_core::{Mesh, Vertex};

fn triangle() -> Mesh {
    Mesh {
        vertices: [[0., 0., 0.], [1., 0., 0.], [0., 2., 0.]]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0., 1., 0.],
                color: [0.5; 3],
            })
            .collect(),
        indices: vec![0, 1, 2],
        revision: 1,
    }
}

fn flora(prototype: usize, translation: [f32; 3], phase: f32, bend: f32) -> FloraInstance {
    FloraInstance {
        prototype,
        translation,
        yaw_quarters: 0,
        phase,
        bend,
        height_m: 0.5,
    }
}

fn wind_records(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(WIND_RECORD_SIZE)
        .map(|chunk| *bytemuck::from_bytes::<[f32; 4]>(chunk))
        .collect()
}

#[test]
fn the_wind_record_is_one_vec4_of_phase_bend_height_and_enable() {
    assert_eq!(WIND_RECORD_SIZE, 16, "location 4 is one vec4<f32>");
    assert_eq!(ZERO_WIND.data, [0.0; 4]);
    let plan = plan_flora_scene(
        &[triangle()],
        &[
            FloraInstance {
                height_m: 1.25,
                ..flora(0, [3., 0., -4.], 0.25, 0.75)
            },
            flora(0, [0., 0., 0.], 1.0, 0.0),
        ],
    )
    .unwrap();
    // Two per-instance buffers of the same record count: a batch's
    // `firstInstance` selects the same slice of both.
    assert_eq!(plan.instance_bytes.len(), plan.wind_bytes.len());
    assert_eq!(plan.instance_count, 2);
    assert!(plan.wind_capable);
    assert_eq!(
        wind_records(&plan.wind_bytes),
        vec![[0.25, 0.75, 1.25, 1.0], [1.0, 0.0, 0.5, 1.0]],
        "declared order is (phase, bend, height_m, enabled)"
    );
}

#[test]
fn non_flora_uploads_leave_the_wind_attribute_zero() {
    let plan = plan_static_scene(
        &[triangle()],
        &[
            StaticInstance {
                prototype: 0,
                translation: [1., 2., 3.],
                yaw_quarters: 1,
            },
            StaticInstance {
                prototype: 0,
                translation: [-5., 0., 0.],
                yaw_quarters: 3,
            },
        ],
    )
    .unwrap();
    assert!(!plan.wind_capable);
    assert_eq!(plan.instance_bytes.len(), plan.wind_bytes.len());
    for record in wind_records(&plan.wind_bytes) {
        assert_eq!(
            record, [0.0; 4],
            "a static instance must pack the disabled wind record"
        );
    }
    // `wind.w = 0` is what the shader early-outs on, so this is the guarantee
    // that existing geometry is untouched.
    assert!(wind_records(&plan.wind_bytes)
        .iter()
        .all(|record| record[3] < 0.5));
}

#[test]
fn flora_bounds_grow_by_the_wind_margin_and_static_bounds_do_not() {
    let flora_plan = plan_flora_scene(&[triangle()], &[flora(0, [0., 0., 0.], 0., 1.)]).unwrap();
    let static_plan = plan_static_scene(
        &[triangle()],
        &[StaticInstance {
            prototype: 0,
            translation: [0., 0., 0.],
            yaw_quarters: 0,
        }],
    )
    .unwrap();
    // The prototype spans x 0..1, y 0..2, z 0..0.
    assert_eq!(static_plan.bounds, [[0., 0., 0.], [1., 2., 0.]]);
    let m = MAX_WIND_DISPLACEMENT_M;
    assert_eq!(flora_plan.bounds, [[-m, 0., -m], [1. + m, 2., m]]);
    // Height is never inflated: displacement is horizontal.
    assert_eq!(flora_plan.bounds[0][1], static_plan.bounds[0][1]);
    assert_eq!(flora_plan.bounds[1][1], static_plan.bounds[1][1]);
}

#[test]
fn the_flora_budget_rejects_whole_updates_instead_of_truncating() {
    let meshes = [triangle()];
    let over: Vec<FloraInstance> = (0..(MAX_FLORA_BYTES / 16) + 1)
        .map(|i| flora(0, [i as f32, 0., 0.], 0.5, 0.5))
        .collect();
    assert_eq!(over.len(), MAX_FLORA_INSTANCES + 1);
    let error = plan_flora_scene(&meshes, &over).expect_err("over budget must be rejected");
    assert!(error.contains("budget"), "{error}");
    // Exactly at the budget is accepted, and nothing is dropped.
    let at: Vec<FloraInstance> = over[..MAX_FLORA_INSTANCES].to_vec();
    let plan = plan_flora_scene(&meshes, &at).unwrap();
    assert_eq!(plan.instance_count as usize, MAX_FLORA_INSTANCES);
    assert_eq!(plan.wind_bytes.len(), MAX_FLORA_BYTES);
}

#[test]
fn invalid_wind_fields_are_rejected_before_anything_is_packed() {
    let meshes = [triangle()];
    for (name, bad) in [
        ("phase below range", flora(0, [0.; 3], -0.01, 0.5)),
        ("phase above range", flora(0, [0.; 3], 1.01, 0.5)),
        ("phase not finite", flora(0, [0.; 3], f32::NAN, 0.5)),
        ("bend above range", flora(0, [0.; 3], 0.5, 1.5)),
        ("bend not finite", flora(0, [0.; 3], 0.5, f32::INFINITY)),
        (
            "zero height",
            FloraInstance {
                height_m: 0.0,
                ..flora(0, [0.; 3], 0.5, 0.5)
            },
        ),
        (
            "height past the cap",
            FloraInstance {
                height_m: MAX_FLORA_HEIGHT_M + 0.1,
                ..flora(0, [0.; 3], 0.5, 0.5)
            },
        ),
        (
            "non-finite translation",
            flora(0, [f32::NAN, 0., 0.], 0.5, 0.5),
        ),
        ("unknown prototype", flora(9, [0., 0., 0.], 0.5, 0.5)),
    ] {
        assert!(plan_flora_scene(&meshes, &[bad]).is_err(), "{name}");
    }
    // The boundary values themselves are valid.
    for good in [
        flora(0, [0.; 3], 0.0, 0.0),
        flora(0, [0.; 3], 1.0, 1.0),
        FloraInstance {
            height_m: MAX_FLORA_HEIGHT_M,
            ..flora(0, [0.; 3], 0.5, 0.5)
        },
    ] {
        plan_flora_scene(&meshes, &[good]).unwrap();
    }
}

#[test]
fn flora_reuses_prototype_pooling_and_per_prototype_batching() {
    let plan = plan_flora_scene(
        &[triangle(), triangle()],
        &[
            flora(0, [0., 0., 0.], 0.1, 0.9),
            flora(1, [5., 0., 0.], 0.2, 0.8),
            flora(0, [9., 0., 0.], 0.3, 0.7),
        ],
    )
    .unwrap();
    assert_eq!(plan.prototypes.len(), 2, "one batch per prototype");
    assert_eq!(plan.prototypes[0].instance_count, 2);
    assert_eq!(plan.prototypes[1].instance_count, 1);
    // Grouping reorders instances; the wind records follow the same grouping,
    // which is the invariant `firstInstance` relies on.
    assert_eq!(
        wind_records(&plan.wind_bytes)
            .iter()
            .map(|r| r[0])
            .collect::<Vec<_>>(),
        vec![0.1, 0.3, 0.2]
    );
    assert_eq!(plan.prototypes[1].instance_offset, 2);
}

#[test]
fn the_shaders_and_the_pipeline_agree_about_the_wind_attribute() {
    let world = include_str!("world.wgsl");
    assert!(
        world.contains("@location(4) wind: vec4<f32>"),
        "world.wgsl must declare the wind attribute at location 4"
    );
    assert!(
        world.contains("if wind.w < 0.5 { return world_position; }"),
        "world.wgsl must early-out on a disabled wind record"
    );
    assert!(
        world.contains("wind: vec4<f32>") && world.contains("player: vec4<f32>"),
        "world.wgsl must read the wind and player uniform fields"
    );
    // The shadow pass deliberately does not consume the attribute; casters use
    // their rest pose. See `Shadow::record`.
    assert!(
        !include_str!("shadow.wgsl").contains("@location(4)"),
        "shadow.wgsl must not read the wind attribute"
    );
    assert!(
        !include_str!("hud.wgsl").contains("@location(4)"),
        "the HUD pipeline has no instance bindings at all"
    );
}

#[test]
fn still_air_and_no_push_are_the_defaults_every_existing_sample_gets() {
    let settings = LightingSettings::default();
    settings.wind.validate().unwrap();
    settings.player.validate().unwrap();
    assert_eq!(settings.wind.strength_m, 0.0);
    assert_eq!(settings.player.radius_m, 0.0);
    // Zero strength and zero radius: `displace` adds nothing even to a flora
    // vertex whose own record is enabled.
    assert_eq!(settings.wind.packed()[2], 0.0);
    assert_eq!(settings.player.packed()[3], 0.0);
}

#[test]
fn wind_is_normalized_folded_and_bounded() {
    let wind = Wind {
        direction_xz: [0.0, 4.0],
        strength_m: 0.6,
        time_s: Wind::TIME_PERIOD_S + 12.5,
    };
    wind.validate().unwrap();
    let packed = wind.packed();
    assert!((packed[0] - 0.0).abs() < 1.0e-6 && (packed[1] - 1.0).abs() < 1.0e-6);
    assert_eq!(packed[2], 0.6);
    assert!(
        (packed[3] - 12.5).abs() < 1.0e-3,
        "time must fold into its period: {}",
        packed[3]
    );
    // A degenerate direction is still air, not a NaN direction.
    let degenerate = Wind {
        direction_xz: [0.0, 0.0],
        strength_m: 1.0,
        time_s: 0.0,
    };
    assert_eq!(degenerate.packed(), [1.0, 0.0, 0.0, 0.0]);

    for bad in [
        Wind {
            strength_m: -0.1,
            ..Wind::default()
        },
        Wind {
            strength_m: Wind::MAX_STRENGTH_M + 0.1,
            ..Wind::default()
        },
        Wind {
            strength_m: f32::NAN,
            ..Wind::default()
        },
        Wind {
            direction_xz: [f32::INFINITY, 0.0],
            ..Wind::default()
        },
        Wind {
            time_s: f32::NAN,
            ..Wind::default()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn the_player_push_is_finite_and_bounded() {
    PlayerPush {
        position: [10., 2., -3.],
        radius_m: PlayerPush::MAX_RADIUS_M,
    }
    .validate()
    .unwrap();
    for bad in [
        PlayerPush {
            position: [f32::NAN, 0., 0.],
            radius_m: 1.0,
        },
        PlayerPush {
            position: [0.; 3],
            radius_m: -0.1,
        },
        PlayerPush {
            position: [0.; 3],
            radius_m: PlayerPush::MAX_RADIUS_M + 0.1,
        },
        PlayerPush {
            position: [0.; 3],
            radius_m: f32::INFINITY,
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}
