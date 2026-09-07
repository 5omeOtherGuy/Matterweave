# Local v0.3 coordination board

`board.py` implements the bounded access gate described in
[the delivery protocol](../../docs/V0.3.md). It uses Python 3.10+ standard-library
SQLite, Linux `/proc`, Git and `flock`. No model calls, services, third-party Python
packages, build commands or process termination are performed by this helper.

This is a trusted local protocol, **not authentication or a filesystem sandbox**.
Actors self-declare identity. Workers with direct database/filesystem access can
bypass it. A board update cannot stop an agent. Keep the database on a local
filesystem in the canonical Git-common-directory run, with one inbox consumer per
actor. Do not point ordinary workers at raw SQLite queries or the skill helper.

## Start a run

Choose a unique run ID and record the resulting path in every launch contract:

```bash
RUN_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/matterweave-runs/v03-unique-run"
mkdir -p "$RUN_DIR"
BOARD_DB="$RUN_DIR/board.sqlite"
BOARD_TOOL=/absolute/checkout/tools/coordination/board.py
python3 "$BOARD_TOOL" --db "$BOARD_DB" init --run v03-unique-run --actor lead
python3 "$BOARD_TOOL" --db "$BOARD_DB" hold --run v03-unique-run --actor lead \
  --leader-id ACTUAL_NATIVE_LEAD_TASK_ID > "$RUN_DIR/holder.json"
```

Keep the last command running in a dedicated supervised terminal/process. It
emits one JSON line with `epoch` and then owns an exclusive kernel lock until it
exits. A second holder fails. The native lead task ID is distinct from the
holder's PID; no shared desktop daemon needs to be identified or stopped.
Alternatively, `hold --leader-pid PID` records a genuinely dedicated lead process
and exits when that exact boot/PID/start identity disappears. Do not use a short
shell's PID. A native holder must be stopped explicitly by its owner at closeout.

All remaining commands take one JSON object from `--file PATH` or stdin. Every
request includes `actor` and `run`; lead-only operations also include the emitted
`epoch`. For example, write these fields plus the appropriate operation fields to
`$RUN_DIR/request.json`, then run:

```bash
python3 "$BOARD_TOOL" --db "$BOARD_DB" recovery --file "$RUN_DIR/request.json"
python3 "$BOARD_TOOL" --db "$BOARD_DB" reconcile --file "$RUN_DIR/reconcile.json"
```

The first request is `{"actor":"lead","run":"v03-unique-run","epoch":"EMITTED_EPOCH"}`.
For a new empty board, the reconcile request adds
`"fingerprint":"RETURNED_FINGERPRINT","tasks":{},"evidence":"PATH_TO_STARTUP_INSPECTION"`.
The fingerprint includes current state, pending actions, receipts and submissions;
any intervening change requires rereading. The startup gate remains closed until
reconciliation succeeds. `snapshot` and `recovery` are available during recovery;
workers cannot read assignments or mutate state until it succeeds.

## Allocate and launch

The lead reads `snapshot`, then calls `replace` with `expect` equal to its current
version and `current` containing `tasks`, `decisions` and optional `objective`.
Concurrent replacements with the same expected version have exactly one winner.
The losing caller rereads and reconciles. The uploaded file is input, not another
live ledger. Example `current` (substitute actual revision, paths and task ID):

```json
{
  "tasks": {
    "terrain": {
      "attempt": 1,
      "worker": "terrain-a1",
      "route": "claude-code/opus-5",
      "effort": "medium",
      "base": "ACTUAL_FULL_BASE_REVISION",
      "worktree": "/absolute/canonical/worktree",
      "paths": ["crates/terrain/src"],
      "resources": [],
      "dependencies": [],
      "interfaces": {"jobs": 1},
      "checks": ["Exact acceptance commands and behavioral checks"],
      "runtime": {"kind": "native", "id": "ACTUAL_NATIVE_WORKER_TASK_ID"},
      "evidence": "/absolute/run/evidence/terrain-a1",
      "state": "ready"
    }
  },
  "decisions": {
    "jobs": {
      "version": 1,
      "body": "Immutable job input/result contract and explicit publication checks.",
      "evidence": "docs/path-to-accepted-interface.md"
    }
  }
}
```

