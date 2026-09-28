# GPUI Web

`gpui_web` renders GPUI windows into browser canvases through WebGPU. Run pages
on localhost or HTTPS in a browser with WebGPU enabled. WebGL is not enabled by
the default dependency configuration.

## Build and run

The default `multithreaded` feature depends on `wasm_thread` and requires nightly:

```sh
rustup toolchain install nightly --target wasm32-unknown-unknown
cargo +nightly check -p gpui_web --target wasm32-unknown-unknown --locked
```

The single-threaded backend can be checked with stable Rust:

```sh
rustup target add wasm32-unknown-unknown
cargo check -p gpui_web --no-default-features --target wasm32-unknown-unknown --locked
```

The `hello_web` example uses Trunk. From this directory:

```sh
cargo install trunk --locked
cd examples/hello_web
trunk serve
```

`Application::run` retains the application for the browser page's lifetime.
Embedders that manage application lifetime themselves can use `run_embedded`
and retain the returned `ApplicationHandle`.

## Execution and platform limits

Enabling `multithreaded` alone does not create shared WebAssembly memory.
Workers require an atomics-enabled shared-memory build and cross-origin
isolation (`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`). The dispatcher falls back to the
browser main thread when shared memory is unavailable; CPU-heavy background
tasks can then delay rendering. The example's Trunk configuration supplies the
headers. Its `.cargo/config.toml` rebuilds the standard library with atomics,
imports shared memory, and exports the symbols needed by wasm-bindgen's thread
initialization. Run Trunk from `examples/hello_web` so Cargo reads that local
configuration; passing only `--manifest-path` from the repository root does not
load it.

`WgpuOffscreenRenderer` provides synchronous native GPU readback and is not
exported on Web. Browser readback must yield to the browser event loop.
Device-local geometry payloads in GPUI scene frames and viewport GPU pick
publication are unsupported on Web and report errors. Native rendering APIs
retain their existing behavior.

File dialogs, clipboard integration, native screen picking, and browser media
playback are not implemented by this backend. Browser rendering does not imply
parity with every desktop platform integration.
