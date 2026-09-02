build:
	cargo build --release --package pigment-lsp

test:
	cargo test --workspace --all-targets --locked
