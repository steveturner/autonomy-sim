# autonomy-sim architecture

## Purpose and safety boundary

`autonomy-sim` is a deterministic simulation and visualization engine for defensive uncrewed-system coordination, ISR, communications relay, and C2 interoperability. Phase 1 deliberately contains no target prosecution, weapon model, engagement objective, or autonomous use-of-force decision. Human control is a hard mission constraint: autonomy may navigate, search, loiter, and extend communications, but it cannot turn a sensed object into an engagement action.

## Layered design

```text
scenario TOML
    |
    v
entity + fixed-step kinematics <--- behavior tree / mission playbooks
    |                                  area search, ISR loiter, comms relay
    v
NetworkBackend <--- PropagationModel
    |                free-space/range now; indoor, urban, ns-3 later
    +--- analytic mesh (built in, zero external dependencies)
    +--- SigForge adapter (Phase 1 interface stub; real API in Phase 2)
    |
    v
Ditto peer + document replication model
    |       C2 tasking, PLI, tracks, telemetry; eventual convergence in DDIL
    +----------> versioned state frames ----------> Axum REST/WebSocket
    |                                                     |
    +--> CoT/TAK gateway <--> TAKServer/WebTAK             +--> CesiumJS 2D/3D
```

### Entity and agent layer

An `Entity` has a stable string ID, display name, kind, affiliation, MIL-STD-2525C SIDC, domain (`ground`, `air`, `maritime`, or `space`), WGS84 position, velocity, heading, configured radios, mission role, and mission state. Active platform/site entities are Ditto peers with stable ID `ditto/<entity-id>`; environmental `fire` and navigation `waypoint` entities are document subjects rather than radio peers. There is no separate centralized messaging node. A fixed-step scheduler owns simulation time. Every tick is ordered: tick behavior trees or the wildfire mission FSM, integrate kinematics, update the fire model, evaluate peer links, update local Ditto documents, replicate documents over available links, update convergence, export gateway-visible documents to CoT, and publish a state frame. Scenario seed and tick size make analytic runs reproducible.

### Behavior-tree and mission layer

Behavior trees use `Sequence`, `Fallback`, and `Parallel` composites with explicit condition and action leaves. Every tick returns `running`, `success`, or `failure`; the active leaf and status are included in entity state for inspection. Phase 1 playbooks are:

- `area_search`: traverse a declared waypoint coverage route.
- `persistent_surveillance`: transit to and loiter around an ISR point.
- `comms_relay`: move toward the midpoint of disconnected peers and hold when connectivity is restored.
- `firefighting`: run the Rust tanker FSM `holding → enroute_to_fire → on_station → dropping → egress → enroute_to_base → reloading → holding`. Flocking combines separation, alignment, cohesion, and goal steering under configured neighbor, speed, and turn-rate limits. The Grass Valley base limits concurrent reload slots; a drop reduces the assigned Paradise fire cell's intensity.

The tree is an execution structure, not an optimizer. Later playbook selection can use MAP-Elites, but candidate generation cannot relax safety constraints or human authority.

### Network and propagation seams

`NetworkBackend` owns node registration and returns a complete, deterministic set of pairwise `LinkState` values for a tick. These are **Ditto peer links riding an emulated carrier**, not an ad-hoc application messaging channel: `link_type` identifies the carrier (`mesh`, `cellular`, `satcom`, or `ble`), while the endpoints identify Ditto peers. A link transition means two peers gained or lost a replication path. The backend is responsible for up/down state, normalized quality, distance, latency, loss, and capacity. It does not own entity motion, Ditto documents, or wall-clock scheduling.

The built-in analytic backend combines radio compatibility with a `PropagationModel`. Its outdoor model uses geodesic/slant distance, configured range, and a monotonic path-loss-inspired quality curve. The separate propagation trait keeps terrain/LOS, indoor body blocking, urban ray models, and ns-3 integration replaceable without changing missions or the API.

The `SigForgeBackend` Phase 1 stub implements the same trait but returns a clear unavailable error. Phase 2 will register NEMs through SigForge REST/gRPC, publish position events, and consume its link matrix/WebSocket. IDs remain autonomy-sim IDs at this boundary; adapter-owned maps translate them to NEM IDs.

