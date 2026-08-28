# autonomy-sim delivery plan

## Phase 1 — runnable analytic ISR slice

The Phase 1 acceptance path is a scenario TOML driving a fixed-step Rust simulation, streamed through Axum WebSocket/REST to a CesiumJS client, with a moving platform visible and CoT written to a demo file. Work proceeds in vertical increments:

1. Freeze `autonomy-sim/v1` in `ARCHITECTURE.md`; scaffold Rust 2024 workspace and Vite frontend.
2. Move one scenario entity through the scheduler and display it from `/api/v1/stream`.
3. Add the entity/domain model, auditable behavior-tree composites/leaves, and area-search, persistent-surveillance, and comms-relay playbooks.
4. Add `NetworkBackend` and `PropagationModel`; implement deterministic analytic links, link transition detection, synthetic Ditto traffic, and the explicit SigForge adapter stub.
5. Add CoT rendering plus file/UDP/TCP sinks and verify valid PLI/track event output.
6. Render all platforms and live link flaps in Cesium 2D/3D, with per-transport styling and optional photorealistic tiles.
7. Ship the ISR demo, tests, launch commands, license, and a documented end-to-end smoke check.

Phase 1 is complete when a clean checkout can run the analytic backend without CORE/EMANE, observe entities moving, see at least one mesh link transition up/down, fetch the REST snapshot, and inspect emitted CoT XML. No lethal-engagement logic is accepted.

## Phase 2 — SigForge and richer TAK C2

- Replace `SigForgeBackend`'s unavailable implementation with REST/gRPC node registration, mobility event publication, and WebSocket link/traffic ingestion.
- Preserve autonomy-sim IDs while persisting NEM-ID mappings and reconnect cursors.
- Validate analytic-versus-SigForge traces using identical scenarios.
- Add authenticated TAK Server transport, certificate handling, and a quarantined CoT ingest path.
- Convert only allow-listed, authenticated C2 tasking into proposed mission changes; require explicit human approval before activation.

## Phase 3 — propagation and environment fidelity

- Add outdoor terrain/LOS sampling, antenna patterns, and weather loss.
- Add indoor/urban plug-ins for walls, floors, body blocking, multipath, and non-geospatial robot maps.
- Add an ns-3 propagation/network adapter behind the existing traits.
- Introduce uncertainty fields and provenance in link state without changing v1 field meaning.
- Record/replay backend outputs for deterministic regression tests.

## Phase 4 — playbook library and evaluation

- Build a versioned behavior-tree playbook registry with static validation and execution traces.
- Use MAP-Elites/quality-diversity offline to populate diverse, high-performing ISR/coverage/relay playbooks across environment descriptors.
- Keep human-control and safety policy as hard feasibility constraints, never fitness rewards.
- Add operator comparison tools, scenario suites, coverage/connectivity metrics, and reproducible seeds.
- Promote a wire v2 only for proven contract needs; provide a v1 compatibility adapter.

## Verification strategy

- Unit tests: geodesy/kinematics, behavior-tree status propagation, analytic link thresholds, transition detection, scenario validation, CoT XML escaping and shape.
- Contract tests: serialize representative `hello` and `state` envelopes and assert enum spellings, units, stable link IDs, and CZML longitude/latitude ordering.
- Integration test: advance the ISR scenario until a known link flap and validate REST/WebSocket-equivalent snapshots.
- Frontend checks: TypeScript compile, production build, reconnect behavior, complete-frame reconciliation, 2D/3D toggle, and all four transport styles.
- Manual demo: launch the server and Vite client, inspect the live Cesium scene, `/api/v1/snapshot`, and CoT output.

## Design forks recorded for later phases

- SigForge transport: prefer gRPC for control and its WebSocket for observation unless measured consistency requires one channel.
- TAK ingest: keep it isolated from mission execution until identity, authorization, replay protection, and human-approval semantics are decided.
- Photorealistic terrain: optional external keys must enhance the demo, never become a launch dependency.
- Traffic: Phase 1 synthetic aggregates prove the contract; packet-derived Ditto telemetry belongs in the real backend.

## Research basis

Behavior trees are selected for modular, reactive, inspectable execution ([Colledanchise and Ögren](https://arxiv.org/abs/1709.00084)). The future playbook archive follows MAP-Elites' quality-diversity framing ([Mouret and Clune](https://arxiv.org/abs/1504.04909)). Network fidelity progresses from the built-in analytic model to SigForge's CORE/EMANE-compatible tier ([CORE/EMANE architecture](https://coreemu.github.io/core/emane.html), [EMANE guide](https://emane.io/introduction)) and Ditto peer-link observations. Visualization follows Cesium/CZML ([Cesium CZML API](https://cesium.com/learn/cesiumjs/ref-doc/CzmlDataSource.html)); C2 output follows the indexed [Cursor-on-Target standard](https://quicksearch.dla.mil/qsDocDetails.aspx?ident_number=284928).
