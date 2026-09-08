use crate::{
    indirect::{IndirectVolume, UpdateBudget, MAX_UPDATE_RAYS, MAX_UPDATE_WORK},
    Sun,
};
use matterweave_core::World;

#[test]
fn partial_results_cannot_survive_an_edit_and_invalid_light_clears_output() {
    let mut world = World::new(0);
    world.set([0, 0, 0], 1);
    let mut v = IndirectVolume::new([0, 0, 0], [1; 3], 256, 16., [[0.5; 3]; 256]).unwrap();
    let small = UpdateBudget { rays: 2, work: 1 };
    assert!(!v.update(&world, 0, Sun::default(), small).unwrap().complete);
    world.set([0, 0, 0], 0);
    assert!(
        v.update(&world, 0, Sun::default(), UpdateBudget { rays: 0, work: 6 })
            .unwrap()
            .complete
    );
    for f in 0..6 {
        assert_eq!(v.sample([0, 0, 0], f), [0.; 3]);
    }
    assert!(v
        .update(
            &world,
            0,
            Sun {
                intensity: f32::NAN,
                ..Sun::default()
            },
            small
        )
        .is_err());
    assert!(!v.valid_for(&world, 0, Sun::default()));
}

#[test]
fn absolute_work_caps_and_lookup_bounds_hold() {
    let mut world = World::new(0);
    for x in 0..16 {
        for z in 0..16 {
            world.set([x, 0, z], 1);
        }
    }
    let mut v = IndirectVolume::new([0; 3], [16; 3], 256, 1., [[0.5; 3]; 256]).unwrap();
    let s = v
        .update(
            &world,
            0,
            Sun::default(),
            UpdateBudget {
                rays: usize::MAX,
                work: usize::MAX,
            },
        )
        .unwrap();
    assert!(s.rays <= MAX_UPDATE_RAYS && s.work <= MAX_UPDATE_WORK && !s.complete);
    assert!(v.resident_bytes() < 400_000);
    for cell in [[-1, 0, 0], [16, 0, 0], [i32::MIN; 3], [i32::MAX; 3]] {
        assert_eq!(v.sample(cell, 0), [0.; 3]);
    }
    assert_eq!(v.sample([0; 3], 6), [0.; 3]);
}

#[test]
fn bad_palette_and_sampling_parameters_are_rejected() {
    for samples in [0, 257, u32::MAX] {
        assert!(IndirectVolume::new([0; 3], [1; 3], samples, 16., [[0.5; 3]; 256]).is_err());
    }
    for c in [-0.1, 1.1, f32::NAN] {
        let mut p = [[0.5; 3]; 256];
        p[255][2] = c;
        assert!(IndirectVolume::new([0; 3], [1; 3], 32, 16., p).is_err());
    }
    let world = World::new(0);
    let mut v = IndirectVolume::new([-8192; 3], [1; 3], 1, 0.001, [[1.; 3]; 256]).unwrap();
    assert!(
        v.update(&world, 0, Sun::default(), UpdateBudget { rays: 0, work: 6 })
            .unwrap()
            .complete
    );
}