### Ditto peer and document layer

Ditto is the primary inter-node communication model. C2 tasking, PLI, tracks, and platform telemetry live in the collections `c2.tasking`, `c2.pli`, `c2.tracks`, and `telemetry.platform`. Wildfire coordination adds `mission.fire_cells`, `mission.base_queue`, and `mission.drop_assignments`; document `value` carries the fire cell, slot/queue, and tanker claim/completion state described below. The AAB supervisor authors fire-cell and base-queue documents, while each tanker authors its drop-assignment document. Each peer updates its locally authored documents, discovers reachable peers from current `NetworkBackend` links, and exchanges newer document revisions within link quality/capacity budgets. When two peers have several carriers up, Phase 1 moves document revisions over the highest-quality carrier and accounts for discovery/keepalive overhead on the others. Replicas remain available while disconnected and eventually converge when a path returns; no central broker is required.

Phase 1 models CRDT behavior at the document/revision level: peer discovery, replica watermarks, bounded per-link propagation, pending-document counts, and convergence state. It intentionally does not embed `dittoffi`. Phase 2+ will replace this behavioral model with real Ditto small peers whose transports run through SigForge/CORE-EMANE, following the `ditto-barrage-*` scale-test pattern. The wire contract remains the observation boundary for both implementations.

### C2/TAK gateway

TAK is an edge gateway, not the platform-to-platform transport. The gateway watches the Ditto document space available at its local peer and maps replicated `c2.pli`/`c2.tracks` documents into Cursor-on-Target XML `event` records for TAKServer/WebTAK. Phase 1 includes append-only file, UDP, and TCP CoT sinks and exports only records whose latest document revision has reached the gateway replica. Phase 2 adds the reverse mapping from authenticated TAK tasking into `c2.tasking` documents; authorization, replay protection, validation, and explicit human approval remain mandatory before a task document can affect a mission.

### Visualization

The frontend is a Vite/TypeScript CesiumJS client. The default 2D top-down scene has a public imagery fallback and needs no token. The 3D mode uses the globe and can optionally add Google Photorealistic 3D Tiles when `VITE_GOOGLE_MAPS_API_KEY` is present. Entity iconography derives from kind/domain. Active Ditto peer links are rendered and removed on state changes; color identifies `mesh`, `cellular`, `satcom`, or `ble`, while opacity/width communicate quality and traffic.

## Stable Phase 1 wire contract

The public interface is `autonomy-sim/v1`. Additive fields may appear without a version bump. Existing field meaning, enum values, and units will not change within v1. Clients must ignore unknown fields. Removing or redefining fields requires `autonomy-sim/v2`.

### Transport

- WebSocket: `GET /api/v1/stream`. The server sends one `hello`, immediately sends the latest `state`, then sends a `state` after every simulation tick. Text frames contain one UTF-8 JSON object. Client messages are not required in Phase 1; unsupported messages are ignored.
- REST snapshot: `GET /api/v1/snapshot` returns the exact latest `state` envelope.
- Scenario registry: `GET /api/v1/scenarios` returns the exact selection contract below. `id` is the stable slug used by the CLI, REST API, WebSocket API, and envelopes; `name` is display text. `entity_count` counts configured platform/site nodes and excludes generated environmental fire-cell entities.
- Selection query: `GET /api/v1/snapshot?scenario=<id>` and `GET /api/v1/stream?scenario=<id>` select that registered scenario before returning/upgrading. Omitting `scenario` uses the current active/default scenario.
- Explicit selection: `POST /api/v1/scenario` with `{"id":"<id>"}` returns `{"active":"<id>"}`.
- Health: `GET /healthz` returns `{"status":"ok"}`.
- Single-active prototype: the process runs exactly one simulation. A query or POST selecting another scenario replaces the running simulation for every client, publishes its initial state immediately, and resets `sequence` and `sim_time_s` to zero. Clients use the envelope `scenario` field to detect the change before applying per-scenario sequence ordering.
- Ordering: within one active scenario run, `sequence` increases by one per state frame. A reconnect starts with a new `hello`; clients replace their complete local view with every `state.payload` and may discard sequence numbers less than or equal to the last applied value for the same `scenario`.
- Time: `sim_time_s` is simulation seconds from scenario start, not wall time. CoT carries UTC wall time because TAK consumers require it.

