# Local coordination board

`board.py` is a trusted-local, bounded access gate for one lead and its workers. It uses
Python 3.10+ standard-library SQLite, Linux `/proc`, Git and `flock`. It performs no model
calls, service calls, third-party Python imports, build commands or process termination. The
protocol is described in the [v0.3 delivery plan](../../docs/V0.3.md).

This is a protocol, **not authentication or a filesystem sandbox**. Actors self-declare
identity; a worker with direct database or filesystem access can bypass the board, and a board
update cannot stop an agent. Keep the database on a local filesystem in the canonical
Git-common-directory run with one inbox consumer per actor, and do not point ordinary workers
at raw SQLite.

## Start a run

Choose a unique run ID and record the paths in every launch contract:

```bash
RUN_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/matterweave-runs/v03-unique-run"
mkdir -p "$RUN_DIR"
BOARD_DB="$RUN_DIR/board.sqlite"
BOARD_TOOL=/absolute/checkout/tools/coordination/board.py
python3 "$BOARD_TOOL" --db "$BOARD_DB" init --run v03-unique-run --actor lead
python3 "$BOARD_TOOL" --db "$BOARD_DB" hold --run v03-unique-run --actor lead \
  --leader-id ACTUAL_NATIVE_LEAD_TASK_ID > "$RUN_DIR/holder.json"
```

- `init` creates the database exclusively (it fails if the file exists) and records the run and
  lead.
- `hold` runs until it exits and owns an exclusive kernel lock on `<db>.lock`. It writes one
  JSON line containing `epoch` and then supervises: `--leader-id` records a native task ID and
  is stopped explicitly by its owner; `--leader-pid PID` tracks a genuinely dedicated lead
  process and exits when that exact boot/PID/start identity disappears. A second holder fails.
  The native lead task ID is distinct from the holder PID.
- `process --pid PID` prints the `/proc` identity (`pid`, `start`, `pgid`, `sid`, `boot`), or
  fails if the process is not alive.

Every other command takes one JSON object from `--file PATH` or stdin (`-`, the default);
requests cap at 16 KiB. Every request includes `actor` and `run` (bounded ASCII identifiers);
lead operations also include the emitted `epoch`. Output is compact sorted JSON on stdout;
errors are a JSON object on stderr with exit code 2.

## Lead operations

| Verb | Request fields and result |
| --- | --- |
| `snapshot` | Returns `version`, `current` and `reconciled`. Available during recovery. |
| `recovery` | Returns the state fingerprint, run, version, previous leader, and per task its attempt, worker, runtime, current tree (`head`, `changes_sha256`, `clean`) and submission, plus a pending count. Available during recovery. |
| `reconcile` | `fingerprint`, `tasks` (per current task a `tree` object and inactivity `proof` plus `evidence`), `evidence`, optional `previous_leader`. Rejects if the fingerprint moved; marks the startup gate open. |
| `replace` | `expect` (current version) and `current` (bounded to 8 KiB). Compare-and-swap; concurrent replacements with the same expected version have exactly one winner. The uploaded file is input, not a live ledger. |
| `inspect` | `task`, `attempt`; returns that task's status row including submission and applied evidence. |
| `quiescence` | `task`, `attempt`, `proof` (bounded to 2 KiB); records quiescence. |
| `send` | Worker `send` fields plus the lead epoch. |

The startup gate stays closed until `reconcile` succeeds: only `snapshot`, `recovery` and
`reconcile` are accepted before then. The reconcile fingerprint covers current state, pending
actions, receipts and submissions; any intervening change requires rereading. The first
reconcile of an empty board supplies `{"fingerprint":"RETURNED_FINGERPRINT","tasks":{},"evidence":"PATH_TO_STARTUP_INSPECTION"}`.

### Assignment shape

`replace`'s `current` holds `tasks`, `decisions` and an optional `objective`. A task requires:
`attempt`, `worker`, `route`, `effort`, `base`, `worktree` (canonical absolute path),
`paths` (literal repository-relative, no globs), `resources`, `dependencies`, `interfaces`,
`checks`, `runtime`, `evidence`, `state`. A decision requires `version`, `body` and `evidence`.

```json
{
  "tasks": {
    "terrain": {
      "attempt": 1, "worker": "terrain-a1", "route": "claude-code/opus-5",
      "effort": "medium", "base": "ACTUAL_FULL_BASE_REVISION",
      "worktree": "/absolute/canonical/worktree", "paths": ["crates/terrain/src"],
      "resources": [], "dependencies": [], "interfaces": {"jobs": 1},
      "checks": ["Exact acceptance commands and behavioral checks"],
      "runtime": {"kind": "native", "id": "ACTUAL_NATIVE_WORKER_TASK_ID"},
      "evidence": "/absolute/run/evidence/terrain-a1", "state": "ready"
    }
  },
  "decisions": {
    "jobs": {"version": 1, "body": "Immutable job input/result contract.", "evidence": "docs/path.md"}
  }
}
```