For external processes allocate with `runtime:{"kind":"unlaunched"}`, launch the
worker through its owned supervisor in an isolated session (for example `setsid`),
then retrieve its identity using `board.py --db DB process --pid PID`. Replace the
runtime with that returned object through another lead CAS before allowing work.
A process worker must have PID = process-group ID = session ID. The unlaunched
attempt cannot pass `check` or submit. Native tasks use the precise native task ID.
The model must wait at this startup boundary until its runtime is registered.

Only the lead replaces assignments and lifecycle states. The enforced lifecycle
is `ready -> running -> submitted -> accepted`, with `blocked` and `revoked`.
Worker `submit` records immutable completion evidence. Its `accepted:false` and
`acceptance_state:awaiting_lead_review` mean successful submission awaiting review,
not a failed submission; the lead then advances the
snapshot to `submitted`, reviews, verifies quiescence, and accepts with
`acceptance:{"revision":"FULL_REVISION","evidence":"CHECKED_EVIDENCE_POINTER"}`.
Submission and acceptance require the current clean worktree HEAD. Further edits
invalidate acceptance checks. Lead review evidence remains an explicit assertion,
not an automatically verified test result.

Ownership is immutable within an attempt. Literal repository-relative paths
(no glob patterns) cannot overlap across active tasks. Worktrees and exclusive
resource strings cannot have two active owners. Dependencies must name current
tasks, have no cycles, and be accepted before dependent work passes `check`.
Retain an accepted dependency in the current snapshot until its dependents finish.

## Copyable worker launch brief

The lead substitutes actual run, worker, task and attempt values in these files.
The leaf needs this section and its bounded `view`, not the full board history.
Set `BOARD_TOOL` and `BOARD_DB` to the absolute paths in the launch contract.
Create `worker.json` with:

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1}
```

Read the scoped assignment and ordered inbox:

```bash
python3 "$BOARD_TOOL" --db "$BOARD_DB" view --file worker.json
python3 "$BOARD_TOOL" --db "$BOARD_DB" inbox --file worker.json
```

After actually adopting the interface described in `view`, create `apply.json`:

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1,"decision":"jobs","decision_version":1,"evidence":"PATH_TO_ACTUAL_APPLICATION_EVIDENCE"}
```

```bash
python3 "$BOARD_TOOL" --db "$BOARD_DB" apply --file apply.json
```

For each returned inbox message, verify the action/consequence and write
`effect.json` with its actual ID (example 12):

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1,"id":12,"disposition":"done","evidence":"PATH_TO_VERIFIED_CONSEQUENCE"}
```

Use `deferred` plus a durable todo pointer if it is still unresolved; unresolved
`decision`, `control` and `blocker` messages keep editing/submission checks closed.
Repeat `effect` for every message before acknowledging. In `ack.json`, use exactly
the batch's returned `through` value (example 15):

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1,"through":15}
```

```bash
python3 "$BOARD_TOOL" --db "$BOARD_DB" effect --file effect.json
python3 "$BOARD_TOOL" --db "$BOARD_DB" ack --file ack.json
python3 "$BOARD_TOOL" --db "$BOARD_DB" pending --file worker.json
python3 "$BOARD_TOOL" --db "$BOARD_DB" check --file worker.json
```

Only proceed with dependent edits when `check` succeeds. Repeat inbox handling and
`check` at phase boundaries, after long tools, and before submission. On replay,
inspect `message` with its exact `id` and verify durable/external effects before
repeating any operation. A successful `ack` alone does not authorize editing.

## Worker commands

