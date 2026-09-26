# Build and contribute

**English** | [Español](../es/development.md) | [Home](../../README.md)

This guide covers the Windows x64 development and release toolchain.

## Set up tools

Install Rust through rustup, the MSVC C++ build tools, Bun **1.4.2**, PowerShell
**7**, and WebView2 Runtime. `rust-toolchain.toml` pins Rust **1.94.1**.
Install Dioxus CLI **0.7.9** from its
[official release](https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.9),
or compile it with:

```powershell
cargo install dioxus-cli --version 0.7.9 --locked
```

## Run and build

From the repository root:

```powershell
bun install --frozen-lockfile
bun run tailwind:build
dx serve --platform desktop
```

Pass runtime options after `--`, for example:

```powershell
dx serve --platform desktop -- --binary-dir "C:\Tools\upscalers" --gpu-id 0
dx build --platform desktop --release --locked
```

The release application is in `target/dx/miko-kagura/release/windows/app/`.
Keep its executable and assets together. Dioxus collects `asset!()` resources;
plain `cargo run` does not produce the equivalent runnable layout.

## Run checks

```powershell
cargo test --locked -j 2
bun run docs:check
```

For complete distribution packages, follow [Releasing](releasing.md). GPU
inference needs a Vulkan-capable device; successful unit tests alone do not
verify the installed engine/driver combination.

## Architecture

The axum server runs on a dedicated thread with a Tokio runtime. The Dioxus
window owns the main thread. Both share `Arc<AppState>` and use a Tokio
`broadcast` channel of `ServerEvent` messages to update the dashboard.
NCNN executables perform inference in subprocesses.

| Source | Responsibility |
| --- | --- |
| `src/config.rs` | Server defaults and supported model registry |
| `src/upscaler.rs` | Engine discovery, subprocesses and GPU detection |
| `src/server.rs` | HTTP routes, request processing and shared state |
| `src/settings.rs` | JSON preferences |
| `src/history.rs`, `src/cache.rs` | Comparisons, captures and temporary storage |
| `src/clamp.rs` | Output dimension calculation |
| `src/plugins.rs` | Source image repair |
| `src/ui/`, `src/main.rs` | Dashboard, CLI and startup |
| `scripts/`, `packaging/` | Release automation and installer template |

## Maintain both documentation languages

Update the English and Spanish files together. Keep commands, model keys,
filenames, and UI labels unchanged when translating instructions. Each page
must link to its language counterpart and the corresponding README.

Use one purpose per guide, prerequisites before numbered steps, short active
sentences, and descriptive links. Keep the READMEs focused on downloads and
getting started. `bun run docs:check` checks page parity, heading structure,
language links and local link targets; review translation accuracy manually.

These conventions follow
[GitHub Docs best practices](https://docs.github.com/en/contributing/writing-for-github-docs/best-practices-for-github-docs)
and [writing for translation](https://docs.github.com/en/contributing/writing-for-github-docs/writing-content-to-be-translated).