```json
{
  "active": "isr-relay-demo",
  "scenarios": [
    {
      "id": "isr-relay-demo",
      "name": "ISR Relay Demo",
      "description": "Two ISR drones, a mobile relay, two people, and a C2 node with flapping Ditto links",
      "entity_count": 6,
      "default": true
    },
    {
      "id": "wildfire-paradise",
      "name": "Wildfire - Paradise",
      "description": "Twelve UAS air tankers coordinate fire-suppression drops between Grass Valley AAB and Paradise",
      "entity_count": 14,
      "default": false
    }
  ]
}
```

### Envelope union

```json
{
  "schema": "autonomy-sim/v1",
  "message_type": "hello",
  "scenario": "isr-relay-demo",
  "sequence": 0,
  "sim_time_s": 0.0,
  "payload": {
    "scenario": "isr-relay-demo",
    "tick_hz": 5.0,
    "server": "autonomy-sim/0.1.0"
  }
}
```

```json
{
  "schema": "autonomy-sim/v1",
  "message_type": "state",
  "scenario": "isr-relay-demo",
  "sequence": 42,
  "sim_time_s": 8.4,
  "payload": {
    "entities": [],
    "links": [],
    "link_events": [],
    "traffic": [],
    "ditto_peers": [],
    "ditto_documents": [],
    "ditto_replication_events": [],
    "fire_cells": [],
    "base": null,
    "czml": []
  }
}
```

### Entity state

All angles are degrees, altitude is WGS84 meters, and speed is meters per second.

```json
{
  "id": "uav-alpha",
  "name": "UAV Alpha",
  "kind": "uas",
  "affiliation": "friendly",
  "sidc": "SFAPMFQ--------",
  "icon_hint": "fixed_wing_uas",
  "domain": "air",
  "position": { "lat_deg": 34.0501, "lon_deg": -117.2502, "alt_m": 180.0 },
  "kinematics": { "speed_mps": 14.0, "heading_deg": 91.5, "vertical_speed_mps": 0.0 },
  "mission_role": "scout",
  "mission_state": "coverage_waypoint_2",
  "heading_deg": 91.5,
  "mission": {
    "playbook": "persistent_surveillance",
    "active_node": "loiter",
    "status": "running"
  }
}
```

The canonical entity fields and enum spellings are:

- `kind`: `uas`, `air_tanker`, `rotary`, `person`, `ground_vehicle`, `base`, `fire`, `waypoint`, `threat_uas`, `radar_sensor`, `ew_jammer`, `interceptor`, `gun_system`, or `protected_site`.
- `affiliation`: `friendly`, `hostile`, `neutral`, or `unknown`. Position 2 of `sidc` encodes the same affiliation (`F`, `H`, `N`, or `U`), producing the standard blue friendly frame and red hostile frame in a 2525C renderer.
- `sidc`: exactly 15 ASCII characters, using the MIL-STD-2525C SIDC layout. Important mappings include fixed-wing UAV/air tanker/threat UAV `MFQ---`, rotary wing `MH----`, interceptor `MFFI--`, radar `ESR---`, utility ground vehicle `EVU---`, military base `IB----` with the installation modifier, and Emergency Management wild-fire incident `CH----`.
- `icon_hint`: stable renderer fallback string; the SIDC remains authoritative.
- `mission_role`: free string such as `tanker`, `leadplane`, `scout`, `relay`, or `air_attack_base`.
- `mission_state`: current leaf/FSM string. Wildfire tanker values are exactly `holding`, `enroute_to_fire`, `on_station`, `dropping`, `egress`, `enroute_to_base`, or `reloading`.
- `heading_deg`: canonical top-level heading in degrees clockwise from true north. It mirrors `kinematics.heading_deg`; both remain in v1 for compatibility.
- `retardant_pct`: optional number in `[0,100]`, present on air tankers and absent on other kinds.
- `intensity`: optional number in `[0,100]`, present on `fire` entities and absent on other kinds.
- `domain`: `ground`, `air`, `maritime`, or `space`. Nested mission `status` remains `running`, `success`, or `failure`.