Runtime kinds are `unlaunched`, `process` and `native`. An external process is allocated with
`"runtime":{"kind":"unlaunched"}`, launched through its owned supervisor in an isolated
session (for example `setsid`), then registered from `process --pid PID` through a second lead
CAS before any work is allowed. A process worker must have PID = process-group ID = session
ID. Native tasks use the exact native task ID. The worker waits at this boundary until its
runtime is registered.

### Lifecycle, ownership and acceptance

Only the lead replaces assignments and lifecycle states. The enforced lifecycle is
`ready -> running -> submitted -> accepted`, with `blocked` and `revoked`. Worker `submit`
records immutable completion evidence; `accepted:false` and
`acceptance_state:awaiting_lead_review` mean a successful submission awaiting review. The lead
advances the snapshot to `submitted`, reviews, verifies quiescence, and accepts with
`acceptance:{"revision":"FULL_REVISION","evidence":"CHECKED_EVIDENCE_POINTER"}`. Submission and
acceptance require the current clean worktree HEAD; further edits invalidate acceptance. Lead
review evidence remains an assertion, not an automatically verified test result.

Ownership is immutable within an attempt. Literal repository-relative paths cannot overlap
across active tasks, and worktrees and exclusive resource strings cannot have two active
owners. Dependencies must name current tasks, have no cycles, and be accepted before dependent
work passes `check`; keep an accepted dependency in the snapshot until its dependents finish.

### Recovery and quiescence

Stop the exact owned worker and its children through its actual supervisor first, then call
lead `quiescence` with `task`, `attempt` and:

```json
{"proof":{"tree":{"head":"ACTUAL_HEAD","changes_sha256":"ACTUAL_HASH","clean":true},
 "runtime_id":"ACTUAL_NATIVE_WORKER_TASK_ID","verified_inactive":true,
 "evidence":"POINTER_TO_SUPERVISION_AND_PRESERVATION_EVIDENCE"}}
```

The tree hash covers tracked diff contents and untracked file contents, not only filenames.
Process runtime proofs omit the native-only fields; the helper checks the original process
identity and every live member of its owned Linux session. Native proofs are explicit manual
supervision attestations, not PID checks. The board never sends termination signals; it
rechecks the proof and tree before releasing ownership.

Quiescence closes worker activity for that attempt. A replacement must increment the attempt
and use a never-before-used worker instance; late old-attempt status or submission is
rejected. Closing as `accepted` or `revoked` also requires quiescence. Closed tasks are
archived by removing them after quiescence, and an archived task ID cannot be reused. An
unchanged closed task may remain only while useful for acceptance or dependencies; historical
submissions remain in SQLite without a routine unbounded listing command.

On holder restart, the new epoch invalidates old lead requests and closes the startup gate.
Read `recovery` and `snapshot`, inspect targeted submissions/statuses and unresolved messages,
and supply each current task's tree and inactivity proof under `tasks`. All current owned
workers must be inactive before recovery opens writes; replace their attempts before resuming.
The previous process lead must be inactive unless the same exact process resumes. For a
different native lead, supply `previous_leader` with exact `runtime_id`, `verified_inactive:true`
and evidence; for the same native lead resuming its holder, supply exact `runtime_id`,
`same_lead_resume:true` and evidence that it is the sole active lead. The holder lock cannot
independently prove native model liveness.

## Worker operations

The common request fields are:

```json
{"actor":"terrain-a1","run":"v03-unique-run","task":"terrain","attempt":1}
```

| Verb | Additional fields and result |
| --- | --- |
| `view` | Returns the assignment and only its applicable current decisions (bounded to 6 KiB); no global task list. |
| `check` | Validates active ownership, registered runtime, accepted dependencies, explicitly applied interfaces and no pending decision/control/blocker. Run before editing phases, after long tools and before submission. |
| `status` | Reads the current status; add `expect` and `body` to replace it atomically. Version 0 is initially empty. Status churn sends no messages. |
| `apply` | `decision`, `decision_version`, `evidence`; durably records application to this exact task/attempt. The version must equal the current assignment. |
| `submit` | `revision`, `evidence`; submits the clean, stable worktree HEAD. Identical retries are safe. Only the lead accepts. |
| `send` | `to`, `kind`, sender-unique `key`, `body:{"action":"…","evidence":"POINTER"}`. The recipient is this task's owner or the lead. |
| `inbox` | Returns the same persisted ordered batch until its exact boundary is acknowledged. No cursor argument. |
| `message` | `id`; returns exactly one received message plus its durable effect, if any. |
| `effect` | `id`, `disposition` (`done`, `deferred`, `superseded`), `evidence` pointer. Records a verified effect or persistent deferred action. |
| `ack` | `through` must equal the returned delivered batch boundary; every message must already have an effect record. |
| `pending` | Lists unresolved/deferred actions; optional `after` for pagination. |

