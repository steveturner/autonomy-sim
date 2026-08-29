# autonomy-sim

Phase 1 prototype of a defensive autonomy, mission, Ditto-first C2/TAK, and visualization layer for uncrewed-system simulation. It runs locally with a deterministic analytic network model; CORE, EMANE, SigForge, a native `dittoffi` runtime, Cesium ion, and a TAK server are not required.

The included ISR scenario drives two drones, two people, a relay rover, and a C2 gateway. Every platform is a behavioral Ditto peer. C2 tasking, PLI, tracks, and telemetry are replicated documents that persist across partitions and converge peer-to-peer when links return. As platforms move, those Ditto peer links appear and drop across mesh, cellular, satcom, and BLE carriers.

This project is for simulation, ISR, communications, and coordination only. It contains no lethal-engagement logic. Human authorization is enforced as a hard behavior-tree condition.

## Quick start

Prerequisites: Rust with Edition 2024 support, Node.js 20 or newer, and npm.

Install the frontend packages once:

```bash
make setup
```

Start the complete demo with one command:

```bash
make live
```

Run the Paradise wildfire swarm with the analytic backend (no CORE/EMANE required):

```bash
make wildfire
```

Use `make wildfire-live` to start both the wildfire simulator and frontend. Both targets accept `HOST=0.0.0.0` for the existing trusted-LAN profile.

This runs the simulator/WebSocket service and Vite together. To run them in separate terminals instead, start the simulator in terminal 1:

```bash
make demo
```

Start Cesium in terminal 2:

```bash
make frontend
```

