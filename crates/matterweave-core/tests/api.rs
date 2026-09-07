use matterweave_core::World;

#[test]
fn empty_world_is_queryable() {
    let world = World::new(7);
    assert_eq!(world.seed(), 7);
    assert_eq!(world.get([-1, 0, 16]), 0);
    assert_eq!(world.revision(), 0);
}
