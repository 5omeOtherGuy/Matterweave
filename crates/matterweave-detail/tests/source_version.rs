use matterweave_detail::{DetailScene, DetailVolume, Lod, Scale, Transform};

fn scene() -> DetailScene {
    let mut scene = DetailScene::new();
    let mut volume = DetailVolume::new("solid", Scale::new(0.25).unwrap());
    volume.set([0, 0, 0], 1).unwrap();
    scene.add_prototype(volume).unwrap();
    scene
        .place("first", "solid", Transform::identity())
        .unwrap();
    scene
}

#[test]
fn source_versions_distinguish_world_replacement_and_track_snapshots() {
    let mut live = scene();
    let snapshot = live.fork_source();
    assert_eq!(live.source_version(), snapshot.source_version());
    // Independently reconstructed scenes must not accidentally validate old jobs,
    // even with identical local prototype revision counters and content.
    assert_ne!(live.source_version(), scene().source_version());
    let before = live.source_version();
    live.edit_prototype("solid", [0, 0, 0], 0).unwrap();
    assert_ne!(live.source_version(), before);
    assert_eq!(snapshot.source_version(), before);
    // Undoing content does not make the earlier asynchronous job current again.
    live.edit_prototype("solid", [0, 0, 0], 1).unwrap();
    assert_ne!(live.source_version(), before);
}

#[test]
fn derived_work_noops_and_rejected_mutations_keep_the_source_version() {
    let mut live = scene();
    let before = live.source_version();
    live.prototype_mesh("solid", Lod::Half).unwrap();
    live.invalidate("solid");
    assert!(!live.edit_instance("first", [0, 0, 0], 1).unwrap());
    assert!(live.edit_instance("missing", [0, 0, 0], 0).is_err());
    assert!(live.place("first", "solid", Transform::identity()).is_err());
    assert_eq!(live.source_version(), before);
    live.place("second", "solid", Transform::identity())
        .unwrap();
    assert_ne!(live.source_version(), before);
    let before_edit = live.source_version();
    live.edit_instance("second", [0, 0, 0], 0).unwrap();
    assert_ne!(live.source_version(), before_edit);
}