Open [http://127.0.0.1:5173](http://127.0.0.1:5173). The default view is token-free 2D top-down. Select **3D PHOTOREAL** for the 3D globe. To add Google Photorealistic 3D Tiles, copy `frontend/.env.example` to `frontend/.env`, set `VITE_GOOGLE_MAPS_API_KEY`, and restart Vite; without a key, 3D falls back to the Cesium globe.

## Trusted-LAN remote access

Loopback remains the default. To opt in to remote access, bind both services to every interface:

```bash
make live HOST=0.0.0.0
```

From another machine, open `http://<sim-host-LAN-IP>:5173`. Allow inbound TCP ports 5173 and 9000 in the host firewall if necessary. The frontend derives its default API/WebSocket host from `window.location.hostname`, so a remote browser connects back to `<sim-host-LAN-IP>:9000` rather than its own loopback address.

For separate terminals, use:

```bash
make demo HOST=0.0.0.0
make frontend HOST=0.0.0.0
```

For a split-host or nonstandard API deployment, set `VITE_API_HOST=hostname:port`, `VITE_API_URL=http://hostname:port`, or the exact `VITE_WS_URL`. See `frontend/.env.example`.

> **SECURITY:** The Phase 1 HTTP API and WebSocket have no authentication, authorization, or TLS. Binding to `0.0.0.0` exposes simulation state to anyone who can reach those ports. Use remote mode only on a trusted LAN or isolated demo network. Do not expose it directly to the public internet.

The backend writes one standalone Cursor-on-Target event per line to `output/isr-demo.cot`. Follow it with:

```bash
tail -f output/isr-demo.cot
```

## What the demo shows

- `uav-alpha` follows a coverage route across the gateway's mesh horizon, creating an observable up/down/up link transition while satcom remains available.
- `uav-bravo` executes a persistent ISR loiter using a parallel behavior subtree.
- `relay-one` uses a fallback subtree: it holds while the direct C2-to-Alpha mesh is up and moves toward the peers' midpoint when that link drops.
- `scout-one` patrols beyond BLE range of `scout-two` while cellular connectivity persists.
- Link color identifies transport; opacity indicates quality; line width indicates synthetic Ditto replication traffic. The right rail records link transitions.
- The HUD exposes CRDT document and peer-convergence state; documents remain locally available during DDIL partitions and propagate after reconnection.
- The 2D/3D switch preserves live platform state and tracks.

## API and wire contract

- `GET http://127.0.0.1:9000/healthz` — process health.
- `GET http://127.0.0.1:9000/api/v1/snapshot` — latest complete state envelope.
- `GET http://127.0.0.1:9000/api/v1/scenarios` — registered chooseable scenarios.
- `ws://127.0.0.1:9000/api/v1/stream` — `hello`, immediate current `state`, then one complete `state` per simulation tick.

Select the single active scenario through either snapshot or WebSocket query parameters:

```bash
curl --silent 'http://127.0.0.1:9000/api/v1/snapshot?scenario=wildfire-paradise' | jq
# WebSocket: ws://127.0.0.1:9000/api/v1/stream?scenario=wildfire-paradise
```

Selection replaces the process-wide active simulation for all clients. The optional explicit switch endpoint is `POST /api/v1/scenario` with `{"id":"wildfire-paradise"}`. `GET /api/v1/scenarios` reports the current `active` ID, and every hello/state envelope carries the same stable scenario slug.

The stable `autonomy-sim/v1` message schema, enum values, units, ordering, and CZML projection are defined in [ARCHITECTURE.md](ARCHITECTURE.md). State frames make entities, Ditto peer identities, document replicas, replication events, current carrier links, traffic aggregates, and CZML explicit.

Inspect a snapshot:

```bash
curl --silent http://127.0.0.1:9000/api/v1/snapshot | jq
```

## Scenario conventions

Scenarios are TOML and follow SigForge's top-level conventions:

```toml
[scenario]
name = "example"
seed = 42
realtime = true

[simulation]
tick_hz = 5.0
network_backend = "analytic"

[[nodes]]
id = "uav-one"
name = "UAV One"
kind = "uas"
domain = "air"
position = { lat_deg = 34.0, lon_deg = -117.0, alt_m = 200.0 }

[[nodes.radios]]
link_type = "mesh"
range_m = 1000.0
capacity_bps = 4000000
base_latency_ms = 8.0

[nodes.mission]
playbook = "area_search"
speed_mps = 20.0
human_authorized = true
waypoints = [
  { lat_deg = 34.0, lon_deg = -117.0, alt_m = 200.0 },
  { lat_deg = 34.01, lon_deg = -117.0, alt_m = 200.0 },
]
```

Registered scenario names can be passed without a path, for example `cargo run -- --scenario wildfire-paradise`. Kinds are `uas`, `air_tanker`, `rotary`, `person`, `ground_vehicle`, `base`, `fire`, `waypoint`, `threat_uas`, `radar_sensor`, `ew_jammer`, `interceptor`, `gun_system`, and `protected_site`; domains are `ground`, `air`, `maritime`, and `space`. Radios use `mesh`, `cellular`, `satcom`, or `ble`. Standard playbooks are `hold`, `area_search`, `persistent_surveillance`, and `comms_relay`; the `wildfire` builder adds `firefighting`. Invalid scenarios fail before the server binds.

Run another scenario or override the API address directly:

```bash
cargo run -- --scenario scenarios/thin-slice.toml --bind 127.0.0.1:9100
```

When the backend port changes, set `VITE_API_HOST=127.0.0.1:9100` for Vite. `VITE_API_URL=http://127.0.0.1:9100` and the lower-level `VITE_WS_URL=ws://127.0.0.1:9100/api/v1/stream` are also supported.

## CoT/TAK output

The `[cot]` section accepts four sink modes:

```toml
[cot]
sink = "file" # disabled | file | udp | tcp
path = "output/events.cot"
endpoint = "239.2.3.1:6969" # required for udp/tcp, unused for file
interval_s = 1.0
stale_after_s = 10
```

The CoT component is a gateway at the ground-station Ditto peer, not a node-to-node transport. Phase 1 maps friendly PLI/track documents that have reached that gateway into CoT events with contact and course/speed detail. UDP and TCP send newline-delimited standalone events. The reverse TAK-to-Ditto mapping is intentionally deferred until authentication, authorization, replay protection, validation, and explicit human approval are designed.

## Development and verification

```bash
make check
```

This runs Rust formatting, Clippy with warnings denied, all Rust unit/integration tests, and the frontend production build. The ISR integration test advances 220 simulation seconds without waiting on wall time and asserts that all four transports occur and multiple link drops/restorations are emitted.

Useful individual commands:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix frontend run build
```

## Phase 1 status

Implemented:

- WGS84 entity/kinematics model across ground, air, maritime, and space domains.
- Fixed-step scheduler and validated TOML scenarios.
- Auditable sequence, fallback, and parallel behavior trees with ISR/coverage/relay playbooks.
- `NetworkBackend` and `PropagationModel` traits, outdoor analytic networking, four carrier types, transition events, quality/loss/latency/capacity, and deterministic Ditto replication traffic.
- A behavioral Ditto model with one peer per entity, `c2.tasking`, `c2.pli`, `c2.tracks`, and `telemetry.platform` collections, bounded per-link document propagation, offline persistence, and eventual convergence after reconnect.
- Axum REST snapshot and Tokio WebSocket state streamer using the documented v1 contract and CZML-compatible packets.
- A Ditto-to-CoT gateway with PLI/track XML and file, UDP, and TCP sinks.
- CesiumJS 2D and 3D modes, platform tracks, live link reconciliation, transport styling, traffic indication, and optional Google Photorealistic 3D Tiles.

Stubbed or deferred:

- The `SigForgeBackend` is an explicit trait implementation that fails closed with integration guidance; real SigForge API/WebSocket/PHY integration is Phase 2.
- Native Ditto small-peer/`dittoffi` nodes over SigForge/CORE-EMANE are Phase 2; Phase 1 models CRDT convergence behavior rather than running the production Ditto engine.
- TAK-to-Ditto ingest and production TAK Server certificate handling are Phase 2.
- Analytic traffic is a deterministic document-operation aggregate, not a Ditto packet capture.
- Terrain/LOS, indoor body blocking, urban propagation, and ns-3 are later `PropagationModel` implementations.
- MAP-Elites is a Phase 4 offline playbook-library mechanism, not part of runtime mission execution.

See [PLAN.md](PLAN.md) for the phased roadmap.

## License

Apache-2.0.
