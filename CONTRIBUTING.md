# Contributing to Sizer

Thanks for taking the time to contribute. This project holds a high bar for
correctness because the codebase is a compression/decompression engine that
runs untrusted, arbitrary input — bugs here are usually security bugs, not
just cosmetic ones.

## Development setup

Requires a recent stable [Rust toolchain](https://rustup.rs) (`rustup update
stable`). Video work (`sizer-codecs-video`, `video-compress`) additionally
needs [FFmpeg](https://ffmpeg.org) on `PATH` -- its tests skip gracefully
without it (see `require_ffmpeg!` in that crate) rather than failing, but
you'll want it installed to actually exercise that code.

```bash
git clone https://github.com/<your-fork>/scizer.git
cd scizer
cargo build --workspace
cargo run -p sizer-cli -- detect ./some-file
```

## Before opening a PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

CI runs the same four checks; a PR that fails any of them won't merge.

## Project conventions

- **`sizer-core` has no codec implementations and no surface-specific IO.**
  It defines the `Codec`, `Detector`, and `Progress` traits and streams
  everything through `tokio::io::{AsyncRead, AsyncWrite}`. If you're adding
  an actual compression algorithm, it belongs in a new or existing
  `sizer-codecs-*` crate, not in core.
- **Everything streams — except whole-image codecs, deliberately.**
  Archive/video codecs must not buffer an entire input/output in memory —
  peak RAM should stay roughly constant whether the input is 10MB or
  100GB (see `integrity::hash` for the pattern). Image codecs
  (`sizer-codecs-image`) are a documented exception: PNG/JPEG re-encoding
  fundamentally needs whole-frame access, and image files are bounded in a
  way archive/video streams aren't — don't use this as precedent to add
  buffering elsewhere.
- **Multiple codec shapes, not one.** `sizer_core::Codec` is for
  *reversible* byte-stream formats (`decompress(compress(x)) == x`,
  checked via SHA-256 — archives). `sizer_codecs_image::ImageCodec` is for
  *lossy/perceptual, in-memory-buffer* recompression (checked via
  decoded-pixel fidelity — images). `sizer_codecs_video::VideoCodec` is
  for *lossy/perceptual, file-path-based* recompression (checked via a
  cheap validity probe, not full fidelity — video, where buffering the
  whole file defeats the point). `sizer_codecs_document::DocumentCodec` is
  structurally identical to `ImageCodec` but kept separate since a PDF
  isn't an image and the two domains should evolve independently. Don't
  force a codec into the wrong shape to avoid adding a new trait; propose
  a new shape if none of these fit. See `docs/ARCHITECTURE.md` ("Multiple
  codec shapes, not one", "Video (M5): shell out, don't bundle",
  "Documents (M7): embedded images only, not a PDF engine").
- **Video shells out to a system `ffmpeg`; it is never linked, vendored,
  or bundled.** This is a deliberate choice that sidesteps FFmpeg's own
  GPL/LGPL licensing question entirely (see "Video (M5)" in
  `docs/ARCHITECTURE.md`) — don't add `ffmpeg-next`/`ffmpeg-sys` FFI
  bindings or a vendored FFmpeg build without raising that discussion
  again first. If you touch `sizer-codecs-video`'s progress parsing,
  note that ffmpeg's `-progress` output has a real naming bug: its
  `out_time_ms` key reports **microseconds**, not milliseconds (confirmed
  by inspecting raw `-progress pipe:1` output) — the code parses
  `out_time_us` and divides by 1000 explicitly; don't "simplify" that back
  to trusting `out_time_ms`'s name.
- **Untrusted input gets a ratio/size guard, every time.** Any code path
  that decompresses attacker-controlled bytes must enforce
  `CompressOptions::max_decompressed_bytes` and return
  `Error::ExpansionLimitExceeded` rather than trusting the input's claimed
  size. Decompression bombs are the single most common CVE class in this
  product category — see `docs/ARCHITECTURE.md`.
- **Don't hand-roll parsers for security-critical formats.** Wrap an
  existing, audited library (e.g. `zstd`, `flate2`, `image`, `oxipng`,
  `jpeg-encoder`, `lopdf`) rather than writing a new JPEG/ZIP/PDF parser
  from scratch. Prefer pure-Rust encoders when one is viable (no C
  toolchain to build/maintain); reach for a heavier native dependency
  (`mozjpeg`, `libwebp`) only when the pure-Rust option genuinely can't
  hit the quality/ratio bar.
- **`PdfCodec` touches embedded JPEGs only.** No page rasterization, font
  subsetting, content-stream minification, or Office format (DOCX/XLSX/
  PPTX) support — that's deliberately scoped, not an oversight, since
  image-dominated PDFs (scans, photo-heavy documents) capture most of the
  realistic win for far less engineering than a general PDF optimizer.
  See "Documents (M7)" in `docs/ARCHITECTURE.md` before expanding scope
  here, and when verifying changes, prefer an independent tool (`qpdf
  --check`, decoding extracted streams with `ffmpeg`) over only trusting
  this codebase's own round-trip parse of its own output.
- **No placeholder/fake implementations.** If a feature isn't ready, don't
  wire it into the CLI/API behind a flag — leave it out of the tree until
  it's real. `sizer-cli`'s `detect` subcommand doc-comment explains what
  stage of the pipeline currently exists; keep that comment honest as more
  lands.
- **RAR is decode-only, HEIC encode is unimplemented.** These are
  deliberate licensing decisions, not oversights — see
  `docs/ARCHITECTURE.md` before "fixing" either.
- **No Node/npm in `sizer-desktop`.** The frontend (`crates/sizer-desktop/ui/`)
  is plain HTML/CSS/vanilla JS on purpose — don't introduce a bundler,
  React, or `package.json` for UI work there; extend the existing
  `app.js`/`style.css` instead.
- **Codec name/format lookup lives in `sizer-registry`, once.** If a new
  surface (WASM, cloud API) needs to map a codec name string to an
  instance, depend on `sizer-registry` rather than re-adding the match
  statement — that's exactly the duplication it was pulled out to avoid.
- **Cloud, database, or telemetry additions need a heads-up first.**
  Everything shipped so far (M0-M5, M7) is local-only by design (see
  "Cloud mode is opt-in, not core" in `docs/ARCHITECTURE.md`); M6 (the
  actual cloud API/worker/storage milestone) is currently paused pending
  an infrastructure/deployment-target decision, not abandoned. Don't add
  a network call, analytics, or a persistence layer to a local-only crate
  without raising it first — that's a scope decision, not an
  implementation detail.
- **Declare `tokio` features directly, not via `.workspace = true`.**
  `tokio.workspace = true` inherits the *whole* workspace default feature
  set (`rt-multi-thread`, `fs`, ...), which doesn't compile for
  `wasm32-unknown-unknown`. Every crate `sizer-wasm` depends on
  (`sizer-core`, `sizer-codecs-archive`, `sizer-codecs-image`) declares
  `tokio = { version = "1", default-features = false, features = [...] }`
  directly instead, listing only what it actually uses — keep doing that
  for any crate that might end up in the wasm dependency graph, and check
  new features you add against tokio's wasm32-allowlist (`sync`, `macros`,
  `io-util`, `rt`, `time` — notably *not* `rt-multi-thread`).
- **No Node/npm in `sizer-wasm` either.** Same rule as `sizer-desktop`,
  same reasoning — `crates/sizer-wasm/web/` is plain HTML/CSS/vanilla JS.
  Building it needs `wasm-bindgen-cli` (`cargo install wasm-bindgen-cli`,
  version must match the `wasm-bindgen` crate version in `Cargo.lock`
  exactly) but nothing from the npm ecosystem.
- **CPU-bound codec work goes through `run_blocking`, not
  `tokio::task::spawn_blocking` directly.** `spawn_blocking` doesn't exist
  on wasm32 (no OS thread pool) and doesn't need to — see
  `sizer_codecs_image::run_blocking`'s doc comment and "Browser build
  (M4)" in `docs/ARCHITECTURE.md`.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`,
`fix:`, `refactor:`, `docs:`, `test:`, `chore:`).

## Reporting bugs / requesting features

Use the issue templates. For compression correctness bugs, include the
input format/size, the codec/command used, and — if it's a data-integrity
bug — the expected vs. actual SHA-256 of the round-tripped output.

## Code of conduct

Be respectful. Assume good faith. Disagree on technical merits, not people.
