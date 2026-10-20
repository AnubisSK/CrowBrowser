# Project: Lightweight Cross-Platform Browser (macOS & Linux)
Targets:
- macOS: Monterey (12.x+), Intel Core i5 (Broadwell) / Apple Silicon, 4GB RAM minimum.
- Linux: Modern distributions (Arch, Debian/Ubuntu, Fedora) via WebKitGTK, X11 & Wayland.
Goal: Extreme memory efficiency, zero bloat, modern web compatibility, zero-compromise security.

## Architecture & Core Decisions
- Engine: `wry` + `tao`
  - macOS: WebKit FFI via system WebKit framework (`nonPersistentDataStore`).
  - Linux: WebKitGTK FFI (`webkit2gtk-4.1` / `javascriptcoregtk-4.1` cez `WebContext::new_ephemeral()`).
  - DO NOT build an HTML/JS engine from scratch.
- UI Layer: Native `wry` WebView overlay or lightweight native control bar. No Electron, no Chromium/CEF.
- Allocator: `mimalloc` or `jemalloc` (enforce via `Cargo.toml` to prevent heap fragmentation across long sessions).
- Memory Constraint: Strict budget. Sub-300MB baseline RAM. Aggressively unload hidden tabs, freeze background WebViews, and drop DOM caches where possible.
- Process Model (Linux): Manage WebKitGTK process memory models (`WEBKIT_NETWORK_CACHE_DIR`, process count limits).

## Security & Reliability Mandates (Audit Compliance)
- File Downloads & Execution:
  - DO NOT directly execute files via `open` or `xdg-open`.
  - Finished downloads must only trigger "reveal in file manager" (`open -R <path>` on macOS, D-Bus `ShowItems` or `xdg-open <parent_dir>` on Linux).
  - Prevent Drive-By Downloads: Downloads must require an explicit user confirmation dialog before saving to disk.
- URL Navigation & Spoofing Defense:
  - Address bar URL must NEVER be updated on `PageLoadEvent::Started`. Update the visible URL strictly on `Committed` / `Finished` events to prevent Address Bar Spoofing.
  - Implement visual SSL/TLS state in `toolbar.html` (secure lock icon for HTTPS, explicit warning indicator for plaintext HTTP).
  - URL resolver (`to_url`) must default `localhost`, `127.0.0.1`, and `.local` domains to `http://`.
  - Maintain explicit scheme bypass for valid protocols (`data:`, `blob:`, `mailto:`, `about:`, `file:`) so they are not redirected to search.
- Vault & Password Integrity:
  - Password change (`change-pw`) must be atomic and transactional: write the new vault to a `.vault.tmp` file and rename to the target file. Only update in-memory state and return `pwResult(true)` AFTER successful disk persistence.
  - Zeroization: Sensitive keys, master passwords, and intermediate salt buffers must use `zeroize::Zeroizing`.
- IPC & Cookie Restoration:
  - Ephemeral WebViews only: Tab webviews must never have `with_ipc_handler` attached. `window.ipc` is restricted solely to internal chrome (toolbar, panel, lock).
  - DOM XSS Protection: UI scripts must strictly use `textContent` / `document.createElement`, never `innerHTML`.
  - Cookie restoration (`CookiesRestored`) must include a strict timeout fallback (max 3 seconds) to prevent infinite UI freeze if WebKit callbacks stall.

## Token Saving & Output Rules
- Be concise. Return only working code diffs or new files; skip conversational filler.
- Do not repeat explanations or write unit tests unless explicitly asked.
- Prefer editing existing functions over regenerating whole files.
- Platform separation: Maintain clean boundaries using `#[cfg(target_os = "macos")]` and `#[cfg(target_os = "linux")]`.

## Tech Stack & Dependencies
- Lang: Rust (Edition 2024 / latest stable)
- Crates: `tao` (windowing), `wry` (webview bindings), `mimalloc`, `tokio` (runtime), `zeroize`, `argon2`, `chacha20poly1305`
- Networking/Filters: Network-level domain blocking for ads/telemetry before navigation (`wry::RequestAsync` / custom protocol handler).

### Linux System Packages
- Arch / CachyOS: `sudo pacman -S webkit2gtk-4.1 gtk3 pkg-config libsoup3`
- Debian / Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config libsoup-3.0-dev build-essential`
- Fedora: `sudo dnf install webkit2gtk4.1-devel gtk3-devel pkgconf libsoup3-devel`

## Dev Commands
- Build: `cargo build`
- Run: `cargo run`
- Check: `cargo check`
- Release: `cargo build --release`
- Linux (force X11 / Wayland backend if debugging):
  - Wayland: `GDK_BACKEND=wayland cargo run`
  - X11: `GDK_BACKEND=x11 cargo run`
