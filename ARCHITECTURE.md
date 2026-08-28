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
    +----------> versioned state frames ----------> Axum REST/WebSocket
                       |                                  |
                       +--> CoT/TAK PLI + track sink      +--> CesiumJS 2D/3D
```

### Entity and agent layer

An `Entity` has a stable string ID, display name, kind (`drone`, `person`, `ground_vehicle`, `ground_station`, or `sensor`), domain (`ground`, `air`, `maritime`, or `space`), WGS84 position, velocity, heading, configured radios, and mission state. A fixed-step scheduler owns simulation time. Every tick is ordered: tick behavior tree, integrate kinematics, evaluate network links, record link transitions and synthetic Ditto traffic, emit CoT, publish a state frame. Scenario seed and tick size make analytic runs reproducible.

### Behavior-tree and mission layer

Behavior trees use `Sequence`, `Fallback`, and `Parallel` composites with explicit condition and action leaves. Every tick returns `running`, `success`, or `failure`; the active leaf and status are included in entity state for inspection. Phase 1 playbooks are:

- `area_search`: traverse a declared waypoint coverage route.
- `persistent_surveillance`: transit to and loiter around an ISR point.
- `comms_relay`: move toward the midpoint of disconnected peers and hold when connectivity is restored.

The tree is an execution structure, not an optimizer. Later playbook selection can use MAP-Elites, but candidate generation cannot relax safety constraints or human authority.

### Network and propagation seams

`NetworkBackend` owns node registration and returns a complete, deterministic set of pairwise `LinkState` values for a tick. The backend is responsible for link type, up/down state, normalized quality, distance, latency, loss, and capacity. It does not own entity motion or wall-clock scheduling.

The built-in analytic backend combines radio compatibility with a `PropagationModel`. Its outdoor model uses geodesic/slant distance, configured range, and a monotonic path-loss-inspired quality curve. The separate propagation trait keeps terrain/LOS, indoor body blocking, urban ray models, and ns-3 integration replaceable without changing missions or the API.

The `SigForgeBackend` Phase 1 stub implements the same trait but returns a clear unavailable error. Phase 2 will register NEMs through SigForge REST/gRPC, publish position events, and consume its link matrix/WebSocket. IDs remain autonomy-sim IDs at this boundary; adapter-owned maps translate them to NEM IDs.

### C2/TAK bridge

The outbound bridge renders Cursor-on-Target XML `event` records with `point` and `detail/contact` for platform PLI/track updates. Sinks are pluggable. Phase 1 includes an append-only file sink for a dependency-free demo and UDP/TCP sinks for a configured TAK endpoint. Incoming tasking is out of scope and must later pass authentication, authorization, validation, and explicit human-control policy before becoming mission input.

### Visualization

The frontend is a Vite/TypeScript CesiumJS client. The default 2D top-down scene has a public imagery fallback and needs no token. The 3D mode uses the globe and can optionally add Google Photorealistic 3D Tiles when `VITE_GOOGLE_MAPS_API_KEY` is present. Entity iconography derives from kind/domain. Active Ditto peer links are rendered and removed on state changes; color identifies `mesh`, `cellular`, `satcom`, or `ble`, while opacity/width communicate quality and traffic.

## Stable Phase 1 wire contract

The public interface is `autonomy-sim/v1`. Additive fields may appear without a version bump. Existing field meaning, enum values, and units will not change within v1. Clients must ignore unknown fields. Removing or redefining fields requires `autonomy-sim/v2`.

### Transport

- WebSocket: `GET /api/v1/stream`. The server sends one `hello`, immediately sends the latest `state`, then sends a `state` after every simulation tick. Text frames contain one UTF-8 JSON object. Client messages are not required in Phase 1; unsupported messages are ignored.
- REST snapshot: `GET /api/v1/snapshot` returns the exact latest `state` envelope.
- Health: `GET /healthz` returns `{"status":"ok"}`.
- Ordering: `sequence` increases by one per state frame in a server process. A reconnect starts with a new `hello`; clients replace their complete local view with every `state.payload` and may discard sequence numbers less than or equal to the last applied value.
- Time: `sim_time_s` is simulation seconds from scenario start, not wall time. CoT carries UTC wall time because TAK consumers require it.

### Envelope union

```json
{
  "schema": "autonomy-sim/v1",
  "message_type": "hello",
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
  "sequence": 42,
  "sim_time_s": 8.4,
  "payload": {
    "entities": [],
    "links": [],
    "link_events": [],
    "traffic": [],
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
  "kind": "drone",
  "domain": "air",
  "position": { "lat_deg": 34.0501, "lon_deg": -117.2502, "alt_m": 180.0 },
  "kinematics": { "speed_mps": 14.0, "heading_deg": 91.5, "vertical_speed_mps": 0.0 },
  "mission": {
    "playbook": "persistent_surveillance",
    "active_node": "loiter",
    "status": "running"
  }
}
```

`kind` is one of `drone`, `person`, `ground_vehicle`, `ground_station`, `sensor`. `domain` is one of `ground`, `air`, `maritime`, `space`. Mission `status` is `running`, `success`, or `failure`.

### Link state and transitions

Every configured compatible radio pair is present in `links`, including down links. Endpoint IDs are lexically sorted. `id` is stable and formatted `link/<link_type>/<a>/<b>`. Quality and loss are bounded `[0,1]`; capacity and traffic use bits per second.

```json
{
  "id": "link/mesh/ground-c2/uav-alpha",
  "source": "ground-c2",
  "target": "uav-alpha",
  "link_type": "mesh",
  "state": "up",
  "quality": 0.78,
  "distance_m": 812.4,
  "latency_ms": 18.2,
  "packet_loss": 0.04,
  "capacity_bps": 2800000
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

Traffic is a per-up-link aggregate for the current tick. In Phase 1 it is deterministic synthetic Ditto replication load, not packet capture.

```json
{
  "link_id": "link/mesh/ground-c2/uav-alpha",
  "tx_bps": 186000,
  "rx_bps": 171000,
  "messages_per_s": 24.0,
  "queue_depth": 2
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
  "properties": { "entity_id": "uav-alpha", "kind": "drone", "domain": "air" }
}
```

## Architectural references

- Colledanchise and Ögren, [Behavior Trees in Robotics and AI](https://arxiv.org/abs/1709.00084), motivates modular, reactive, analyzable task switching.
- Mouret and Clune, [Illuminating Search Spaces by Mapping Elites](https://arxiv.org/abs/1504.04909), is the basis for a future diverse playbook archive rather than a single opaque policy.
- [CORE with EMANE](https://coreemu.github.io/core/emane.html) and the [EMANE guide](https://emane.io/introduction) describe the real network-emulation tier that SigForge replaces/adapts.
- Ditto's local repository demonstrates transport-abstracted peer sync and direction-aware link properties; autonomy-sim visualizes that connectivity without embedding Ditto itself.
- Cesium's [CzmlDataSource](https://cesium.com/learn/cesiumjs/ref-doc/CzmlDataSource.html) is the compatibility target for the CZML projection.
- The DoD index for the [Cursor-on-Target Message Standard](https://quicksearch.dla.mil/qsDocDetails.aspx?ident_number=284928) defines the C2 interchange family used by the TAK bridge.

