<p align="center">
  <img src="assets/logo-1024.png" width="160" alt="Crow logo">
</p>

<h1 align="center">CrowBrowser</h1>

<p align="center">
  <b>Ľahký, súkromný a šifrovaný webový prehliadač pre macOS a Linux, napísaný v Ruste.</b><br>
  <i>A lightweight, private, and encrypted web browser for macOS and Linux, written in Rust.</i><br>
  Postavený na systémovom WebKite (na Linuxe WebKitGTK). Žiadny Chromium ani Electron.<br>
  Built on system WebKit (WebKitGTK on Linux). Zero Chromium, zero Electron.
</p>

<p align="center">
  <a href="#slovenčina">🇸🇰 Slovenčina</a> • <a href="#english">🇬🇧 English</a>
</p>

---

<a name="slovenčina"></a>
## 🇸🇰 Slovenčina

### Prečo CrowBrowser

- **Ľahký:** Hlavný proces zaberá okolo 35 MB RAM a binárka má menej ako 2 MB. Neaktívne karty sa automaticky uspávajú.
- **Šifrovaný:** História, záložky, cookies, heslá, relácie a nastavenia sú na disku chránené heslom (Argon2id + XChaCha20-Poly1305). Bez hlavného hesla nikto údaje neprečíta.
- **Blokovanie reklám:** Reklamy a sledovacie skripty sú blokované na úrovni sieťovej vrstvy WebKitu pred odoslaním požiadavky.
- **Natívny a kompatibilný:** Využíva WebKit z macOS (rovnaký engine ako Safari) alebo WebKitGTK na Linuxe. Moderné webové štandardy fungujú spoľahlivo.
- **Nulový zápis na disk (Ephemeral):** Všetky dáta relácie žijú len v RAM a ukladajú sa výhradne do šifrovaného trezora.

### Čo je nové vo verzii 0.1.2

- **Univerzálna podpora architektúr (Apple Silicon + Intel):**
  - Natívna podpora pre `aarch64-apple-darwin` (M1/M2/M3/M4) aj `x86_64-apple-darwin` (Intel).
  - Skripty na generovanie **Universal 2 fat binary** DMG inštalátorov.
- **Odolnosť voči pádom a OOM (SEC-02):**
  - Detekcia ukončenia renderovacieho procesu WebKitu (`webViewWebContentProcessDidTerminate:` na macOS, signál `web-process-terminated` na Linuxe).
  - Pri preťažení pamäte, páde WebGL alebo nekonečnej slučke tab nezamrzne – zobrazí sa bezpečnostná stránka s možnosťou obnovy karty.
- **Bezpečnostné spevnenie (Security Hardening):**
  - **SEC-01:** Navigačný filter v internom UI blokuje akúkoľvek externú navigáciu a chráni privilegovaný most `window.ipc`.
  - **SEC-04:** Čítanie favikony cez `eval_isolated` v izolovanom JavaScript svete WebKitu bez znečistenia globálneho `window` (ochrana pred fingerprintingom).
  - **SEC-05:** Striktný zákaz prístupu z lokálnych súborov (`file://`) na iné súbory či domény (`allowFileAccessFromFileURLs` vypnuté).
  - **SEC-06:** Doplnená direktíva `frame-ancestors 'none'` v CSP pre interné offline stránky.
- **Predvolený prehliadač:** Integrovaná možnosť nastaviť CrowBrowser ako predvolený systémový prehliadač na macOS aj Linuxe.
- **Plynulejšie ovládanie:** Ošetrené gestá ťahania okna myšou a preusporiadania kariet bez race conditions.

### ⚠️ Upozornenie o vývoji cez AI a zodpovednosti

> [!IMPORTANT]
> **Zodpovednosť používateľa a AI vývoj:**
> Tento softvér je do veľkej miery vyvíjaný a písaný pomocou **umelej inteligencie (AI asistentov)** pod dohľadom tvorcu. 
> Softvér je poskytovaný **„tak, ako je“ (AS IS)**, bez akejkoľvek záruky na bezchybnosť, dostupnosť alebo vhodnosť na konkrétny účel.
> **Každý používateľ berie plnú osobnú zodpovednosť** za používanie prehliadača, navštevovanie webových stránok, lokálne nastavenia, bezpečnosť svojho hlavného hesla a uchovávané dáta.

### Inštalácia

1. Stiahni `dist/Crow-<verzia>-universal.dmg` (alebo verziu pre konkrétnu architektúru).
2. Presuň ikonu **Crow** do priečinka **Aplikácie** (`/Applications`).
3. Pri prvom spustení si nastav bezpečné heslo trezora (minimálne 8 znakov).

> *Poznámka k macOS:* Ak bol DMG stiahnutý z internetu, systém môže zobraziť hlásenie Gatekeepera. Klikni na aplikáciu pravým tlačidlom myši a zvoľ **Otvoriť**.

### Vývoj a kompilácia

```sh
cargo run                                 # spustenie vývojovej verzie
cargo build --release                     # optimalizovaný release build
./scripts/build_macos.sh                  # vytvorenie Universal 2 binárky (ARM64 + Intel)
./scripts/make_dmg.sh                     # zabalenie DMG s Universal 2 binárkou
./scripts/make_dmg.sh arm64               # zabalenie DMG pre Apple Silicon
./scripts/make_dmg.sh x86_64              # zabalenie DMG pre Intel
```

