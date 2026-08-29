# Real Ditto and SigForge adapters

The analytic `NetworkBackend` and behavioral Ditto model remain autonomy-sim's dependency-free defaults. Two additive workspace crates provide the production seams:

- `autonomy-sim-ditto-real` owns one actual Ditto small peer and persistent store per scenario entity.
- `autonomy-sim-net-sigforge` implements `NetworkBackend` against SigForge's session REST API.

## Run two real Ditto peers

The native transport is feature-gated because building Ditto is substantially heavier than the default simulator and requires a licensed Ditto source checkout. The current bridge is intended for Linux/CORE-EMANE environments.

Set the checkout and an offline-playground license in your shell. Do not commit the license or put it in a scenario:

```bash
export DITTO_SOURCE_DIR=/absolute/path/to/ditto
export DITTO_LICENSE='<offline Ditto license>'
```

Build `libdittoffi`, then run the two integration cases. The Make target treats the Ditto checkout as read-only and places native build artifacts under autonomy-sim's `target/dittoffi`:

```bash
make test-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

Override `DITTO_BUILD_TARGET_DIR=/another/build/directory` if the default artifact location is unsuitable.

The tests start fresh native peer processes. One verifies direct replication; the other first synchronizes over a live link, removes that link, proves that a newly written document stays isolated, restores the link, and proves eventual convergence.

Run the same gated flow interactively:

```bash
make demo-ditto-real DITTO_SOURCE_DIR="$DITTO_SOURCE_DIR"
```

Use `DITTO_REAL_MODE=converge` for an initially connected demo, `DITTO_REAL_PORT_BASE=47000` to choose the first of two consecutive ports, or `DITTO_REAL_STORAGE_ROOT=/path` to retain the peer stores. The Make targets unset `NO_COLOR` because the referenced Ditto runtime currently parses that variable as a boolean system parameter and does not accept the conventional value `1`.

For a prebuilt Ditto library, bypass the build target and point Cargo at the header and library directories:

```bash
export DITTOFFI_INCLUDE_DIR=/absolute/path/to/ditto/crates/dittoffi
export DITTOFFI_LIB_DIR=/absolute/path/containing/libdittoffi.so
env -u NO_COLOR cargo test -p autonomy-sim-ditto-real --features dittoffi --test real_peers
```

### Transport behavior

`RealDittoTransport::new` creates a persistent Ditto peer for every supplied entity, subscribes it to `c2.tasking`, `c2.pli`, `c2.tracks`, and `telemetry.platform`, and disables ambient discovery transports. Each peer listens on an explicit TCP port.

Call `apply_links` with the current `NetworkBackend::compute_links` result. Every entity pair with at least one `Up` carrier receives exactly one explicit Ditto connection. Removing its final up carrier removes that connection, so an emulated partition prevents document exchange; restoring a carrier allows Ditto to converge its actual CRDT collection. `write_document`, `read_document`, and `observe` expose real DQL data and replica/convergence state to the simulator integration layer.

Reachability is real and gated, but the adapter does not yet apply per-packet delay, loss, or capacity to Ditto's TCP stream. CORE/EMANE or a traffic-control layer can provide that shaping without changing the transport API.

## SigForge `NetworkBackend`

Construct the production REST adapter with:

```rust
use autonomy_sim_net_sigforge::SigForgeNetworkBackend;

let backend = SigForgeNetworkBackend::connect("http://127.0.0.1:8080")?;
```

On its first `compute_links` call, the adapter fetches `GET /api/v1/session/nodes`, sorts the returned NEM IDs, and maps them deterministically to the supplied entity order. Every call sends each position to `PUT /api/v1/session/nodes/{nem_id}/position` and consumes the directed PHY matrix from `GET /api/v1/session/links`.

Ditto requires bidirectional reachability, so both directed measurements must exist and meet the configured SINR threshold. The weaker SINR drives normalized quality; compatible entity radios supply the carrier type, capacity, and base latency. A missing reverse measurement fails the pair closed. `SigForgeMapping` controls the SINR threshold, full-quality point, and adapter latency.

`SigForgeApi` is the narrow trait boundary for alternate WS/gRPC clients and is exercised with a fake client. A loopback HTTP test verifies the concrete REST paths and request bodies. The current SigForge reference service acknowledges REST position updates but may require its WS mobility path for a live EMANE session; the adapter still publishes the update and consumes the real link matrix. The adapter supports plain HTTP, so use a local TLS-terminating proxy if necessary.

The two crates deliberately do not alter autonomy-sim's scenario schema or v1 wire contract. A simulator selector can instantiate `SigForgeNetworkBackend` for its network mode and `RealDittoTransport` for its Ditto mode while retaining the analytic and behavioral defaults.
