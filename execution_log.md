# v0.3 execution log

Updated: 2026-09-07. This log records executed work and observations; [delivery plan](docs/V0.3.md) records intended gates. Model cards are routing priors, not measured Matterweave competence. Raw worker logs and comparative evidence remain under `/mnt/bench/matterweave-dev/v0.3/`; durable summaries belong here and in release artifacts. Lead owns this log; workers submit short evidence handoffs.

## 01 — Preserve the plan and start coordination bootstrap

- **Actions Taken:** Added v0.3 delivery/board protocol and linked HANDOFF, STATUS and ROADMAP; opened PR #3. Ran documentation checks and the installed coordination helper suite (11 passed). Created isolated board/bootstrap and fixed read-only investigation worktrees at `fc7a939`. Verified ADB is authorized and the installed app is v0.2.0/code 2.
- **Issues & Friction:** Existing helper does not enforce attempt ownership, small aggregate inbox output, applied decisions or process fencing. Its actor identity is self-declared. Existing CI runs host/Android builds even for documentation changes; those checks are pending.
- **Decisions & Rationale:** Dispatch one Astra/high bootstrap worker, plus read-only Muse/high physics and Opus/medium streaming investigations; three workers plus lead is the full current capacity. Feature writers wait for the board gate. Restrict external tools and descendants per invocation.
- **Solutions Applied:** Lead requested direct SQLite CLI plus a small lock holder after the bootstrap worker proposed a Unix-socket supervisor; this reduces coordination infrastructure. Shared app/build/phone resources remain lead-owned.
- **Insights:** The skill provides sound storage primitives but the operational guarantees require explicit implementation and rehearsal. Read-only investigations can proceed while those safeguards are built. No claim of validated board recovery yet.

## 02 — First Muse source investigation

- **Actions Taken:** OpenCode ran exact `opencode/muse-spark-1.3-contributor-free`, requested high, explicit fixed workspace, plan agent with read/search only. Completed in 25.43 seconds, exit 0, two reported step finishes with zero model cost. Captured session `ses_f8248309bffeSgYyFbGjoSkHl0` and source-linked proposal for a breakable wall.
- **Issues & Friction:** Raw events were about 238 KiB despite a concise assignment. The handoff contains a budget-analysis error: seven full cubes fracture to 56 single-voxel bodies, which cannot be fractured again, so its suggested late `56+7` scenario does not follow. Some proposed tests assert incidental mesh counts or duplicate existing coverage.
- **Decisions & Rationale:** Retain useful source mapping and the fully-fracturable bounded-structure idea. Independently verify geometry/physics assumptions and tighten acceptance tests before assigning implementation.
- **Solutions Applied:** Extracted only final prose and per-step usage into lead context; raw tool transcripts stay in artifacts. Marked the handoff partially useful rather than accepting every statement.
- **Insights:** This first real code-reading task supports Muse as a fast source investigator, but not unreviewed correctness authority. It agrees with the card's bounded-task recommendation and its warning that production repair burden is unmeasured. One task is not a comparative benchmark.

## 03 — Opus streaming investigation and integration branch

- **Actions Taken:** Claude Code completed the fixed-source streaming investigation in 96.91 seconds on `claude-opus-5`, medium requested, with no permission denials. PR #3 passed docs/host/Android CI and merged as `b6cb9c4`; created `codex/v0.3-integration`. Backed up the phone's current private app files before test manipulation.
- **Issues & Friction:** Opus reported 96,742 cache-creation and 409,497 cache-read input tokens across calls, despite a short brief; raw events were about 293 KiB. Reported $1.3474965 is list-price equivalent including a Haiku helper, not verified cash spend. Its design's global residency epoch could discard useful retained-chunk work during travel; the near-radius collision argument requires explicit movement/availability guarantees, not a speed assumption.
- **Decisions & Rationale:** Keep single-thread authoritative world ownership, immutable inputs, bounded preparation and synchronous collision publication as useful design constraints. Avoid accepting its Arc conversion and budget guesses without workload evidence. Tighten next-worker startup context and require explicit race/movement rules.
- **Solutions Applied:** Captured final artifact and usage separately from full events. Plan to use Claude safe mode with an explicit project contract/manual instruction reads on the implementation task, preserving subscription authentication, to reduce automatic customization context.
- **Insights:** Complete briefs did produce useful source-specific analysis, as the Opus card suggests, but medium effort did not prevent broad output/context overhead. Model-returned success is not an architectural acceptance check. Native shadow worker proposed a compatible lighting API and optional validated GPU timestamps; no shadow code written yet.
