# Sizer architecture

## One engine, many surfaces

```
Desktop (Tauri)  ──┐
Browser (WASM)   ──┼──►  sizer-core  ◄── sizer-codecs-{image,video,archive,doc}
Cloud (Axum API) ──┤
Mobile (Flutter) ──┘
```

`sizer-core` defines three things and implements none of them itself:

- **`Codec`** — a streaming, bidirectional compress/decompress trait.
  Reads/writes are `tokio::io::{AsyncRead, AsyncWrite}`, never a
  fully-buffered `Vec<u8>`, so peak memory stays roughly constant whether
  the input is 10MB or 100GB.
- **`Detector`** — identifies a format from magic bytes (cheap, always run)
  with entropy/already-compressed analysis layered on top in the codec
  crates (heavier, only run when it changes the decision).
- **`Progress`** — a sink for progress events, implemented differently per
  surface (terminal bar in the CLI, a Tauri event, a `postMessage` from a
  web worker, a Redis-backed job status for the cloud worker).

Concrete codecs live in separate `sizer-codecs-*` crates and register
against these traits. This is also the plugin boundary: adding a new format
means adding a new codec crate, not touching `sizer-core`.

## Multiple codec shapes, not one

`Codec` (above) models *reversible* compression: `decompress(compress(x))`
reconstructs `x` exactly, and that's mechanically verifiable with a
SHA-256 comparison (`compress --verify` in the CLI does exactly this).
That's the right shape for archive formats — gzip, zstd, zip, 7z — where
the whole point is "get the identical bytes back later, just smaller in
transit."

It is the *wrong* shape for image (and later video) recompression.
Recompressing a JPEG produces a **different but valid** JPEG that decodes
to similar, not identical, pixels — there's no "decompress it back to the
original file bytes" operation to speak of, and forcing one through
`Codec::decompress` would mean returning something other than what the
method's own contract promises.

So `sizer-codecs-image` defines its own `ImageCodec` trait instead:
single-direction (`recompress`, not `compress`/`decompress`), and its
correctness check is [`sizer_codecs_image::compare_pixels`] — decode both
the original and the recompressed output and compare pixels, expecting an
exact match for lossless codecs (PNG, via `oxipng`) and an acceptable
delta for lossy ones (JPEG, via `jpeg-encoder`). The CLI reflects this
split: `compress`/`decompress`/`--verify` for archive codecs,
`image-compress`/`--check-fidelity` for image codecs — two different
commands, not one command papering over two different operations.

`VideoCodec` (M5, `sizer-codecs-video`) is a third shape: same
single-direction, non-reversible reasoning as `ImageCodec`, but
file-path-based rather than in-memory-buffer-based, since video is where
"never load the whole file into RAM" matters most. `DocumentCodec` (M7,
`sizer-codecs-document`) is structurally identical to `ImageCodec`
(single-direction, in-memory buffer) but kept as its own trait rather than
reusing `ImageCodec` directly — a PDF isn't an image, and the two domains
should be free to diverge later without one trait serving two unrelated
purposes. See "Video (M5)" and "Documents (M7)" below for each one's
specifics.

## Desktop shell (M3): no Node, no cloud, no telemetry

`sizer-desktop` is a Tauri app whose frontend is **plain HTML/CSS/vanilla
JS** — no React, no bundler, no `package.json`, no `node_modules`. That's
a deliberate call, not a placeholder: the whole point of the rewrite in
this repo's history was moving *off* a JS-heavy stack, and Tauri doesn't
require a JS framework for something this size — it just needs static
assets in `ui/` and calls into Rust via `invoke()`. Reintroducing a Node
toolchain for a page with a drop zone, a couple of dropdowns, and a
progress bar would have been exactly the kind of unnecessary dependency
the project's core principles rule out.

`sizer-desktop`'s own `Cargo.toml` depends on `sizer-registry` (below) and
the codec crates directly — it contains no compression logic of its own,
matching "never duplicate business logic." It's also fully offline: no
network calls, no database, no telemetry. `tauri-plugin-dialog` (native
file picker) is the only plugin in use; nothing in M3 talks to a server.

