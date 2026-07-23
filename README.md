# Sizer

The universal file compression engine, built in Rust, shared by every
surface: CLI, desktop, browser (WASM), and cloud API. Drop a file, get back
the smallest correct output, without ever choosing an algorithm yourself.

> **Status: pre-alpha, milestone M7.** This is a from-scratch rewrite of an
> earlier browser-only prototype. Archive compression (gzip/zstd), image
> recompression (PNG/JPEG), video recompression (via a system-installed
> ffmpeg), and now PDF embedded-image recompression all work end-to-end via
> the CLI and a Tauri desktop app; the browser build covers gzip + JPEG only
> (see [Roadmap](#roadmap)/`docs/ARCHITECTURE.md` for why) and has no
> video/document support. Cloud mode (M6) is paused pending an
> infrastructure decision; mobile (M8) doesn't exist yet.

## Why Rust, why one core

Every "compress this file" tool either runs in a browser tab (limited by JS
performance and available memory) or uploads to a server you have to trust.
Sizer's engine is a single Rust crate (`sizer-core`) with no platform
assumptions baked in — every surface (Tauri desktop, WASM in the browser,
Axum cloud API, Flutter mobile via FFI) is a thin adapter around the same
code, so there is exactly one place compression logic lives and one place
it gets tested. The desktop app and browser build are both fully offline:
no network calls, no database, no telemetry, nothing uploaded anywhere.

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the full design
rationale, including the licensing and toolchain constraints that shaped it
(why RAR is decode-only, why HEIC encoding isn't shipped, why decompression
has hard size-ratio limits, why image/video/document codecs use different
traits than archive codecs, why the desktop/browser UIs have no Node/npm
dependency, why the browser build only has gzip + JPEG so far, why video
shells out to a system ffmpeg instead of bundling one, and why PDF support
only touches embedded images so far).

## Workspace layout

```
crates/
  sizer-core/            Platform-agnostic Codec/Detector/Progress traits,
                          SHA-256 integrity verification, the decompression-
                          bomb guard (LimitedWriter). No codec implementations,
                          no surface-specific IO. wasm32-compatible.
  sizer-codecs-archive/  Streaming gzip (wasm32-compatible) + zstd
                          (native-only, see ARCHITECTURE.md) codecs.
  sizer-codecs-image/    JPEG (lossy, wasm32-compatible) + PNG (lossless,
                          native-only) recompression codecs (ImageCodec
                          trait -- a different shape than Codec).
  sizer-codecs-video/    Video recompression by shelling out to a
                          system-installed ffmpeg (VideoCodec trait --
                          file-path-based, a third codec shape). Native-only.
  sizer-codecs-document/ PDF embedded-JPEG recompression via lopdf
                          (DocumentCodec trait). Native-only.
  sizer-registry/        Codec name/format -> instance lookup, shared by
                          every surface so it's defined exactly once.
  sizer-cli/              CLI front end: detect / compress / decompress /
                          bench / image-compress / video-compress /
                          document-compress.
  sizer-desktop/          Tauri desktop shell. Plain HTML/CSS/vanilla-JS
                          frontend (ui/), no Node toolchain.
  sizer-wasm/             wasm-bindgen browser surface (gzip + JPEG only --
                          no video, no documents, no zstd, no PNG; see
                          ARCHITECTURE.md). Plain HTML/CSS/vanilla-JS
                          frontend (web/), runs inside a dedicated Web
                          Worker so compression never blocks the page.
```

More crates land per the roadmap below (`sizer-api`, `sizer-worker`).

## Getting started

Requires a recent stable [Rust toolchain](https://rustup.rs). Video
recompression additionally requires [FFmpeg](https://ffmpeg.org) installed
and on `PATH` (`ffmpeg -version` should work) -- everything else, including
PDF support, needs no extra system dependencies.

```bash
cargo build --workspace

cargo run -p sizer-cli -- detect ./some-file
cargo run -p sizer-cli -- compress ./some-file ./some-file.zst --codec zstd --verify
cargo run -p sizer-cli -- decompress ./some-file.zst ./some-file.out
cargo run -p sizer-cli -- bench ./some-file   # vs system gzip/zstd, if installed

cargo run -p sizer-cli -- image-compress ./photo.png ./photo.small.png --codec png --check-fidelity
cargo run -p sizer-cli -- image-compress ./photo.png ./photo.jpg --codec jpeg --effort 80 --check-fidelity

cargo run -p sizer-cli -- video-compress ./clip.mp4 ./clip.small.mp4 --effort 70 --check

cargo run -p sizer-cli -- document-compress ./report.pdf ./report.small.pdf --effort 60

cargo run -p sizer-desktop   # launches the desktop app
```

```bash
cargo fmt --all -- --check     # formatting
cargo clippy --workspace --all-targets -- -D warnings   # lints
cargo test --workspace --all-targets                     # unit tests
```

### Browser build

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <matches the wasm-bindgen version in Cargo.lock> --locked

cargo build -p sizer-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir crates/sizer-wasm/pkg \
  target/wasm32-unknown-unknown/release/sizer_wasm.wasm

# serve crates/sizer-wasm/ over HTTP (ES module workers need a real
# origin, not file://) with any static file server, then open web/index.html
```

## Roadmap

| # | Milestone | Status |
|---|---|---|
| M0 | Workspace scaffold: `sizer-core` traits, CLI shell, CI | ✅ done |
| M1 | Streaming archive codec (zstd/gzip) end-to-end, benchmarked vs system tools | ✅ done |
| M2 | Image domain: PNG (lossless) + JPEG (lossy) recompression, pixel-fidelity verification | ✅ done (WebP/AVIF encode deferred, see ARCHITECTURE.md) |
| M3 | Tauri desktop shell, offline-only | ✅ done |
| M4 | WASM build + browser UI | ✅ done (gzip + JPEG only; zstd/PNG need a wasm-capable C toolchain, see ARCHITECTURE.md; true multi-core "threaded" WASM also deferred) |
| M5 | Video domain, shelling out to a system-installed ffmpeg | ✅ done (native-only -- no browser video, see ARCHITECTURE.md) |
| M6 | Cloud API + worker pool + Postgres/Redis/S3, opt-in, sandboxed | ⏸ Paused (infra/deployment-target decision pending) |
| M7 | Document domain: PDF embedded-image recompression | ✅ done (embedded JPEGs only -- no rasterization, fonts, or Office formats yet, see ARCHITECTURE.md) |
| M8 | Mobile (Flutter + Rust FFI), plugin system formalized | Next |

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for the dev workflow and coding
conventions.

## License

[MIT](./LICENSE)
