.PHONY: setup demo frontend check clean-output

setup:
	npm --prefix frontend install

demo:
	cargo run -- --scenario scenarios/isr-demo.toml

frontend:
	npm --prefix frontend run dev

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	npm --prefix frontend run build

clean-output:
	find output -type f -name '*.cot' -delete 2>/dev/null || true

