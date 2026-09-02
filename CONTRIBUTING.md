# Contributing

Contributions to Zed Pigments are welcome.

## Local development

Run the repository checks before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

Build the native server with:

```sh
cargo build --release --package pigment-lsp
```

Put `target/release` on the `PATH` inherited by Zed, then install `zed-pigments` through
Zed's **Install Dev Extension** action. The extension deliberately prefers a
`pigment-lsp` already available in the worktree environment, which avoids requiring a
GitHub release during development.

Parser changes should include focused tests for accepted syntax, token boundaries,
false positives, variables, and UTF-16 ranges. Keep variable behavior safe and
document-local unless a change introduces explicit project indexing.

## Releases

Keep the package and extension manifest versions aligned. After review, a `v*` tag
builds the native archives and publishes a GitHub release. Publishing or updating the
Zed registry entry is intentionally a separate manual pull request to
`zed-industries/extensions`.

The project derives from `huacnlee/color-lsp` and is inspired by
`abe33/atom-pigments`; preserve their attribution and MIT notices.
