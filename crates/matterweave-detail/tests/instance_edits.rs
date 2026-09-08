use matterweave_detail::*;

#[test]
fn shared_source_edits_are_local_and_replayable() {
    fn fixture() -> DetailScene {
        let mut s = DetailScene::new();
        s.add_prototype(parasol_mushroom("cap").unwrap()).unwrap();
        s.place("first", "cap", Transform::identity()).unwrap();
        s.place(
            "second",
            "cap",
            Transform::new([3., 0., 0.], Yaw::Deg90).unwrap(),
        )
        .unwrap();
        s
    }
    let mut a = fixture();
    let before = a.counts();
    let cell = [0, 4, 0];
    assert_ne!(a.prototype("cap").unwrap().get(cell), material::AIR);
    assert!(a.edit_instance("first", cell, material::AIR).unwrap());
    assert_ne!(a.prototype("cap").unwrap().get(cell), material::AIR);
    assert_eq!(a.counts().prototypes, before.prototypes + 1);
    let private = a
        .draws()
        .into_iter()
        .find(|i| i.instance == "first")
        .unwrap()
        .prototype;
    assert_eq!(a.prototype(&private).unwrap().get(cell), material::AIR);
    assert!(!a.edit_instance("first", cell, material::AIR).unwrap());
    assert_eq!(a.counts().prototypes, before.prototypes + 1);
    let mut replay = fixture();
    replay.edit_instance("first", cell, material::AIR).unwrap();
    assert_eq!(
        a.prototype(&private).unwrap().snapshot(),
        replay.prototype(&private).unwrap().snapshot()
    );
    assert!(a.edit_instance("second", [i32::MAX, 0, 0], 0).is_err());
    assert_eq!(a.counts().prototypes, before.prototypes + 1);
}