The worker startup sequence is `view` (adopt the interface), `inbox`, `apply` each decision,
`effect` each returned message, `ack` the exact `through`, then `check`. On replay, inspect a
message with its exact `id` and verify durable/external effects before repeating any operation;
a successful `ack` alone does not authorize editing.

Allowed message kinds are `decision`, `blocker`, `question`, `update`, `handoff` and `control`.
A `decision` send is lead-only and includes `decision` plus `decision_version` matching the
current assignment; a replacement decision in the snapshot increments its version and names
`supersedes: PREVIOUS_VERSION`. A superseding message names `supersedes: PREVIOUS_MESSAGE_ID`
and must target the same task/attempt/recipient; recipients record the old message's
`superseded` disposition before handling the new one. Apply an interface before marking its
decision message `done` — receipt is not application. Deferred urgent actions remain in
`pending` and block `check` and `submit` until resolved or explicitly superseded. Inboxes do
not wake models; urgent work still needs native steering or an owned-process stop by the lead.

Lead `inspect` reads one task's status. Lead `pending`/`message` may add `for_actor` with a
historical worker ID; lead `effect` with `for_actor` permits only an explicit `superseded`
disposition, preserving obsolete-attempt actions without reviving that worker.

There is **no exactly-once transaction across SQLite, Git, external tools and processes**.
Recording `effect` is not execution of the described side effect; a crash before recording an
external consequence requires inspecting the actual artifact before retrying. Identical `send`
keys return the original message ID, while changed content with that key fails. Completed
effects and submissions accept identical retries. CAS replacement and status retries return a
version conflict after an ambiguous successful write, so reread the actual current state;
never invent a fresh retry key on uncertainty.

## Bounds and provenance

Enforced UTF-8 limits count compact JSON serialization and therefore include JSON escaping.
Oversized content fails; nothing is silently truncated.

| Item | Limit |
| --- | --- |
| Request object | 16 KiB |
| `current` snapshot | 8 KiB |
| Scoped `view` | 6 KiB |
| `status` body | 2 KiB |
| Message body | 1 KiB |
| Inbox / pending batch | At most four messages and 8 KiB serialized |
| Application evidence | 512 B per pointer, 2 KiB combined |
| Consequence evidence | 1 KiB |
| Submission / proof document | 2 KiB |

Inbox pagination may return fewer than four messages to meet the byte cap. One consumer per
inbox is a trusted launch rule; no actor authentication or multi-consumer arbitration exists.
Large artifacts stay at explicit evidence paths.

The board was written after evaluating the machine-local `multi-model-orchestration` repository
revision `ba0c3242d414d93411f4cc7dcf916cae6066cef7`, specifically
`harnesses/codex/skills/multi-model-orchestration/scripts/coord.py` and its
`references/coordination.md`. That candidate supplies SQLite CAS/retry concepts but has
broader inbox reads, permissive acknowledgment and no lifecycle enforcement; no license file
was present at its root, so no source was copied. This is an independent implementation using
Python's standard-library `sqlite3` and `fcntl`; it does not import or modify installed skill
files. SQLite is public domain and Python retains its PSF license; no project license is
selected here. Runtime versions are the host's, not newly pinned dependencies.

No distributed/network filesystem support, model launching, paid-route fallback, worker
killing, arbitrary history/log retrieval, automatic source fencing, cryptographic identity, or
automatic test/review acceptance is provided. Recovery fingerprints summarize the durable
database; the lead must inspect referenced pending actions and evidence. Git operations,
actual worker quiescence and native lead liveness remain the lead's verified supervision
responsibility.

## Verification

Run the fixture rehearsals (the default scratch/log parent stays under `/mnt/bench`; override
`MATTERWEAVE_COORD_TEST_ROOT` only with an appropriate local fixture directory):

```bash
python3 -m unittest discover -s tools/coordination -p 'test_*.py' -v
python3 tools/check_docs.py
```

Bootstrap verification on 2026-09-07: the 16-case fixture suite passed (100.437 s), then all
four affected cases passed again (39.318 s) after review tightened blocked-task checks, lead
epoch validation, retained interface-application evidence and historical-worker
pending-action inspection. The suite includes concurrent CAS, bounds/pagination, status
replacement, ack skipping, crash replay, stale attempts, explicit interface
application/supersession, surviving process children, native opaque IDs and restart
attestations, single-holder exclusion, live previous lead rejection, dirty submissions and
dependency cycles. These are local protocol rehearsals, not mobile or model-performance
measurements. No failing test remained, and `python3 tools/check_docs.py` passed; no engine
build or device test was run.

See also [DEVELOPMENT](../../docs/DEVELOPMENT.md) for the board operations guide and workspace
commands.
