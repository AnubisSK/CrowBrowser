# Project: Lightweight macOS Browser (Rust)
Target: macOS Monterey (12.x), Intel Core i5 (Broadwell), 4GB RAM.
Goal: Extreme memory efficiency, zero bloat, 100% web compatibility in 2026.

## Architecture & Core Decisions
- Engine: `wry` + `tao` (WebKit FFI via macOS WebKit framework). DO NOT build an HTML/JS engine from scratch.
- UI Layer: Native `wry` WebView overlay or lightweight `iced` control bar. No Electron, no Chromium/CEF.
- Allocator: `mimalloc` or `jemalloc` (enforce via Cargo.toml to prevent memory fragmentation).
- Memory Constraint: Strict budget. Max 300MB baseline RAM. Aggressively unload hidden tabs.

## Token Saving & Output Rules
- Be concise. Return only working code diffs or new files; skip conversational filler.
- Do not repeat explanations or write unit tests unless explicitly asked.
- Prefer editing existing functions over regenerating whole files.

## Tech Stack
- Lang: Rust (Edition 2024 / latest stable)
- Crates: `tao` (windowing), `wry` (webview), `mimalloc`, `tokio` (runtime)
- Networking/Filters: Network-level domain blocking for ads/telemetry before navigation.

## Dev Commands
- Build: `cargo build`
- Run: `cargo run`
- Check: `cargo check`
- Release: `cargo build --release`
