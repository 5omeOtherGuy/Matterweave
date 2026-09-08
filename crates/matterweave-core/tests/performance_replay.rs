//! P01 first deterministic authoritative-world replay slice.
//!
//! Versioned JSON fixture/input tape plus integration tests exercising the
//! real public `World` generation/streaming/edit/save APIs. Test-only host
//! regression proof; NOT phone replay support. 64-body/physics, app input
//! replay integration and camera/input/hardware captures are explicitly left
//! to later slices and are not claimed here.

use matterweave_core::{World, GENERATOR_VERSION};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const FIXTURE: &str = include_str!("fixtures/performance_replay_v1.json");
const FIXTURE_VERSION: u32 = 1;
const MAX_TAPE_STEPS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    fixture_version: u32,
    generator_version: u32,
    seed: u64,
    scenario: String,
    #[serde(default)]
    description: String,
    baseline_probes: Vec<ProbeCell>,
    baseline_stats: BaselineStats,
    steps: Vec<Step>,
    expected: Expected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeCell {
    cell: [i32; 3],
    material: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselineStats {
    chunks: usize,
    solid_voxels: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: String,
    #[serde(default)]
    position: Option<[f32; 3]>,
    #[serde(default)]
    cell: Option<[i32; 3]>,
    #[serde(default)]
    material: Option<u8>,
    #[serde(default)]
    expect_applied: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    cells: Vec<CellExpect>,
    stats: StatsExpect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellExpect {
    cell: [i32; 3],
    material: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatsExpect {
    chunks: usize,
    solid_voxels: usize,
    stored_overrides: usize,
}

/// Strict fixture validation: unsupported versions, unknown ops, malformed or
/// unbounded tapes and missing expected state fail instead of skipping.
fn parse_fixture_strict(text: &str) -> Result<Fixture, String> {
    let fixture: Fixture = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if fixture.fixture_version != FIXTURE_VERSION {
        return Err(format!(
            "unsupported fixture_version {}",
            fixture.fixture_version
        ));
    }
    if fixture.generator_version != GENERATOR_VERSION {
        return Err(format!(
            "unsupported generator_version {}",
            fixture.generator_version
        ));
    }
    if fixture.scenario.is_empty() {
        return Err("scenario must not be empty".to_owned());
    }
    if fixture.steps.is_empty() || fixture.steps.len() > MAX_TAPE_STEPS {
        return Err("tape must hold 1..=128 steps".to_owned());
    }
    for step in &fixture.steps {
        match step.op.as_str() {
            "stream" => {
                let position = step.position.ok_or("stream step needs position")?;
                if !position.iter().all(|v| v.is_finite()) {
                    return Err("stream position must be finite".to_owned());
                }
            }
            "set" => {
                let cell = step.cell.ok_or("set step needs cell")?;
                step.material.ok_or("set step needs material")?;
                step.expect_applied.ok_or("set step needs expect_applied")?;
                if !World::contains_stream_cell(cell) {
                    return Err(format!("set cell {cell:?} outside editable domain"));
                }
            }
            other => return Err(format!("unsupported op {other}")),
        }
    }
    if fixture.expected.cells.is_empty() {
        return Err("expected state must list cells".to_owned());
    }
    Ok(fixture)
}

fn load_pinned_fixture() -> Fixture {
    parse_fixture_strict(FIXTURE).expect("pinned fixture must validate")
}

/// Replays the input tape from fresh world state using real public APIs.
fn replay(fixture: &Fixture) -> Result<World, String> {
    let mut world = World::generate(fixture.seed);
    world.enable_streaming();
    for step in &fixture.steps {
        match step.op.as_str() {
            "stream" => {
                let position = step.position.ok_or("stream step needs position")?;
                if !world.stream_around(position) {
                    return Err(format!("stream_around failed for {position:?}"));
                }
            }
            "set" => {
                let cell = step.cell.ok_or("set step needs cell")?;
                let material = step.material.ok_or("set step needs material")?;
                let expect = step.expect_applied.ok_or("set step needs expect_applied")?;
                if world.set(cell, material) != expect {
                    return Err(format!("set {cell:?} applied != {expect}"));
                }
            }
            other => return Err(format!("unsupported op {other}")),
        }
    }
    Ok(world)
}

fn assert_expected_cells(world: &World, fixture: &Fixture) {
    for expect in &fixture.expected.cells {
        assert_eq!(
            world.get(expect.cell),
            expect.material,
            "cell {:?}",
            expect.cell
        );
    }
}

fn assert_expected_stats(world: &World, fixture: &Fixture) {
    let stats = world.stats();
    assert_eq!(stats.chunks, fixture.expected.stats.chunks, "chunks");
    assert_eq!(
        stats.solid_voxels, fixture.expected.stats.solid_voxels,
        "solid_voxels"
    );
    assert_eq!(
        stats.stored_overrides, fixture.expected.stats.stored_overrides,
        "stored_overrides"
    );
}

static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);
struct SaveDir(PathBuf);
impl SaveDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-replay-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for SaveDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn broken_variant(mutate: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    mutate(&mut value);
    serde_json::to_string(&value).unwrap()
}

#[test]
fn baseline_generation_is_deterministic_and_matches_pinned_probes() {
    let fixture = load_pinned_fixture();
    let first = World::generate(fixture.seed);
    let second = World::generate(fixture.seed);
    assert_eq!(first.seed(), fixture.seed);
    assert_eq!(first.stats(), second.stats());
    assert_eq!(first.stats().chunks, fixture.baseline_stats.chunks);
    assert_eq!(
        first.stats().solid_voxels,
        fixture.baseline_stats.solid_voxels
    );
    assert_eq!(first.stats().stored_overrides, 0);
    assert_eq!(
        first.stats().allocated_bytes,
        first.stats().chunks * matterweave_core::CHUNK_VOLUME
    );
    for probe in &fixture.baseline_probes {
        assert_eq!(
            first.get(probe.cell),
            probe.material,
            "cell {:?}",
            probe.cell
        );
        assert_eq!(
            second.get(probe.cell),
            probe.material,
            "cell {:?}",
            probe.cell
        );
    }
    // Same seed reproduces exactly; a different seed changes terrain somewhere
    // in the small baseline region instead of matching by coincidence.
    let other = World::generate(fixture.seed + 1);
    let mut changed = other.stats() != first.stats();
    for x in -32..32 {
        for z in -32..32 {
            for y in -8..20 {
                changed |= other.get([x, y, z]) != first.get([x, y, z]);
                assert_eq!(second.get([x, y, z]), first.get([x, y, z]));
            }
        }
    }
    assert!(changed, "seed+1 must change baseline terrain");
}

#[test]
fn unsupported_fixture_version_is_rejected() {
    let text = broken_variant(|value| value["fixture_version"] = serde_json::json!(999));
    assert!(parse_fixture_strict(&text).is_err());
}

#[test]
fn unsupported_generator_version_is_rejected() {
    let text = broken_variant(|value| value["generator_version"] = serde_json::json!(999));
    assert!(parse_fixture_strict(&text).is_err());
}

#[test]
fn unknown_op_is_rejected() {
    let text = broken_variant(|value| value["steps"][0]["op"] = serde_json::json!("teleport"));
    assert!(parse_fixture_strict(&text).is_err());
}

#[test]
fn missing_expected_state_is_rejected() {
    let text = broken_variant(|value| {
        value.as_object_mut().unwrap().remove("expected");
    });
    assert!(parse_fixture_strict(&text).is_err());
    let text = broken_variant(|value| value["expected"]["cells"] = serde_json::json!([]));
    assert!(parse_fixture_strict(&text).is_err());
}

#[test]
fn malformed_tape_is_rejected() {
    assert!(parse_fixture_strict("{not json").is_err());
    assert!(parse_fixture_strict("").is_err());
    let text = broken_variant(|value| value["steps"] = serde_json::json!([]));
    assert!(parse_fixture_strict(&text).is_err());
    let text =
        broken_variant(|value| value["steps"][0]["position"] = serde_json::json!([null, 0, 0]));
    assert!(parse_fixture_strict(&text).is_err());
}

#[test]
fn replay_twice_yields_identical_save_bytes_and_pinned_state() {
    let fixture = load_pinned_fixture();
    let first = replay(&fixture).expect("tape must replay");
    let second = replay(&fixture).expect("tape must replay twice");
    assert_eq!(first.seed(), fixture.seed);
    assert_eq!(first.revision(), second.revision());
    assert_expected_cells(&first, &fixture);
    assert_expected_cells(&second, &fixture);
    assert_expected_stats(&first, &fixture);
    assert_expected_stats(&second, &fixture);
    let dir = SaveDir::new();
    first.save(dir.path("first.json")).unwrap();
    second.save(dir.path("second.json")).unwrap();
    assert_eq!(
        fs::read(dir.path("first.json")).unwrap(),
        fs::read(dir.path("second.json")).unwrap(),
        "exact final save bytes must match across replays"
    );
}

#[test]
fn save_reload_preserves_exact_authoritative_state() {
    let fixture = load_pinned_fixture();
    let world = replay(&fixture).expect("tape must replay");
    let dir = SaveDir::new();
    world.save(dir.path("world.json")).unwrap();
    let before = fs::read(dir.path("world.json")).unwrap();
    let loaded = World::load(dir.path("world.json")).unwrap();
    assert_eq!(loaded.seed(), world.seed());
    assert_eq!(loaded.revision(), world.revision());
    assert_eq!(loaded.stats(), world.stats());
    assert_expected_cells(&loaded, &fixture);
    assert_expected_stats(&loaded, &fixture);
    loaded.save(dir.path("resaved.json")).unwrap();
    assert_eq!(
        fs::read(dir.path("resaved.json")).unwrap(),
        before,
        "resaved bytes must equal the original save"
    );
}

#[test]
fn negative_boundary_eviction_and_reversal_guards() {
    let fixture = load_pinned_fixture();
    // Euclidean chunk math is independently authoritative: -17 shares chunk
    // [-2,0,0] while -16 belongs to [-1,0,0], matching positive edge 15/16.
    let mut raw = World::new(0);
    assert!(raw.set([-17, 2, 0], 3));
    assert!(raw.set([-16, 2, 0], 4));
    assert!(raw.set([15, 2, 0], 5));
    assert!(raw.set([16, 2, 0], 6));
    assert!(raw.chunk_keys().contains(&[-2, 0, 0]));
    assert!(raw.chunk_keys().contains(&[-1, 0, 0]));
    assert!(raw.chunk_keys().contains(&[0, 0, 0]));
    assert!(raw.chunk_keys().contains(&[1, 0, 0]));

    let mut world = replay(&fixture).expect("tape must replay");
    // Negative-boundary edit from the tape is resident at center [0,0].
    assert_eq!(world.get([-17, 2, -17]), 5);
    assert_eq!(world.get([-1, 2, 0]), 7);
    // Travel west evicts the eastern override: evicted chunks read as air and
    // report no revision, while the edit persists as a stored override.
    assert!(world.stream_around([-48.0, 0.0, 0.0]));
    assert_eq!(world.get([48, 2, 0]), 0);
    assert_eq!(world.chunk_revision([3, 0, 0]), None);
    assert_eq!(world.get([-17, 2, -17]), 5);
    // Fast reversal restores the exact edited material, not regenerated terrain.
    assert!(world.stream_around([0.0, 0.0, 0.0]));
    assert_eq!(world.get([48, 2, 0]), 9);
    assert_eq!(world.get([-1, 2, 0]), 7);
    assert_expected_cells(&world, &fixture);
}

#[test]
fn known_noop_edits_change_nothing() {
    let fixture = load_pinned_fixture();
    let mut world = replay(&fixture).expect("tape must replay");
    let revision = world.revision();
    // Same material is a no-op.
    assert!(!world.set([0, 2, 0], 7));
    // Resident chunk key outside the published window is rejected.
    assert!(!world.set([200, 2, 0], 1));
    // Outside the editable simulation domain is rejected.
    assert!(!world.set([256, 0, 0], 1));
    assert_eq!(world.revision(), revision);
    assert_expected_cells(&world, &fixture);
}