For worker operations the common request fields are:

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1}
```

Use the same file/stdin command pattern for these verbs:

| Verb | Additional request fields and result |
| --- | --- |
| `view` | Assignment and only its applicable current decisions; no global task list. |
| `check` | Validate active ownership, registered runtime, accepted dependencies, explicitly applied interfaces and no pending decision/control/blocker. Run before editing phases, after long tools and before submission. |
| `status` | Read current status; add `expect` and `body` to replace it. Version 0 is initially empty. Status churn sends no messages. |
| `apply` | `decision`, `decision_version`, `evidence`; durably records application to this exact task/attempt. Version must equal current assignment. |
| `submit` | `revision`, `evidence`; submit the clean, stable worktree HEAD. Identical retries are safe. Only the lead accepts. |
| `send` | `to`, `kind`, sender-unique `key`, `body:{"action":"One action/question","evidence":"POINTER"}`. Recipient is this task's owner or lead. |
| `inbox` | Return the same persisted ordered batch until its exact boundary is acknowledged. No cursor argument. |
| `message` | `id`; inspect exactly one received message plus its durable consequence. |
| `effect` | `id`, `disposition` (`done`, `deferred`, `superseded`), `evidence` pointer. Record a verified effect or persistent deferred action. |
| `ack` | `through` must be exactly the returned delivered batch boundary; each message must have a durable consequence/defer record. |
| `pending` | Optional `after` for pagination of unresolved/deferred actions, including those already receipt-acknowledged. |

Lead targeted `inspect` with `task` and `attempt` reads the current status,
submission and application evidence. Lead `pending`/`message` can add `for_actor`
with a historical worker ID for bounded recovery inspection. Lead `effect` with
`for_actor` permits only explicit `superseded` disposition; this preserves and
resolves obsolete-attempt actions without reviving that worker. Lead `send`
additionally requires its epoch.
Allowed message kinds are `decision`, `blocker`, `question`, `update`, `handoff`,
`control`. A decision send is lead-only and includes `decision` plus
`decision_version` matching the current assignment. A replacement decision in the
snapshot increments its version and names `supersedes: PREVIOUS_VERSION`.
A superseding message names `supersedes: PREVIOUS_MESSAGE_ID` and must target the
same task/attempt/recipient. Supersession is explicit: recipients record the old
message's `superseded` disposition, then handle the new pending action.

Apply an interface before marking its decision message `done`. Receipt is not
application. Deferred urgent actions remain in `pending` and block `check` and
`submit` until resolved or explicitly superseded. Inboxes do not wake models;
urgent work still needs native steering or an owned-process stop by the lead.

On crash replay, read each message's durable `effect` and verify actual external
side effects before doing anything again. The tests rehearse a durable board effect
followed by a crash before receipt acknowledgment. There is **no exactly-once
transaction across SQLite, Git, external tools and processes**; recording `effect`
is not execution of the described side effect. A crash before recording an
external consequence requires inspecting the actual artifact before retrying.
Identical send keys return the original message ID; changed content with that key
fails. Completed effects and submissions accept identical retries. CAS replacement
and status retries return a version conflict after an ambiguous successful write,
so reread their actual current state. Never invent a fresh retry key on uncertainty.

## Quiescence and recovery

Stop the exact owned worker and its children using its actual supervisor first.
Then inspect/preserve the worktree and call lead `quiescence` with task/attempt and:

```json
{
  "proof": {
    "tree": {"head":"ACTUAL_HEAD","changes_sha256":"ACTUAL_HASH","clean":true},
    "runtime_id": "ACTUAL_NATIVE_WORKER_TASK_ID",
    "verified_inactive": true,
    "evidence": "POINTER_TO_NATIVE_SUPERVISION_AND_PRESERVATION_EVIDENCE"
  }
}
```

Lead `recovery` reports the actual `tree` object for each current task. It hashes
tracked diff contents and untracked file contents, not only filenames. Process
runtime proofs omit the native-only fields; the helper checks the original process
identity and every live member of its owned Linux session. Native proofs are
explicit **manual supervision attestations**, not PID checks. Processes that escape
an owned session cannot be discovered reliably by this tool: prohibit escape in
launch contracts and reconcile such descendants through their real supervisor.

The board never sends termination signals. It rechecks the proof and tree before
releasing ownership. Quiescence closes worker activity for that attempt. A
replacement must increment the attempt and use a never-before-used worker instance;
late old-attempt status/submission is rejected. Closing as `accepted` or `revoked`
also requires quiescence, preventing reallocation through a renamed task. Archive
closed tasks by removing them after quiescence; archived task IDs cannot be reused.
An unchanged closed task remains in current state only while useful for acceptance
or dependencies. Historical submissions remain in SQLite, without a routine
unbounded listing command.

On holder restart, a new epoch invalidates old lead requests and closes startup.
Read `recovery` and `snapshot`, inspect targeted submissions/statuses and unresolved
messages, and provide each current task's tree and inactivity proof under `tasks`.
All current owned workers must be inactive before recovery opens writes. Replace
their attempts before resuming work; recovery does not resurrect an old worker.
The previous process lead must be inactive unless the same exact process resumes.
For a different native lead supply `previous_leader` with exact `runtime_id`,
`verified_inactive:true` and evidence. For the same native lead resuming its holder,
supply exact `runtime_id`, `same_lead_resume:true` and evidence that it is the sole
active lead. The holder lock cannot independently prove native model liveness.

## Bounds, provenance and verification

Enforced UTF-8 limits count compact JSON serialization (therefore conservatively
include JSON escaping): current snapshot 8 KiB; scoped startup view 6 KiB; status
body 2 KiB; message body 1 KiB; inbox and pending batches at most four messages and
8 KiB serialized. Requests cap at 16 KiB. Oversized content fails; nothing is
silently truncated. Application evidence caps at 512 bytes per pointer and 2 KiB
combined; consequence evidence caps at 1 KiB; submission/proof documents cap at
2 KiB. Large artifacts stay at explicit evidence paths. Inbox pagination may return
fewer than four messages to meet the byte cap. One consumer per inbox is a trusted
launch rule; no actor authentication or multi-consumer arbitration is implemented.

Evaluated reuse candidate: machine-local `multi-model-orchestration` repository
revision `ba0c3242d414d93411f4cc7dcf916cae6066cef7`,
`harnesses/codex/skills/multi-model-orchestration/scripts/coord.py` and
`references/coordination.md`. The candidate supplies SQLite CAS/retry concepts but
has broader inbox reads, permissive acknowledgment and no lifecycle enforcement.
No license file was present at its repository root during inspection, so no source
was copied. This is an independent project implementation using Python's
standard-library `sqlite3` and `fcntl`; it does not import or modify installed skill
files. SQLite is public domain; the Python runtime retains its PSF license. No new
project license is selected here. Runtime versions are those of the host Python,
SQLite and Git, not a newly pinned external dependency.

Run the local fixture rehearsals (default scratch/log parent stays under
`/mnt/bench`; override `MATTERWEAVE_COORD_TEST_ROOT` only with an appropriate local
fixture directory):

```bash
python3 -m unittest discover -s tools/coordination -p 'test_*.py' -v
python3 tools/check_docs.py
```

No distributed/network filesystem support, model launching, paid-route fallback,
worker killing, arbitrary history/log retrieval, automatic source-file fencing,
cryptographic identity, or automatic test/review acceptance is provided. Recovery
fingerprints summarize the durable database; the lead must inspect referenced
pending actions and evidence. Git operations, actual worker quiescence and native
lead liveness remain the lead's verified supervision responsibility.

Bootstrap verification on 2026-09-07: the 16-case fixture suite passed (100.437 s),
then all four affected cases passed again (39.318 s) after final review tightened
blocked-task checks, lead epoch validation, retained interface-application evidence
and historical-worker pending-action inspection. The suite includes concurrent
CAS, bounds/pagination, status replacement, ack skipping, crash replay, stale
attempts, explicit interface application/supersession, surviving process children,
native opaque IDs and restart attestations, single-holder exclusion, live previous
lead rejection, dirty submissions and dependency cycles. These are local protocol
rehearsals, not mobile or model-performance measurements. No failing test remained.
`python3 tools/check_docs.py` also passed; no engine build or device test was run.
