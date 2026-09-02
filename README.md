# Zed Pigments

[![CI](https://github.com/JonathonRP/zed-pigments/actions/workflows/test.yml/badge.svg)](https://github.com/JonathonRP/zed-pigments/actions/workflows/test.yml)

Zed Pigments previews colors through Zed's native Language Server Protocol (LSP)
document-color support. It pairs a cross-platform Rust server, `pigment-lsp`, with a
small Rust/WASM Zed extension. The project is inspired by
[Atom Pigments](https://github.com/abe33/atom-pigments) and is based on
[ColorLSP](https://github.com/huacnlee/color-lsp).

This repository is preparing its first public release. It is not in the Zed extension
registry yet.

## Install as a dev extension

1. Clone this repository.
2. Build the server with `cargo build --release --package pigment-lsp`.
3. Put `target/release` on the `PATH` inherited by Zed.
4. In Zed, open **Extensions**, choose **Install Dev Extension**, and select the
   `zed-pigments` directory.

Once reviewed releases exist, the extension will download the exact binary for the
current OS and architecture from
[`JonathonRP/zed-pigments`](https://github.com/JonathonRP/zed-pigments/releases).
Registry publication is a separate post-review step.

## Supported colors

| Family | Examples |
| --- | --- |
| CSS hex | `#f0c`, `#f0c8`, `#ff00cc`, `#ff00cc88` |
| Hex literals | `0xff00cc`, `0xFF00CC88` |
| RGB | `rgb(255, 0, 204)`, `rgb(100% 0% 80% / 50%)` |
| HSL | `hsl(312 100% 50%)`, `hsla(312, 100%, 50%, .5)` |
| CSS Color 4 | `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()` |
| Named colors | `rebeccapurple`, `transparent` in value contexts |

Named colors are recognized only in declaration, assignment, argument, or quoted value
contexts. An identifier such as `red` in prose, a class name, or a variable declaration
is not treated as a color. Language-aware parsing keeps named colors out of Markdown
prose while honoring CSS's case-insensitive property names.

Document-local variables are resolved recursively for these common forms:

```css
:root {
  --brand: oklch(62% 0.2 25);
  --accent: var(--brand);
}

.button {
  color: var(--accent);
}
```

Sass (`$brand: ...`), Less (`@brand: ...`), and Stylus (`brand = ...` or
`$brand = ...`) assignments and references are also supported when their value resolves
directly to a color or another supported variable. Cycles and unresolved values are
ignored.

For safe CSS semantics, custom properties resolve only when declared at document top
level or under `:root`. Selector-scoped custom properties are left unresolved rather
than leaking their value into unrelated selectors. If a name has any selector-scoped
override, references to that name remain unresolved because selector inheritance is not
modeled; unrelated global custom properties continue to resolve.

## Zed rendering

Set Zed's native document-color mode in `settings.json`:

```json
{
  "lsp_document_colors": "inlay"
}
```

`inlay` is the closest supported equivalent to Atom Pigments' dot marker and is Zed's
default: it places a compact color swatch beside each value. Current Zed releases render
that swatch as a **square**, not a circle. The Zed extension and LSP APIs do not expose
the swatch shape, and Zed Pigments deliberately does not insert Unicode markers or edit
your document to imitate one. A circular dot requires a Zed core renderer change.

The other native modes are `"border"`, `"background"`, and `"none"`. Border and
background can be useful when a larger preview is preferred.

Zed requests `textDocument/colorPresentation` when replacing a color. Zed Pigments
prefers the existing hex, `0x`, RGB, or HSL style when practical and also offers
equivalent hex, modern RGB, and HSL presentations. Hovering a color shows its RGBA and
common CSS equivalents.

## Architecture and scope

The extension launches a native `pigment-lsp` process and Zed communicates with it over
stdio. The server truthfully advertises full document synchronization,
`textDocument/documentColor`, `textDocument/colorPresentation`, and hover. Ranges use
the LSP-required UTF-16 positions, including after non-ASCII and supplementary-plane
characters.

Variable resolution is intentionally document-local. Cross-file Sass/Less imports,
project palettes, and build-tool transformations would require project indexing and
language-specific evaluation.

Unlike Atom Pigments, Zed's extension API does not expose a custom project palette or
search UI, arbitrary editor decorations, or control over document-color swatch shape.
Zed Pigments therefore uses Zed's supported document-color render modes rather than
claiming full Atom feature parity.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

Release tags build `pigment-lsp` archives for x86-64 and ARM64 Windows, macOS, and
Linux. Do not create a tag until the implementation has been reviewed.

## Attribution and license

Zed Pigments is a fork and extension of Jason Lee's
[huacnlee/color-lsp](https://github.com/huacnlee/color-lsp), and its product direction is
inspired by [abe33/atom-pigments](https://github.com/abe33/atom-pigments). Those projects
and their contributors retain their respective copyright notices.

This repository remains available under the [MIT License](LICENSE). The accepted MIT
license is also retained inside the Zed extension directory for future registry
submission.
