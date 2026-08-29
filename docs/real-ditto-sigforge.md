# Real Ditto and SigForge adapters

The analytic `NetworkBackend` and behavioral Ditto model remain autonomy-sim's dependency-free defaults. Two additive workspace crates provide the production seams:

- `autonomy-sim-ditto-real` owns one actual Ditto small peer and persistent store per scenario entity.
- `autonomy-sim-net-sigforge` implements `NetworkBackend` against SigForge's session REST API.

## Run autonomy-sim with real Ditto

The native transport is feature-gated because building Ditto is substantially heavier than the default simulator and requires a licensed Ditto source checkout. The current bridge is intended for Linux/CORE-EMANE environments.

Set the checkout and an offline-playground license in your shell. Do not commit the license or put it in a scenario:

```bash
export DITTO_SOURCE_DIR=/absolute/path/to/ditto
export DITTO_LICENSE='<offline Ditto license>'
```

Build `libdittoffi`, then run the native integration cases. The Make target treats the Ditto checkout as read-only and places native build artifacts under autonomy-sim's `target/dittoffi`:

```bash
make test-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

Override `DITTO_BUILD_TARGET_DIR=/another/build/directory` if the default artifact location is unsuitable.

The tests start fresh native peer processes. One verifies direct replication; another first synchronizes over a live link, removes that link, proves that a newly written document stays isolated, restores the link, and proves eventual convergence. A full-simulator test selects the real transport and verifies that scenario documents replicate between real peers.

The defensive stadium path has a focused license-gated case:

```bash
make test-cuas-real DITTO_SOURCE_DIR=/home/sturner/projects/ditto
```

It configures `RealDittoConfig.collections` to exactly `cuas.tracks`, `cuas.ew_assignments`, and `cuas.engagements`, creates only the eight radio-equipped friendly defenders as peers, applies only defender-to-defender links, and asserts real convergence of all three collections. Hostile simulated tracks and the protected site are never Ditto peers.

Run the complete simulator with the real transport selected:

```bash
make demo-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

This is equivalent to building `autonomy-sim` with `--features ditto-real` and launching it with `--ditto real`. Pass normal simulator and native options through `DITTO_REAL_ARGS`:

```bash
DITTO_REAL_ARGS='--scenario wildfire-paradise --bind 127.0.0.1:9100 --ditto-port-base 47000' \
  make demo-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

Peer stores default to `target/ditto-real/<scenario>`. Other selector options are `--ditto-storage-root`, `--ditto-database-id`, and `--ditto-listen-ip`. The offline license is read only from `DITTO_LICENSE`, not a command-line or scenario value.

Select the startup scenario with `--scenario` when using real Ditto. The HTTP/WebSocket hot-switch endpoint returns `409 Conflict` in this mode because the current native peers must release their explicit ports and persistent stores before another scenario can create its peer set; restart the process to switch. Behavioral mode retains hot switching.

To run the focused two-peer gated harness interactively instead:

```bash
make demo-ditto-peers DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

The harness accepts `DITTO_REAL_MODE=converge`, `DITTO_REAL_PORT_BASE=47000`, and `DITTO_REAL_STORAGE_ROOT=/path`. The Make targets unset `NO_COLOR` because the referenced Ditto runtime currently parses that variable as a boolean system parameter and does not accept the conventional value `1`.

For a prebuilt Ditto library, bypass the build target and point Cargo at the header and library directories:

```bash
export DITTOFFI_INCLUDE_DIR=/absolute/path/to/ditto/crates/dittoffi
export DITTOFFI_LIB_DIR=/absolute/path/containing/libdittoffi.so
export LD_LIBRARY_PATH="$DITTOFFI_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
env -u NO_COLOR cargo run -p autonomy-sim --features ditto-real -- --ditto real
```

### Transport behavior

`RealDittoTransport::new` creates a persistent Ditto peer for every supplied peer entity, subscribes it to the configured collection list, and disables ambient discovery transports. An empty `RealDittoConfig.collections` list retains the C2, telemetry, and wildfire defaults; the C-UAS simulator supplies only its three defensive coordination collections. Each peer listens on an explicit TCP port.

The simulator converts each current `NetworkBackend::link_states` frame into `apply_links` input. Every entity pair with at least one `Up` carrier receives exactly one explicit Ditto connection. Removing its final up carrier removes that connection, so an emulated partition prevents document exchange; restoring a carrier allows Ditto to converge its actual CRDT collection. `write_document`, `read_document`, and `observe` expose real DQL data and replica/convergence state, which populate the existing v1 Ditto peer/document/event fields.

Reachability is real and gated, but the adapter does not yet apply per-packet delay, loss, or capacity to Ditto's TCP stream. CORE/EMANE or a traffic-control layer can provide that shaping without changing the transport API.

## SigForge `NetworkBackend`

Construct the production REST adapter with:

```rust
use autonomy_sim_net_sigforge::SigForgeNetworkBackend;

let backend = SigForgeNetworkBackend::connect("http://127.0.0.1:8080")?;
```

On `register_nodes`, the adapter fetches `GET /api/v1/session/nodes`, sorts the returned NEM IDs, and maps them deterministically to the supplied entity order. Every `link_states` call sends each position to `PUT /api/v1/session/nodes/{nem_id}/position` and consumes the directed PHY matrix from `GET /api/v1/session/links`.

Ditto requires bidirectional reachability, so both directed measurements must exist and meet the configured SINR threshold. The weaker SINR drives normalized quality; compatible entity radios supply the carrier type, capacity, and base latency. A missing reverse measurement fails the pair closed. `SigForgeMapping` controls the SINR threshold, full-quality point, and adapter latency.

`SigForgeApi` is the narrow trait boundary for alternate WS/gRPC clients and is exercised with a fake client. A loopback HTTP test verifies the concrete REST paths and request bodies. The current SigForge reference service acknowledges REST position updates but may require its WS mobility path for a live EMANE session; the adapter still publishes the update and consumes the real link matrix. The adapter supports plain HTTP, so use a local TLS-terminating proxy if necessary.

The existing scenario setting `network_backend = "sigforge"` now constructs this production REST adapter using `sigforge_url`; `analytic` remains the default. The CLI selects the document transport independently with `--ditto behavioral|real`. Neither selection changes the v1 wire schema.

Run an existing scenario against a live SigForge session without editing its TOML:

```bash
make demo-sigforge \
  SCENARIO=isr-relay-demo \
  SIGFORGE_URL=http://127.0.0.1:8080
```

The equivalent direct command is:

```bash
cargo run -p autonomy-sim -- \
  --scenario isr-relay-demo \
  --network-backend sigforge \
  --sigforge-url http://127.0.0.1:8080
```

`--network-backend analytic` explicitly forces the zero-dependency analytic backend. With no CLI network selector, the scenario's `network_backend` and `sigforge_url` remain authoritative. Passing `--sigforge-url` without `--network-backend sigforge` is rejected to avoid silently selecting a live external backend.

The network and document selectors are independent, so a real-Ditto scenario can consume SigForge link state with:

```bash
DITTO_REAL_ARGS='--scenario isr-relay-demo --network-backend sigforge --sigforge-url http://127.0.0.1:8080' \
  make demo-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```