### Wildfire fire cells and base

`payload.fire_cells` and `payload.base` are canonical mission-level projections. Standard/non-wildfire scenarios send `fire_cells: []` and `base: null`. Fire cells also appear in `entities` with `kind: "fire"`, the same `id`, position, and `intensity`; the mission projection carries assignment status without forcing clients to join Ditto documents.

```json
{
  "fire_cells": [{
    "id": "paradise-fire-01",
    "position": { "lat_deg": 39.7596, "lon_deg": -121.6219, "alt_m": 532.0 },
    "intensity": 62.4,
    "assigned_tanker": "tanker-03",
    "status": "dropping"
  }],
  "base": {
    "id": "grass-valley-aab",
    "name": "Grass Valley Air Attack Base",
    "position": { "lat_deg": 39.2244, "lon_deg": -121.0030, "alt_m": 1017.0 },
    "reload_slots": 3,
    "occupied_slots": ["tanker-01", "tanker-04"],
    "queue": ["tanker-07"]
  }
}
```

Fire-cell `status` is `available`, `assigned`, `dropping`, or `contained`. `assigned_tanker` is a tanker entity ID or `null`. `occupied_slots` and `queue` contain tanker entity IDs; queue order is FIFO.

### Link state and transitions

Every configured compatible radio pair is present in `links`, including down links. Endpoint IDs are lexically sorted. `id` is stable and formatted `link/<link_type>/<a>/<b>`. The corresponding peer IDs are explicit; each record is a Ditto replication path over the named carrier. Quality and loss are bounded `[0,1]`; capacity and traffic use bits per second.

```json
{
  "id": "link/mesh/ground-c2/uav-alpha",
  "source": "ground-c2",
  "target": "uav-alpha",
  "source_peer_id": "ditto/ground-c2",
  "target_peer_id": "ditto/uav-alpha",
  "link_type": "mesh",
  "state": "up",
  "quality": 0.78,
  "distance_m": 812.4,
  "latency_ms": 18.2,
  "packet_loss": 0.04,
  "capacity_bps": 2800000
}
```

### Ditto peer state

`ditto_peers` contains one record per active platform/site entity; environmental `fire` and navigation `waypoint` entities do not create peers. `connected_peer_ids` is derived from all current up links, regardless of carrier. `document_count` counts locally present latest-or-stale replicas; `pending_documents` counts known global revisions not yet at that peer. `converged` is true when the peer holds the latest revision of every document currently known to the simulation.

```json
{
  "peer_id": "ditto/uav-alpha",
  "entity_id": "uav-alpha",
  "connected_peer_ids": ["ditto/relay-one"],
  "document_count": 14,
  "pending_documents": 5,
  "converged": false,
  "collection_versions": {
    "c2.pli": 19,
    "c2.tasking": 1,
    "c2.tracks": 19,
    "telemetry.platform": 19
  }
}
```

### Ditto document state and replication events

`ditto_documents` describes the latest known logical revision and which peers have that revision. Phase 1 uses single-author documents and scalar revisions as a behavioral stand-in for real Ditto CRDT metadata; real version vectors/conflict semantics arrive with `dittoffi`. `converged` means all scenario peers hold the latest revision.

```json
{
  "collection": "c2.pli",
  "document_id": "pli/uav-alpha",
  "author_peer_id": "ditto/uav-alpha",
  "revision": 9,
  "updated_at_s": 8.0,
  "value": { "entity_id": "uav-alpha", "heading_deg": 91.5 },
  "replicated_to": ["ditto/relay-one", "ditto/uav-alpha"],
  "converged": false
}
```

`ditto_replication_events` contains document transfers completed during the current tick. It is ephemeral; durable replica state is in `ditto_documents` and `ditto_peers`.

