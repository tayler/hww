run:
	cargo run --features gui --bin hww

# The five gates in .github/workflows/ci.yml, in the workflow's order. `.githooks/pre-push`
# runs this before every push; `git push --no-verify` skips it.
ci:
	cargo fmt --all -- --check
	cargo clippy --all-targets --locked -- -D warnings
	cargo test --locked
	cargo clippy --all-targets --locked --features gui -- -D warnings
	cargo test --locked --features gui

.PHONY: run ci
