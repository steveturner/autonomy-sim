.PHONY: setup live demo wildfire wildfire-live frontend check build-dittoffi test-ditto-real demo-ditto-real demo-ditto-peers clean-output

HOST ?= 127.0.0.1
DITTO_SOURCE_DIR ?=
DITTO_BUILD_TARGET_DIR ?= $(CURDIR)/target/dittoffi

setup:
	npm --prefix frontend install

live:
	@set -eu; \
		cargo run -- --scenario scenarios/isr-demo.toml --bind $(HOST):9000 & sim_pid=$$!; \
		trap 'kill "$$sim_pid" 2>/dev/null || true' EXIT INT TERM; \
		VITE_BIND_HOST=$(HOST) npm --prefix frontend run dev

demo:
	cargo run -- --scenario scenarios/isr-demo.toml --bind $(HOST):9000

wildfire:
	cargo run -- --scenario wildfire-paradise --bind $(HOST):9000

wildfire-live:
	@set -eu; \
		cargo run -- --scenario wildfire-paradise --bind $(HOST):9000 & sim_pid=$$!; \
		trap 'kill "$$sim_pid" 2>/dev/null || true' EXIT INT TERM; \
		VITE_BIND_HOST=$(HOST) npm --prefix frontend run dev

frontend:
	VITE_BIND_HOST=$(HOST) npm --prefix frontend run dev

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	npm --prefix frontend run build

build-dittoffi:
	@test -n "$(DITTO_SOURCE_DIR)" || { echo "set DITTO_SOURCE_DIR to a Ditto source checkout"; exit 2; }
	CARGO_TARGET_DIR="$(DITTO_BUILD_TARGET_DIR)" cargo build --locked --manifest-path "$(DITTO_SOURCE_DIR)/Cargo.toml" -p dittoffi --release --no-default-features --features explicit-fs-storage

test-ditto-real: build-dittoffi
	@test -n "$${DITTO_LICENSE:-}" || { echo "set DITTO_LICENSE to an offline Ditto license"; exit 2; }
	env -u NO_COLOR DITTO_SOURCE_DIR="$(DITTO_SOURCE_DIR)" DITTOFFI_LIB_DIR="$(DITTO_BUILD_TARGET_DIR)/release/deps" RUST_LOG="$${RUST_LOG:-warn}" cargo test -p autonomy-sim-ditto-real --features dittoffi --test real_peers
	env -u NO_COLOR DITTO_SOURCE_DIR="$(DITTO_SOURCE_DIR)" DITTOFFI_LIB_DIR="$(DITTO_BUILD_TARGET_DIR)/release/deps" LD_LIBRARY_PATH="$(DITTO_BUILD_TARGET_DIR)/release/deps$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" RUST_LOG="$${RUST_LOG:-warn}" cargo test -p autonomy-sim --features ditto-real --test real_ditto_runtime

demo-ditto-real: build-dittoffi
	@test -n "$${DITTO_LICENSE:-}" || { echo "set DITTO_LICENSE to an offline Ditto license"; exit 2; }
	env -u NO_COLOR DITTO_SOURCE_DIR="$(DITTO_SOURCE_DIR)" DITTOFFI_LIB_DIR="$(DITTO_BUILD_TARGET_DIR)/release/deps" LD_LIBRARY_PATH="$(DITTO_BUILD_TARGET_DIR)/release/deps$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" RUST_LOG="$${RUST_LOG:-warn}" cargo run -p autonomy-sim --features ditto-real -- --ditto real $${DITTO_REAL_ARGS:-}

demo-ditto-peers: build-dittoffi
	@test -n "$${DITTO_LICENSE:-}" || { echo "set DITTO_LICENSE to an offline Ditto license"; exit 2; }
	env -u NO_COLOR DITTO_SOURCE_DIR="$(DITTO_SOURCE_DIR)" DITTOFFI_LIB_DIR="$(DITTO_BUILD_TARGET_DIR)/release/deps" RUST_LOG="$${RUST_LOG:-warn}" cargo run -p autonomy-sim-ditto-real --features dittoffi --bin autonomy-sim-ditto-real-demo

clean-output:
	find output -type f -name '*.cot' -delete 2>/dev/null || true
