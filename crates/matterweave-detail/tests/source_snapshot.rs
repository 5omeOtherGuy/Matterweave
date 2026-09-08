use matterweave_detail::{material, DetailScene, DetailVolume, Lod, Scale, Transform};

#[test]
fn forked_instance_and_prototype_edits_leave_older_sources_and_meshes_unchanged() {
    let mut volume = DetailVolume::new("solid", Scale::new(0.25).unwrap());
    // Separate chunks, including a negative coordinate; edits remove whole chunks.
    for cell in [[-1, 0, 0], [16, 0, 0]] {
        volume.set(cell, material::BANK_STONE).unwrap();
    }
    let volume_clone = volume.clone();
    let original = volume.snapshot();
    let mut live = DetailScene::new();
    live.add_prototype(volume).unwrap();
    for id in ["first", "second"] {
        live.place(id, "solid", Transform::identity()).unwrap();
    }
    let lods = [Lod::Source, Lod::Half, Lod::Quarter];
    let mesh_data = |mesh: &matterweave_core::Mesh| {
        (
            bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec(),
            mesh.indices.clone(),
        )
    };
    let original_meshes: Vec<_> = lods
        .iter()
        .map(|&lod| mesh_data(live.prototype_mesh("solid", lod).unwrap()))
        .collect();
    let mut candidate = live.fork_source();
    let initial_version = live.source_version();
    let initial_draws = live.draws();
    assert_eq!(candidate.source_version(), initial_version);
    // Derived meshes are not carried into a source-only fork.
    assert_eq!(candidate.counts().cached_mesh_bytes, 0);
    for &lod in &lods {
        candidate.prototype_mesh("solid", lod).unwrap();
    }

    assert!(candidate
        .edit_instance("first", [-1, 0, 0], material::AIR)
        .unwrap());
    let private = candidate
        .draws()
        .into_iter()
        .find(|draw| draw.instance == "first")
        .unwrap()
        .prototype;
    let private_version = candidate.source_version();
    assert_ne!(private_version, initial_version);
    assert_eq!(candidate.prototype("solid").unwrap().snapshot(), original);
    let private_snapshot = candidate.prototype(&private).unwrap().snapshot();
    assert_eq!(
        candidate.prototype(&private).unwrap().get([-1, 0, 0]),
        material::AIR
    );
    let private_meshes: Vec<_> = lods
        .iter()
        .map(|&lod| mesh_data(candidate.prototype_mesh(&private, lod).unwrap()))
        .collect();
    let retained = candidate.fork_source();

    // Mutating the original prototype after the private split must invalidate
    // every original LOD, but not the private instance's derived meshes.
    assert!(candidate
        .edit_prototype("solid", [16, 0, 0], material::AIR)
        .unwrap());
    assert_ne!(candidate.source_version(), private_version);
    let builds = candidate.counts().mesh_builds;
    let revision = candidate.prototype("solid").unwrap().revision();
    for (index, &lod) in lods.iter().enumerate() {
        let mesh = candidate.prototype_mesh("solid", lod).unwrap();
        assert_eq!(mesh.revision, revision);
        assert_ne!(mesh_data(mesh), original_meshes[index]);
        assert_eq!(
            mesh_data(candidate.prototype_mesh(&private, lod).unwrap()),
            private_meshes[index]
        );
    }
    assert_eq!(candidate.counts().mesh_builds, builds + 3);
    assert_eq!(
        candidate.prototype(&private).unwrap().snapshot(),
        private_snapshot
    );
    assert_eq!(retained.source_version(), private_version);
    assert_eq!(retained.prototype("solid").unwrap().snapshot(), original);
    assert_eq!(
        retained.prototype(&private).unwrap().snapshot(),
        private_snapshot
    );

    assert_eq!(volume_clone.snapshot(), original);
    assert_eq!(live.prototype("solid").unwrap().snapshot(), original);
    assert_eq!(live.source_version(), initial_version);
    assert_eq!(live.draws(), initial_draws);
    let builds = live.counts().mesh_builds;
    for (index, &lod) in lods.iter().enumerate() {
        assert_eq!(
            mesh_data(live.prototype_mesh("solid", lod).unwrap()),
            original_meshes[index]
        );
    }
    assert_eq!(live.counts().mesh_builds, builds);
}