```json
{
  "collection": "c2.tasking",
  "document_id": "task/isr-relay-demo",
  "revision": 1,
  "from_peer_id": "ditto/ground-c2",
  "to_peer_id": "ditto/relay-one",
  "link_id": "link/mesh/ground-c2/relay-one",
  "replicated_at_s": 8.4
}
```

`link_type` is `mesh`, `cellular`, `satcom`, or `ble`; `state` is `up` or `down`. `link_events` includes only transitions since the previous frame:

```json
{
  "link_id": "link/mesh/ground-c2/uav-alpha",
  "source": "ground-c2",
  "target": "uav-alpha",
  "link_type": "mesh",
  "state": "down",
  "changed_at_s": 8.4
}
```

### Traffic state

Traffic is a per-up-link aggregate for the current tick. In Phase 1 it is computed from completed document replication operations plus deterministic peer-discovery/keepalive overhead; it is not packet capture. `pending_documents` counts documents whose revision differs between that link's endpoint replicas after the current tick.

```json
{
  "link_id": "link/mesh/ground-c2/uav-alpha",
  "tx_bps": 186000,
  "rx_bps": 171000,
  "messages_per_s": 24.0,
  "queue_depth": 2,
  "document_ops_per_s": 5.0,
  "pending_documents": 2
}
```

### CZML compatibility subset

`payload.czml` is an array of Cesium CZML packets. It is a convenience projection of the canonical entity/link arrays, modeled after SigForge. Entity packet IDs are `entity/<entity-id>` and carry `position.cartographicDegrees` in `[longitude, latitude, altitude]` order plus point/label/properties. Up-link packet IDs equal the canonical link ID and carry a `polyline.positions.cartographicDegrees` array. Down links are omitted. Because a state envelope is a complete snapshot, clients remove visual links absent from the current CZML set; standard incremental CZML clients may instead use `links` as the authority.

```json
{
  "id": "entity/uav-alpha",
  "name": "UAV Alpha",
  "position": { "cartographicDegrees": [-117.2502, 34.0501, 180.0] },
  "point": { "pixelSize": 12, "color": { "rgba": [65, 191, 255, 255] } },
  "label": { "text": "UAV Alpha" },
  "properties": {
    "entity_id": "uav-alpha",
    "ditto_peer_id": "ditto/uav-alpha",
    "kind": "uas",
    "affiliation": "friendly",
    "sidc": "SFAPMFQ--------",
    "icon_hint": "fixed_wing_uas",
    "domain": "air",
    "mission_role": "scout",
    "mission_state": "coverage_waypoint_2",
    "heading_deg": 91.5,
    "retardant_pct": null,
    "intensity": null
  }
}
```

For environmental `fire` and navigation `waypoint` packets, `ditto_peer_id` is `null`; their state is coordinated as documents rather than modeled as a radio peer.

## Architectural references

- Colledanchise and Ögren, [Behavior Trees in Robotics and AI](https://arxiv.org/abs/1709.00084), motivates modular, reactive, analyzable task switching.
- Mouret and Clune, [Illuminating Search Spaces by Mapping Elites](https://arxiv.org/abs/1504.04909), is the basis for a future diverse playbook archive rather than a single opaque policy.
- [CORE with EMANE](https://coreemu.github.io/core/emane.html) and the [EMANE guide](https://emane.io/introduction) describe the real network-emulation tier that SigForge replaces/adapts.
- Ditto's local repository defines the transport-abstracted peer-sync model used here; Phase 1 simulates its discovery, document replication, DDIL persistence, and eventual convergence, while Phase 2+ embeds real small peers over SigForge/CORE-EMANE.
- Cesium's [CzmlDataSource](https://cesium.com/learn/cesiumjs/ref-doc/CzmlDataSource.html) is the compatibility target for the CZML projection.
- The DoD index for the [Cursor-on-Target Message Standard](https://quicksearch.dla.mil/qsDocDetails.aspx?ident_number=284928) defines the C2 interchange family used by the TAK bridge.
