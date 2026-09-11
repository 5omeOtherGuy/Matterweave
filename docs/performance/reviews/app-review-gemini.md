# Independent Gemini app review

Frozen 3f8f01d. Candidate findings; tests not run.

### Engineering Log: Wetland Subsystem Leaf Review

**Target**: Frozen HEAD `3f8f01d`  
**Scope**: `apps/explorer/src/{wetland.rs, wetland_state.rs, wetland_metrics.rs, experience.rs}`, `Controls::start_wetland`, desktop/Android entry hooks.  
**Verification State**: Code inspection only. Tests NOT RUN (no writes, no subprocesses, no device execution).

---

#### Finding 1: Strict parent directory fsync fails valid atomic saves on unsupported platforms
- **File / Line**: `apps/explorer/src/wetland_state.rs:102`
- **Trigger**: `SavedWetland::save` executes `fs::File::open(parent)?.sync_all()?` after the temporary file has already been flushed and renamed to `wetland-session.json`.
- **Consequence**: On platforms or filesystems where opening a directory descriptor via `std::fs::File::open` is unsupported or rejected with permission errors (e.g., Windows returning `ERROR_ACCESS_DENIED`, or filesystems prohibiting directory open), `save` returns an `Err`. This marks autosaves as failed in metrics (`row.save_failures`) and displays "Save failed" on the HUD, despite the save file having been written and renamed.
- **Evidence**:
  ```rust
  fs::rename(&temporary, path)?;
  if let Some(parent) = path.parent() {
      fs::File::open(parent)?.sync_all()?;
  }
  ```
  Contrast with `crates/matterweave-core/src/persistence.rs:158`, which treats parent directory syncing as best-effort:
  ```rust
  if let Ok(directory) = File::open(parent) {
      let _ = directory.sync_all();
  }
  ```
- **Uncertainty**: On Linux and Android where `File::open` on a directory succeeds with `O_RDONLY`, the call succeeds if `parent` permissions allow reading.

---

#### Finding 2: Missing prototype in draw list aborts entire raycast query via early `?`
- **File / Line**: `apps/explorer/src/wetland_state.rs:140`
- **Trigger**: `raycast` encounters any placed instance in `scene.draws()` whose `prototype` identifier is not found in `scene.prototypes`.
- **Consequence**: Using the `?` operator immediately terminates the entire raycast and returns `None`, discarding potential intersections with all subsequent valid prototype instances and terrain later in `scene.draws()`.
- **Evidence**:
  ```rust
  for draw in scene.draws() {
      let volume = scene.prototype(&draw.prototype)?;
      let Some(bounds) = volume.bounds_world(&draw.transform).ok().flatten() else {
          continue;
      };
  ```
  Line 140 aborts the function with `?`, whereas line 141 skips unresolvable bounds with `continue`.
- **Uncertainty**: In canonical generator output, all draws reference registered prototypes; this condition triggers only if external mutation or partial prototype removal leaves a dangling instance reference.

---

#### Finding 3: Corrupt or version-mismatched session file permanently blocks entry without quarantine or fallback
- **File / Line**: `apps/explorer/src/wetland.rs:102`
- **Trigger**: An existing `wetland-session.json` has invalid JSON, an edit out of range, or a mismatched generator version/seed.
- **Consequence**: `SavedWetland::load` returns `Err`, which aborts `Runtime::load` via `?`. The UI catches the error and sets `self.failed = true`, keeping the user trapped on the title menu with "Unable to enter: ...". Unlike the sandbox mode (`Explorer::new`), there is no backup/quarantine into a `.recovery-N.json` file or fallback to clean showcase state, locking out the user until manual file deletion.
- **Evidence**:
  In `Runtime::load`:
  ```rust
  let saved = SavedWetland::load(&directory.join(wetland_state::SAVE_FILE), GENERATOR, SEED)?;
  ```
  In `WetlandApp::draw`:
  ```rust
  Ok(Err(e)) => {
      self.loading = None;
      self.status = format!("Unable to enter: {e}");
      self.failed = true;
  }
  ```
  Contrast with `apps/explorer/src/lib.rs:198-234` where load failure quarantines corrupt files and falls back to world generation.
- **Uncertainty**: Strict fail-stop may be intentional for headless automated test runs to detect corrupt saves immediately.

## Lead triage

Directory fsync: retained on supported Linux/Android platforms; failure accurately reports durability uncertainty. Windows is not a supported target. Missing prototype: rejected as unreachable through DetailScene public mutation APIs, which validate references. Corrupt session lockout: verified in Runtime::load; recovery UX remains a required correction.
