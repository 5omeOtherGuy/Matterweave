use crate::indirect::{IndirectVolume, UpdateBudget, MAX_FACE_SLOTS};
use crate::Sun;
use matterweave_core::World;

fn sun() -> Sun {
    Sun {
        direction_to_sun: [0., 1., 0.],
        intensity: 1.,
    }
}
fn volume() -> IndirectVolume {
    let mut palette = [[0.5; 3]; 256];
    palette[1] = [0.9, 0.05, 0.02];
    IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 16., palette).unwrap()
}
fn room(closed: bool) -> World {
    let mut w = World::new(7);
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, -1, z], 1);
            if closed {
                w.set([x, 3, z], 2);
            }
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    w.set([x, y, z], 2);
                }
            }
        }
    }
    w
}
fn finish(v: &mut IndirectVolume, w: &World, epoch: u64, light: Sun) {
    for _ in 0..1000 {
        let s = v
            .update(
                w,
                epoch,
                light,
                UpdateBudget {
                    rays: 4096,
                    work: 4096,
                },
            )
            .unwrap();
        if s.complete {
            return;
        }
    }
    panic!("bounded fixture never completed");
}
#[test]
fn enclosure_opening_and_closing_controls_colored_bounce() {
    let mut w = room(true);
    let mut v = volume();
    finish(&mut v, &w, 0, sun());
    assert_eq!(
        v.sample([-3, 1, 0], 0),
        [0.; 3],
        "sealed room has no illuminated bounce source"
    );
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, 3, z], 0);
        }
    }
    finish(&mut v, &w, 0, sun());
    let rgb = v.sample([-3, 1, 0], 0);
    assert!(
        rgb[0] > 0.03,
        "sunlit red floor must illuminate inward wall: {rgb:?}"
    );
    assert!(
        rgb[0] > rgb[1] * 2.,
        "colored energy must transfer: {rgb:?}"
    );
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, 3, z], 2);
        }
    }
    v.update(&w, 0, sun(), UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert_eq!(
        v.sample([-3, 1, 0], 0),
        [0.; 3],
        "old open-room cache must disappear immediately"
    );
    finish(&mut v, &w, 0, sun());
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
}
#[test]
fn budget_and_light_or_source_invalidation_are_explicit() {
    let w = room(false);
    let mut v = volume();
    let s = v
        .update(&w, 0, sun(), UpdateBudget { rays: 3, work: 2 })
        .unwrap();
    assert!(s.rays <= 3 && s.work <= 2 && !s.complete);
    finish(&mut v, &w, 0, sun());
    assert!(v.sample([-3, 1, 0], 0)[0] > 0.);
    let dark = Sun {
        intensity: 0.,
        ..sun()
    };
    v.update(&w, 0, dark, UpdateBudget { rays: 0, work: 0 })
        .unwrap();
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
    assert!(!v.valid_for(&w, 0, sun()));
    finish(&mut v, &w, 0, dark);
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
    finish(&mut v, &w, 0, sun());
    assert!(
        !v.valid_for(&w, 1, sun()),
        "replacement source epoch rejects same revision"
    );
    let down = Sun {
        direction_to_sun: [0., -1., 0.],
        ..sun()
    };
    finish(&mut v, &w, 1, down);
    assert_eq!(v.sample([-3, 1, 0], 0), [0.; 3]);
}
#[test]
fn thin_wall_separates_lit_and_sealed_spaces() {
    let mut w = room(false);
    // One-voxel divider, roof only on the right: no interpolation across it.
    for z in -3..=3 {
        for y in 0..=3 {
            w.set([0, y, z], 2);
        }
        for x in 1..=3 {
            w.set([x, 3, z], 2);
        }
    }
    let mut v = volume();
    finish(&mut v, &w, 0, sun());
    assert!(v.sample([0, 1, 0], 1)[0] > 0.02);
    assert_eq!(v.sample([0, 1, 0], 0), [0.; 3]);
}
#[test]
fn caps_and_invalid_inputs_fail_before_allocation() {
    let p = [[0.5; 3]; 256];
    for dims in [[u32::MAX; 3], [0, 1, 1], [MAX_FACE_SLOTS as u32, 1, 1]] {
        assert!(IndirectVolume::new([0; 3], dims, 32, 16., p).is_err());
    }
    assert!(IndirectVolume::new([i32::MAX; 3], [1; 3], 32, 16., p).is_err());
    for range in [0., f32::NAN, f32::INFINITY, 4097.] {
        assert!(IndirectVolume::new([0; 3], [1; 3], 32, range, p).is_err());
    }
    let mut v = volume();
    let w = room(false);
    assert!(v
        .update(
            &w,
            0,
            Sun {
                direction_to_sun: [0.; 3],
                intensity: 1.
            },
            UpdateBudget { rays: 1, work: 1 }
        )
        .is_err());
}
#[test]
fn scheduling_is_deterministic_and_does_not_edit_authority() {
    let w = room(false);
    let revision = w.revision();
    let stats = w.stats();
    let mut a = volume();
    let mut b = volume();
    finish(&mut a, &w, 0, sun());
    for _ in 0..100000 {
        if b.update(&w, 0, sun(), UpdateBudget { rays: 18, work: 11 })
            .unwrap()
            .complete
        {
            break;
        }
    }
    assert_eq!(a.sample([-3, 1, 0], 0), b.sample([-3, 1, 0], 0));
    assert_eq!(w.revision(), revision);
    assert_eq!(w.stats(), stats);
}