`sizer-registry` was pulled out during M3 once the desktop app needed the
exact same "codec name string -> instance" lookup the CLI already had in
`sizer-cli/src/registry.rs` — rather than duplicate that match statement a
second time, it's now a small shared crate both surfaces depend on.
`TauriProgress` (`sizer-desktop/src/progress.rs`) is the third concrete
implementation of `sizer_core::Progress`, alongside the CLI's terminal
progress line and the (not-yet-built) cloud worker's Redis-backed one —
confirming the trait's shape actually holds across three different sinks.

Tauri v2's capability system (`capabilities/default.json`) grants only
`core:default` (needed for progress events) and `dialog:default` (the
Browse button) — no filesystem or shell-execution plugin permissions, since
all file I/O happens through app-defined commands in `commands.rs`, not
through a generic fs plugin exposed to the frontend.

## Browser build (M4): wasm32, one thread, gzip + JPEG only

`sizer-wasm` is a `wasm-bindgen` surface over the *same* `sizer-core`/
`sizer-codecs-archive`/`sizer-codecs-image` crates the CLI and desktop app
use — no reimplementation. Getting there required tightening something
that had been sloppy since M0: `sizer-core`, `sizer-codecs-archive`, and
`sizer-codecs-image` all used to depend on `tokio` via
`tokio.workspace = true`, which inherits the *workspace's* feature set
(`rt-multi-thread`, `fs`, ...) regardless of whether the crate actually
needed all of that. Tokio's runtime/fs machinery doesn't compile for
`wasm32-unknown-unknown` at all — only its `io-util` trait definitions
(`AsyncRead`/`AsyncWrite`) do, since those are pure Rust with no OS
dependency. Each crate now declares its own minimal `tokio` feature set
directly instead of inheriting the workspace default, which is better
hygiene independent of WASM too (no crate should compile in runtime
machinery it never calls).

Two codecs didn't make it into the browser build, for reasons specific to
each, not a blanket "WASM is hard":

- **`zstd`** (`sizer-codecs-archive::ZstdCodec`) wraps `zstd-sys`, a
  binding to the real C `libzstd`. Compiling C for `wasm32-unknown-unknown`
  needs a wasm-targeting C compiler (e.g. an LLVM `clang` built with wasm32
  support, or Emscripten), which isn't set up in this project's toolchain.
  `gzip`'s C dependency (`flate2` → `miniz_oxide`) is the exception: pure
  Rust by default, no C compiler needed at all, so it builds for wasm32
  unmodified.
- **PNG** (`sizer-codecs-image::PngCodec`) wraps `oxipng`, which pulls in
  `libdeflate-sys` **unconditionally** — not behind its `parallel` feature
  as originally assumed when M2 shipped, so there is no wasm32-compatible
  configuration of `oxipng` at all without the same C-toolchain gap as
  zstd. `JpegCodec` has no such dependency (`jpeg-encoder` + `image`'s own
  JPEG codec are pure Rust) and builds unmodified.

Both gaps close the same way: set up a wasm32-targeting C toolchain in CI
and dev environments. Until then, `sizer-wasm` exposes gzip + JPEG only —
see its crate-level doc comment, which is the source of truth for current
scope, not this document.

**`spawn_blocking` doesn't exist on wasm32**, and doesn't need to: on
native, `sizer_codecs_image::run_blocking` uses
`tokio::task::spawn_blocking` to run oxipng/jpeg-encoder's synchronous,
CPU-bound work on tokio's blocking thread pool without stalling the async
runtime. There is no OS thread pool on `wasm32-unknown-unknown` to spawn
onto — but there's also no need for one: `sizer-wasm` is meant to be
instantiated inside a dedicated Web Worker (`crates/sizer-wasm/web/worker.js`),
not on the page's main thread, so "off the main/UI thread" is already true
at the JS level before any Rust code runs. `run_blocking`'s wasm32 branch
just calls the closure inline.

This is **not** the "threaded" WASM the product spec originally asked for
(SharedArrayBuffer + wasm threads splitting *one* compression job across
multiple cores) — that needs `-C target-feature=+atomics,+bulk-memory`,
often a nightly toolchain, and COOP/COEP response headers from whatever
serves the page, none of which is set up yet. What this milestone gets
right is not blocking the page while compressing, which is the more
commonly felt problem; true multi-core parallelism for a single job is a
larger, separately-scoped follow-up.