Linux: `./scripts/install_linux.sh` (podrobnosti a systémové balíčky v [docs/BUILD.md](docs/BUILD.md#linux-webkitgtk)).

---

<a name="english"></a>
## 🇬🇧 English

### Why CrowBrowser

- **Lightweight:** Baseline memory footprint around 35 MB RAM with a sub-2 MB standalone executable. Inactive tabs are automatically suspended.
- **Encrypted:** Browsing history, bookmarks, cookies, credentials, session tabs, and settings are encrypted at rest with Argon2id and XChaCha20-Poly1305.
- **Ad & Tracker Blocking:** Network-level ad and tracker filtering built directly into WebKit's request pipeline before connections are made.
- **Native & Compatible:** Powered by macOS WebKit (the same engine as Safari) or WebKitGTK on Linux. Full compatibility with modern web standards.
- **Zero-Disk Ephemeral Storage:** Normal browsing state resides strictly in memory and only persists through the encrypted vault.

### What's New in Version 0.1.2

- **Universal Multi-Architecture Support (Apple Silicon + Intel):**
  - Native compilation for `aarch64-apple-darwin` (Apple Silicon M1-M4) and `x86_64-apple-darwin` (Intel).
  - Automated build scripts producing **Universal 2 fat binary** bundles and DMGs.
- **Crash Resilience & WebProcess Recovery (SEC-02):**
  - Graceful handling of WebKit rendering engine crashes (`webViewWebContentProcessDidTerminate:` on macOS, `web-process-terminated` signal on Linux).
  - Tabs no longer freeze on Out-Of-Memory (OOM) or heavy WebGL/Canvas loads; a clean in-memory crash recovery page allows immediate reloading.
- **Architectural Security Hardening:**
  - **SEC-01:** Navigation guard on internal UI webviews completely disallows external navigation from privileged contexts with `window.ipc`.
  - **SEC-04:** Favicon fetching executed via `eval_isolated` in WebKit's isolated script world (`WKContentWorld` / `WebKitScriptWorld`), eliminating fingerprinting via `window.__crowIcon`.
  - **SEC-05:** Local file scheme hardening (`allowFileAccessFromFileURLs` and `allowUniversalAccessFromFileURLs` explicitly disabled).
  - **SEC-06:** Strict CSP headers featuring `frame-ancestors 'none'` on internal pages.
- **Default Browser Support:** Built-in capability to query and set CrowBrowser as the default web browser on both macOS and Linux.
- **Gesture & IPC Reliability:** Smooth window dragging and tab reordering without race conditions or extraneous system warnings.

### ⚠️ AI Development & Liability Disclaimer

> [!IMPORTANT]
> **User Responsibility & AI-Assisted Code Notice:**
> This software is largely designed, developed, and written with the assistance of **Artificial Intelligence (AI tools & LLMs)** under human supervision.
> The software is provided **"AS IS"**, without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, or noninfringement.
> **The user assumes full and sole responsibility** for using this browser, accessing any web content, maintaining the confidentiality of their master password, and ensuring the security of their data and computing environment.

### Installation

1. Download `dist/Crow-<version>-universal.dmg` (or your architecture-specific package).
2. Drag **Crow** into your **Applications** folder (`/Applications`).
3. On first launch, create a master vault password (minimum 8 characters).

> *macOS Gatekeeper note:* If downloaded directly via a web browser, right-click the application and select **Open** to bypass initial Gatekeeper prompts for ad-hoc signed binaries.

### Building from Source

```sh
cargo run                                 # debug build & run
cargo build --release                     # optimized release build
./scripts/build_macos.sh                  # build Universal 2 binary (ARM64 + Intel)
./scripts/make_dmg.sh                     # package Universal 2 DMG installer
./scripts/make_dmg.sh arm64               # package Apple Silicon DMG
./scripts/make_dmg.sh x86_64              # package Intel x86_64 DMG
```

Linux build: `./scripts/install_linux.sh` (see dependencies in [docs/BUILD.md](docs/BUILD.md#linux-webkitgtk)).

---

## Dokumentácia / Documentation

| Súbor / File | Popis (SK) | Description (EN) |
|---|---|---|
| [docs/FUNKCIE.md](docs/FUNKCIE.md) | Všetky funkcie, tlačidlá a nastavenia | Detailed features, controls, and configuration |
| [docs/SKRATKY.md](docs/SKRATKY.md) | Klávesové skratky | Keyboard shortcuts |
| [docs/BEZPECNOST.md](docs/BEZPECNOST.md) | Šifrovanie, trezor a bezpečnostný model | Encryption, vault architecture, and security model |
| [docs/ARCHITEKTURA.md](docs/ARCHITEKTURA.md) | Štruktúra kódu, moduly, tok dát | Codebase structure, modules, and data flow |
| [docs/BUILD.md](docs/BUILD.md) | Build, balenie do .app a .dmg, assety | Build process, packaging (.app / .dmg), and assets |
| [docs/OBMEDZENIA.md](docs/OBMEDZENIA.md) | Známe obmedzenia a plány do budúcna | Known limitations and roadmap |
| [penetracne testovanie/](penetracne%20testovanie/) | Bezpečnostné audity a penetračné správy | Security audits and penetration test reports |
