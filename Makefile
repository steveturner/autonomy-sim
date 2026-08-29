.PHONY: setup live demo wildfire wildfire-live frontend check clean-output

HOST ?= 127.0.0.1

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

clean-output:
	find output -type f -name '*.cot' -delete 2>/dev/null || true
