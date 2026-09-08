use matterweave_detail::*;
use std::collections::BTreeSet;
#[test]
fn bracket_has_attached_shelves_growth_bands_and_real_pores() {
    let v = bracket_fungus("bracket").unwrap();
    assert!(v.occupied_cells() < 33288);
    assert_eq!(v.scale().metres(), 0.0625);
    let mut remaining: BTreeSet<_> = v.iter_cells().map(|(c, _)| c).collect();
    let mut stack = vec![[0, 0, 0]];
    remaining.remove(&[0, 0, 0]);
    while let Some(c) = stack.pop() {
        for a in 0..3 {
            for sign in [-1, 1] {
                let mut n = c;
                n[a] += sign;
                if remaining.remove(&n) {
                    stack.push(n);
                }
            }
        }
    }
    assert!(remaining.is_empty(), "shelves must attach to stump");
    for level in [8, 18, 27] {
        assert_ne!(
            v.get([5, level + 1, 1]),
            material::AIR,
            "continuous flesh over pore"
        );
        assert_eq!(
            v.get([5, level, 1 + if level == 18 { 1 } else { 0 }]),
            material::AIR,
            "recessed pore"
        );
        assert!(
            v.iter_cells()
                .filter(|(c, m)| c[1] == level && *m == material::MUSHROOM_GILL)
                .count()
                > 25
        );
    }
    assert!(v
        .iter_cells()
        .all(|(_, m)| material_policy(m) == MaterialPolicy::Collision));
    let before = v.snapshot();
    for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
        v.coarsen(lod).unwrap().mesh_local().unwrap();
    }
    assert_eq!(v.snapshot(), before);
    assert_eq!(
        DetailVolume::from_snapshot(&before)
            .unwrap()
            .occupied_cells(),
        v.occupied_cells()
    );
}