The browser UI (`crates/sizer-wasm/web/`) reuses the exact same plain
HTML/CSS/vanilla-JS approach and visual language as `sizer-desktop/ui/`
(down to sharing `style.css` near-verbatim) — no Node/npm here either, for
the same reasoning as the desktop shell below. `web/app.js` runs on the
main thread and only ever talks to `worker.js` via `postMessage`; it holds
no compression logic itself.

## Video (M5): shell out, don't bundle

`sizer-codecs-video` recompresses video by shelling out to a
system-installed `ffmpeg`/`ffprobe` via `tokio::process` — it does not
link against `libav*`, vendor an FFmpeg build, or ship any FFmpeg binary.
This directly **supersedes** the "FFmpeg must be built LGPL-only" note
written during M0 planning, which assumed Sizer would be the one
distributing FFmpeg (a bundled desktop binary, a Docker image for the
cloud worker, etc.) and would therefore need to police which codecs were
compiled in to stay LGPL-compliant. Shelling out to whatever the user
already has installed sidesteps that whole question: FFmpeg's GPL/LGPL
terms bind whoever builds and distributes the FFmpeg binary, and that's
the user's own OS package manager or installer, not Sizer. This is the
same category of decision as M4's "browser build only, don't try to bundle
`zstd-sys`/`libdeflate-sys` without a wasm C toolchain" — prefer not
owning a piece of infrastructure (a cross-platform FFmpeg build pipeline)
that a well-established one already exists for. If a future milestone
needs a bundled, no-install-required experience, revisit this note before
adding one.

