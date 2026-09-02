# Zed Pigments extension

This directory contains the Rust/WASM Zed extension shim. It launches a local
`pigment-lsp` from `PATH` during development or downloads the matching
`pigment-lsp-{os}-{arch}` archive from
[`JonathonRP/zed-pigments`](https://github.com/JonathonRP/zed-pigments/releases).

Install this directory with Zed's **Install Dev Extension** action. See the
[repository README](../README.md) for build instructions, supported syntax, rendering
settings, limitations, and attribution.
