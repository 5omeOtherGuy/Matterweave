# Production wetland automatic-detail integration

DeepSeek V4.1 Flash/max workers `w_cc534e57` and exact-session continuation `w_43c2a7a3`; lead owns integration and Android acceptance.

- **Actions Taken:** Replaced the static source-mesh graphics adapter with camera/viewport-driven DetailRuntime selection, bounded coarse realization, resident geometry installation and instance-only changes. Source collision remains independent.
- **Issues & Friction:** Initial tests incorrectly compared cache counters as source state and indexed a combined view/projection matrix as a pure projection; both failed and were corrected against actual contracts. Long builds exceeded worker deadlines; code preserved. Lead found unchanged-camera caching could starve capped coarse work.
- **Decisions & Rationale:** Keep source geometry ready, realize at most two new coarse levels per prepare, install full geometry only when changed or the renderer is recreated. Continue stationary bounded prepares while deferred work makes progress; avoid endless retries when no progress is possible.
- **Solutions Applied:** Seven production detail tests pass, including stationary convergence, revision refresh, source immutability and recreation. Wetland tests19 pass/1ignored; the full-map ignored integration was then explicitly run and passed in38.23s. Lead completes strict Clippy and independent review after deadline.
- **Insights:** A per-frame work cap needs a continuation signal even without camera movement. Module tests alone do not prove the production renderer path or Android transition quality; Android remains NOT RUN at this handoff.
