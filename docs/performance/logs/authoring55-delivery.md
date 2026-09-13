# Engineering log: reviewed PR #55 authoring-docs delivery

Worker: `engine/shared-services-authoring` owner (delegated docs delivery) ·
outcome Category: docs · 2026-09-13 · run:
`orchestration/opus-authoring55-review/` (consumed review `w_143c48ca`,
claude-opus-5 medium: **accept with one required sentence-level correction**).
Base: `main` `54ec529`. PR [#55](https://github.com/5omeOtherGuy/Matterweave/pull/55).

Scope: apply the review's four exact sentence-level corrections to
`docs/AUTHORING.md`, `docs/performance/logs/d4-3-authoring.md` and
`docs/performance/logs/d4-3-authoring-repair.md`, keep every other reviewed
sentence and the verbatim gate quotation unchanged, re-verify, push, and merge
PR #55 after the required CI checks pass. No runtime code, no device, no
`/mnt/bench`, no STATUS/HANDOFF/board edits, no D4/M6 closure.

## Actions Taken

- Read the review result and the three owned docs, plus the current audibility
  record in `docs/STATUS.md` (`:159` "Audio audibility is confirmed by the
  owner"; `:233-234` the owner's "Audio works 100% it is all yours."). The
  superseded passage at `docs/STATUS.md:749` is what the first freeze echoed.
- Applied the review's four corrections exactly, with rewrapping only:
  1. `docs/AUTHORING.md` sample-registration closing sentence now says the
     physical simultaneous-touch and lock/unlock observations are the open part
     of the D4.2 gate.
  2. `docs/AUTHORING.md` "D4.2 physical acceptance is still open" bullet now
     leaves only physical simultaneous multi-finger use and lock/unlock
     unverified and states that audible output is accepted on the owner's
     device confirmation (`STATUS.md`, "Audio works 100%").
  3. `docs/performance/logs/d4-3-authoring.md`: removed the "Audible output is
     unverified" bullet from the open-items list and added "Audible output is
     accepted on the owner's device confirmation (STATUS.md); host counters
     remain non-evidence." to the satisfied-evidence list.
  4. `docs/performance/logs/d4-3-authoring-repair.md`: finding 3's disposition
     now reads "gate items still open (physical simultaneous touch,
     lock/unlock)" and the proposed STATUS block now reads "D4.2's physical
     simultaneous touch and lock/unlock remain open device acceptance."
- Verified the corrections rather than assuming them: `git diff` shows exactly
  the four intended hunks in three files (14 insertions / 14 deletions), the
  verbatim accepted-gate quotation at
  `docs/performance/logs/d4-3-authoring.md:92` ("Both samples run without
  engine forks; functional input, physical simultaneous touch, lock/unlock,
  audible output and persistence observed.") is byte-identical, and the review's
  "Host audio proves nothing about devices" bullet is untouched.
- Reconciled `main` before merge: `origin/main` is still `54ec529`, which is
  already an ancestor of the branch head (the earlier `--no-ff` merge
  `be66ff4`), so no new merge was needed and no conflict existed.
- Ran the required checks locally: `python3 tools/check_docs.py` → PASS
  (228 Markdown files, 671 local links, 16 ADRs, 20 requirements) and
  `git diff --check` → PASS.
- Committed and pushed the corrections plus this log, waited for the PR's
  required checks on the new head, and merged PR #55 as a normal merge commit
  (no admin/bypass, no force push, no rebase of shared history).

## Issues & Friction

- The first freeze's audibility wording traced to a superseded `STATUS.md:749`
  passage, not to the current record (`:159`, `:233-234`); prose written from an
  old STATUS snapshot is exactly how an accepted device fact gets reopened.
- Correction 4 mattered more than its size: the repair log's proposed block is
  written verbatim for paste into `docs/STATUS.md`, so the stale "and audible
  output" clause would have regressed the shared record at the next integration
  update.
- Physical device evidence stays out of reach for this task by instruction: no
  phone, no descendants, no `/mnt/bench`. Physical simultaneous touch and
  lock/unlock therefore remain unrun here, not "passing".
- CI timing: PR #55's checks had already passed for the previous head; the new
  head invalidated them, so the merge waited on re-run checks. Polling was
  bounded instead of a tight loop.

## Decisions & Rationale

- Applied only the four supported corrections. Adding further "improvements"
  would have invalidated the Opus review, which explicitly closed the loop for
  these exact text changes ("no further review loop").
- Left `docs/STATUS.md`, `docs/HANDOFF.md` and the board untouched: this task
  owns the authoring branch only. The proposed STATUS text is recorded below in
  final, audibility-corrected form for the next integration delivery worker.
- Did not rebuild the snippet harness. Only prose changed; no code example was
  touched, and the review re-verified every cited signature against the frozen
  revision (`build_showcase`, `Scale::new`, `DetailVolume::new/set`,
  `Transform::new`, `add_prototype`, `place`, chooser wiring, audio constants).
- Merged with a merge commit on the existing branch instead of rebasing: the
  branch was already pushed and cleanly contained `main`, and a merge preserves
  the reviewed commit history without rewriting shared history.
- Kept the "Host audio proves nothing about devices" boundary intact: the mock
  backend, silent host/standalone runs and submitted-frame counters are still
  non-evidence; only the owner's device report closes D4.2 audibility.

## Solutions Applied

- `docs/AUTHORING.md`: two sentences corrected (sample-reuse scope; D4.2 open
  acceptance). Guide remains the reusable-product documentation, no branch
  names for unmerged settings (#53) or lighting (#54) work.
- `docs/performance/logs/d4-3-authoring.md`: audibility moved from the
  open-items list to accepted evidence with the owner confirmation; historical
  gate quotation and all other classifications preserved.
- `docs/performance/logs/d4-3-authoring-repair.md`: disposition and proposed
  STATUS text corrected; historical findings and dispositions otherwise intact.
- Delivery: corrections plus this log committed on
  `engine/shared-services-authoring`, pushed, required CI green, PR #55 merged.
- The shared status update for D4.3 (including the merge SHA) is proposed in
  `docs/performance/logs/d4-3-authoring-repair.md` and below; applying it is the
  next integration delivery worker's decision.

## Insights

- Reviewing worker-proposed STATUS text "as if applied" is a real merge guard:
  the defect was invisible in the log's own narrative and only harmful at paste
  time.
- A docs PR that dates its evidence must grep current STATUS entries rather than
  reuse earlier prose; the audibility flip survived one review round untouched
  because it looked like cautious language.
- The D4.2 acceptance picture after this PR is simple: sample reuse, functional
  input and persistence are satisfied; audible output is owner-accepted; only
  physical simultaneous multi-finger use and lock/unlock remain unrun device
  gates.

## Proposed shared-status update (not applied; next integration worker owns STATUS/board)

Proposed `docs/STATUS.md` addition, verbatim with the merge SHA filled in from
`git log --format='%H %s' --grep='Merge pull request #55' -1 main`:

> **D4.3 authoring documentation delivery (2026-09-13).** PR #55
> (`engine/shared-services-authoring`) merged as `<PR #55 merge SHA>` after the
> Opus review `w_143c48ca` accepted the corrections and all required checks
> passed. `docs/AUTHORING.md` and its logs now state D4.2 accurately: sample
> reuse, functional input and persistence are satisfied; audible output is
> accepted on the owner's device confirmation ("Audio works 100%"); physical
> simultaneous multi-finger use and lock/unlock remain unrun device acceptance.
> The guide documents `build_showcase(SHOWCASE_SEED)?` / `showcase.scene`,
> chooser registration via `experience.rs`/`wetland.rs`, and persistence as
> `World::load` + `world.attachment()`, and it no longer presents a production
> editor, compressed codecs or save-journal migration as D4.2 gates. Unmerged
> settings (#53) and lighting (#54) work is explicitly not shipped. Documentation
> only: no runtime code changed, and D4/M6 are not closed by this PR.

## Verification (exact commands)

- `python3 tools/check_docs.py` → `PASS: 228 Markdown files, 671 local links,
  16 ADRs and 20 requirements.`
- `git diff --check` → clean (no whitespace errors).
- `git diff` on the three files → four intended hunks only; verbatim gate
  quotation at `d4-3-authoring.md:92` byte-identical; no other reviewed text
  changed.
- `git merge-base --is-ancestor origin/main HEAD` → true (main reconciled).
- PR #55 required checks (`docs`, `host`, `android`) green on the merged head;
  merge performed without bypass.

## Unrun gates and limits

- Physical simultaneous multi-finger touch: **NOT RUN** (no device in scope).
- Lock/unlock device cycle: **NOT RUN** (no device in scope).
- Audible output: accepted on the owner's device confirmation, not re-measured
  here; host counters remain non-evidence.
- No performance, thermal or sustained-workload claims; no `/mnt/bench` work.
- Shared settings (#53) and proxy lighting (#54) are unmerged and unreviewed by
  this task and must not be reported as shipped.
- D4 and M6 remain open; this PR completes the D4.3 documentation slice only.