Practical costs of that choice, both real: **users need FFmpeg on PATH
themselves** (`FfmpegCodec::recompress` fails with a message pointing at
this if it's missing, rather than a cryptic subprocess error), and there
is **no browser-side video support** — `sizer-codecs-video` depends on
`tokio::process` (spawning a real OS subprocess), which has no
`wasm32-unknown-unknown` equivalent, and shelling out to a binary makes no
sense inside a browser sandbox regardless of toolchain. Video recompression
stays a native-only feature (CLI, desktop) until/unless a real
browser-side video engine (not a shelled-out process) gets built
separately.

`VideoCodec` is a **third** codec shape, alongside `Codec` (reversible,
byte-stream) and `ImageCodec` (lossy, in-memory buffer): it operates on
**file paths**, not streams or buffers. Video is exactly the case where
"never load the whole file into RAM" matters most (the product spec's own
scale examples go up to 100GB), and `ffmpeg` itself reads/writes files
directly — routing bytes through a Rust-side buffer first would only add
a copy for no benefit. Progress is reported in **milliseconds of output
produced vs. total input duration**, not bytes — parsed from ffmpeg's
`-progress pipe:1` output, which has a well-known naming quirk worth
flagging so nobody "fixes" it later: the `out_time_ms` key actually
reports microseconds despite its name (confirmed by running ffmpeg with
`-progress pipe:1` directly and diffing `out_time_us` against
`out_time_ms` — identical values). `sizer-codecs-video` parses
`out_time_us` and divides by 1000 explicitly rather than trusting the
`_ms`-suffixed key.

Correctness verification is intentionally lighter than `ImageCodec`'s
pixel-fidelity check: full frame-by-frame comparison would mean decoding
every frame of both the original and recompressed video, which is
expensive enough not to belong in a default code path the way
`compare_pixels` does for still images. `sizer-cli video-compress --check`
instead does a cheap sanity check via `ffprobe` — does the output probe as
a valid video, and is its duration within 5% of the input's — catching
"ffmpeg exited 0 but wrote a truncated/corrupt file" without the cost of
real fidelity checking.

## Cloud (M6): presigned uploads, a worker that never terminates HTTP

`sizer-cloud` is the fourth surface over `sizer-core`, and the first one
with genuine data-liability surface -- see "Cloud mode is opt-in, not
core" below for why it's a separate, off-by-default product. It ships as
two binaries sharing one lib crate, never one process: `sizer-cloud-api`
(Axum HTTP -- auth, job bookkeeping, presigned URL issuance) and
`sizer-cloud-worker` (dequeues jobs, runs the codec, uploads the result).

**File bytes never pass through the API process.** A caller `POST`s
`/v1/jobs` and gets back a presigned S3 `PUT` URL; it uploads directly to
storage, then calls `/v1/jobs/{id}/submit`, which does a `HEAD` to confirm
the object landed (and check its size against `max_upload_bytes`) before
enqueueing the job in Redis. The API server's own memory footprint per
request stays flat regardless of file size -- the same "never buffer the
whole file" discipline `Codec`'s streaming trait enforces, applied at the
HTTP layer instead.

**The worker bridges S3 to `sizer-core`'s codec traits through a local
temp file, not a true streaming adapter.** `download_to_file` /
`upload_from_file` (`storage.rs`) copy to/from disk in chunks; the codec
then reads/writes that file directly (`tokio::fs::File` implements
`AsyncRead`/`AsyncWrite`, so archive codecs need no changes) or, for
`ImageCodec`, the whole buffer is read into memory (matching that trait's
existing `Vec<u8>`-in/`Vec<u8>`-out shape, unchanged from M2). This is a
deliberate simplification, not an oversight: single-file-at-a-time disk
buffering is still bounded memory (never the whole *concurrent job set* in
RAM at once, unlike buffering in the API process), and wiring S3's
`ByteStream` directly into `AsyncRead`/`AsyncWrite` would add real
complexity for a win (skipping one disk round-trip) that doesn't matter
next to network transfer time. Revisit only if disk I/O measurably
dominates job latency.

**Only `archive` and `image` job domains are wired up.** `VideoCodec`
(file-path in/out) and `DocumentCodec` (buffer in, `RecompressReport` out,
no progress callback) are structurally different enough from `archive`'s
streaming `Codec` and `image`'s buffer-based `ImageCodec` that dispatching
to them is separate follow-up work in the worker, not a blocked
architecture decision -- see `sizer-cloud`'s own README for the current
"implemented vs. deferred" list.

**Progress crosses a sync/async boundary via a channel, not a direct
Redis call.** `sizer_core::Progress::on_progress` is a synchronous method
codecs call inline from their hot loop (see `sizer-core/src/progress.rs`);
writing to Redis is an async network call. `RedisProgress` (`progress.rs`)
hands each update to an unbounded `mpsc` channel and returns immediately;
a background task drains it and writes to Redis. This is the fourth
concrete `Progress` implementation the M3 section above already
anticipated, alongside the CLI's terminal bar, Tauri's frontend event, and
the browser worker's `postMessage`.

**Local dev uses native Windows services, not Docker Compose,** despite
the M6-decisions note (see project memory / `sizer-cloud/README.md`)
originally planning Docker Compose + MinIO: Docker Desktop and WSL2 were
both broken on the primary dev machine (`wsl --status` returned
`REGDB_E_CLASSNOTREG`, unresolved without an admin-elevated WSL2 repair).
PostgreSQL 17, Memurai Developer (Redis-protocol-compatible; Microsoft's
own native Redis port is unmaintained/EOL, so there's no real "install
actual Redis on Windows" option), and MinIO Server all ship native Windows
builds installable via `winget` and were used instead. This changes
nothing about the code: it only ever speaks the standard Postgres/Redis/S3
client protocols, so this is purely a local-dev-environment substitution,
not a fork in the implementation.

## Documents (M7): embedded images only, not a PDF engine

`sizer-codecs-document`'s `PdfCodec` recompresses a PDF's embedded JPEG
(`DCTDecode`) image streams in place via `sizer_codecs_image::JpegCodec`,
using `lopdf` (pure Rust, no C dependency, same "prefer pure-Rust" pattern
as the image codecs) to parse and rewrite the PDF's object graph. It does
**not** rasterize or re-render pages, subset/optimize fonts, minify
content streams, or touch non-JPEG image filters (`FlateDecode`-only
raster data, JBIG2, CCITT fax, JPX) — this is deliberately the single
highest-value slice, not a general-purpose PDF optimizer: photo-heavy and
scanned-document PDFs (the common "why is this PDF 40MB" case) are usually
image-dominated in file size, so recompressing just the embedded JPEGs
captures most of the available win for a fraction of the engineering a
full structural optimizer would need.

`DocumentCodec` is structurally identical to `ImageCodec` (single-direction
`recompress`, in-memory buffer) but is its own trait — see "Multiple codec
shapes, not one" above for why that's not just duplication.

**Verification is a different shape than `ImageCodec`'s, and lighter than
`VideoCodec`'s cheap-probe compromise too**: there's no PDF rasterizer in
this project to compare rendered pages against, so `PdfCodec::recompress`
returns a `RecompressReport` (bytes plus how many images were actually
recompressed vs. skipped) rather than a fidelity score, and the structural
sanity check — does the output still parse as a valid PDF with the same
page count — happens in the codec itself (every unit test reopens its own
output via `lopdf`) rather than as an optional `--check` flag. During
development this was additionally checked against two tools with zero
relationship to this codebase: `qpdf --check` (confirms the rewritten
PDF's syntax/stream structure is valid) and `ffmpeg` decoding the
recompressed streams extracted back out of the PDF (confirms the embedded
JPEGs themselves are genuinely valid, correctly-sized images, not just
"a stream that happens to still parse"). Neither tool is a dependency of
the shipped code — they were verification-only, run once by hand.

Native-only, no `wasm32` build, for the same structural reason as `PngCodec`
in `sizer-codecs-image`: `lopdf` has no C dependency and would likely
compile for wasm32 fine on its own, but this hasn't been verified or
wired into `sizer-wasm`'s scope yet — treat that as unverified, not as
"confirmed unsupported," if picking this up later.

**Office formats (DOCX/XLSX/PPTX) are not implemented.** They're ZIP
containers with embedded XML and media, structurally closer to
`sizer-codecs-archive` + `sizer-codecs-image` composed together than to
PDF's object-graph model — a distinct, separately-scoped follow-up, not
"PDF support covers documents now."

## Conversion (`sizer-codecs-convert`): a different operation from compression

`sizer-codecs-convert` re-encodes files across formats -- PNG/JPEG/BMP/GIF/TIFF/ICO
image conversion, composing one or more images into a new PDF (one image
per page), and merging multiple existing PDFs into one. It does not
implement `Codec`/`ImageCodec`/`DocumentCodec`: conversion has no
reversibility or fidelity contract to check against (there's no
"decompress it back" or "compare pixels" operation that makes sense for
"turn this PNG into a BMP"), so it's a small set of free functions instead
of a fourth codec shape. Scope is deliberately bounded to what was asked
for, the same "single highest-value slice" reasoning as `PdfCodec`'s
embedded-JPEG-only scope -- not a general "any format to any format"
converter.

Image-to-PDF composition uses `printpdf`, constructing its `ImageXObject`
directly from an `image`-decoded RGB8 buffer rather than going through
`printpdf`'s own `embedded_images` feature: that feature pulls in
`printpdf`'s pinned `image = "0.24.3"` dependency, a different (semver-
incompatible) version from this workspace's `image = "0.25"`, which would
make `DynamicImage` two distinct, non-interchangeable types. Building the
`ImageXObject` by hand (public fields: width/height/color space/bits-per-
component/raw pixel bytes) sidesteps the version mismatch entirely and
avoids pulling in a second copy of `image` as a transitive dependency.
Also worth knowing if touching this code: `PdfDocumentReference::save_to_bytes`
calls `Rc::try_unwrap` internally and **panics** if any other clone of the
document handle is still alive -- `images_to_pdf` is written to only ever
hold one reference (never `.clone()` it) specifically to make that
structurally impossible rather than "probably fine in practice".

PDF merging has no equivalent built-in in `lopdf` (no `Document::merge`);
the implementation follows lopdf's own bundled `examples/merge.rs`
reference pattern (renumber each source document's objects into a shared
ID space, then splice their `Pages`/`Catalog` trees together), with that
example's bookmark/table-of-contents generation stripped out since this
crate's scope is "concatenate the pages," not building a navigable
outline.

Native-only, like `PdfCodec` and `PngCodec` -- the desktop app (its only
caller today) has no wasm32 target, so there was no reason to route this
through the wasm-compatibility discipline `sizer-codecs-image` maintains.

## Decisions that diverge from the "obvious" design

**RAR is decode-only.** The RAR format and its reference encoder are
RarLab's proprietary IP. `unrar` (decode-only) ships under a restrictive
free license that permits extraction but not building a compatible
*encoder*. Sizer extracts `.rar` archives; it creates `.zip`/`.7z`/`.tar.zst`
instead of `.rar`. Do not attempt to add RAR write support without a legal
review — this is a licensing constraint, not a missing feature.

**HEIC encoding is unimplemented; AVIF is the default "modern" target.**
HEIC is HEVC-based, and HEVC sits behind the MPEG-LA/Access Advance patent
pools — shipping an HEIC *encoder* commercially requires a patent-licensing
decision that hasn't been made. AVIF (AV1-based) is royalty-free under the
Alliance for Open Media and has no such blocker. Sizer reads HEIC (e.g. to
re-encode iPhone photos) but does not write it.

**EXE/MSI/DMG/ISO are not a distinct format domain.** These are opaque
binary blobs; the only thing compression can do to them is generic archival
compression (the same path as ZIP/7z). They are routed through
`sizer-codecs-archive`, not treated as a fourth "software" domain with its
own logic.

**No hand-rolled parsers for security-critical formats.** Binary format
parsers (JPEG, ZIP, PDF, video containers) are historically the single
largest CVE source in this exact product category — 7-Zip, WinRAR, and
`unzip` have all shipped critical parser vulnerabilities. Codec crates wrap
existing, audited libraries (`zstd`, `flate2`, `image`, `oxipng`,
`jpeg-encoder`, `ffmpeg`, `lopdf`) rather than reimplementing format
parsing. "Zero
unnecessary dependencies" is a constraint on *product* bloat (no framework
sprawl), not a license to reinvent security-critical binary parsing.
`mozjpeg`/`ravif`/`libwebp` (heavier, C-toolchain-dependent encoders) are
deliberately deferred — M2 only pulls in pure-Rust encoders plus
`oxipng`'s small `libdeflate` C dependency; see
`sizer-codecs-image`'s crate-level doc comment for the current state and
why the bigger native encoders aren't in yet.

**FFmpeg: shelled out to, not bundled.** *(Superseded from this note's
original M0-planning form, which assumed Sizer would distribute FFmpeg
itself and need to pin an LGPL-only build to do so safely. That's no
longer the plan — see "Video (M5): shell out, don't bundle" above for the
current approach and why it sidesteps the LGPL question entirely.)*

## Decompression-bomb defense

Any code path that decompresses attacker-controlled bytes (cloud mode,
archive extraction from an untrusted upload) enforces
`CompressOptions::max_decompressed_bytes` and fails closed with
`Error::ExpansionLimitExceeded` rather than trusting the input's claimed
size. This is not optional per-codec behavior — it is the reason
`max_decompressed_bytes` is a field on the shared `CompressOptions` struct
rather than something each codec crate reinvents.

Implemented as of M1: `sizer_core::LimitedWriter` wraps the output side of
every decompress call (see `sizer-codecs-archive`'s gzip/zstd codecs) and
fails the write itself, mid-stream, the moment the limit is crossed —
decompression stops immediately rather than after the fact, so an
attacker's expansion ratio never gets to run to completion.

Cloud mode (M6, `sizer-cloud`) additionally enforces a per-job
**wall-clock** timeout (`JOB_WALL_CLOCK_TIMEOUT_SECS`, via
`tokio::time::timeout` around the codec call in `worker.rs`). Still not
implemented anywhere: nesting-depth limits on archive-within-archive
extraction, real per-job CPU/memory quotas, per-caller rate/quota
enforcement (the schema has `api_keys.quota_bytes_per_day`; nothing reads
it yet), and process-level sandboxing (seccomp + cgroups at minimum) --
the last of these is a container/orchestrator-level concern for whatever
actually runs `sizer-cloud-worker` in production, not something the
worker binary can self-enforce. See `sizer-cloud/README.md`'s
"implemented vs. deferred" list.

## Cloud mode is opt-in, not core

Local/offline compression (CLI, desktop, browser) has no server dependency
and no data-liability surface. Cloud mode (Postgres for metadata, Redis for
job queuing, S3-compatible storage for file bytes, a stateless worker pool)
is a genuinely different product with genuinely different risk — arbitrary
user uploads, storage cost, retention/privacy obligations. It ships behind
an explicit opt-in with default-short retention (auto-delete uploaded files
after a bounded window) and per-user quotas defined at the schema level
from the start, not retrofitted after M6 ships.

## PWA / offline support — historical note

An earlier browser-only prototype (Next.js + Canvas + ffmpeg.wasm) explored
PWA offline caching and hit unresolved service-worker issues; that
prototype's code has been fully replaced by this Rust-core rewrite. The
WASM surface (M4) starts fresh against `sizer-core` rather than carrying
forward the old approach.
