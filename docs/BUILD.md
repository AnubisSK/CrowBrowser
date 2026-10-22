# Build a distribúcia

## Požiadavky

- macOS 12 Monterey alebo novší
- Rust stable (`rustup`), edícia 2024
- Xcode Command Line Tools (`xcode-select --install`): `codesign`, `SetFile`, `iconutil`

## Príkazy

| Príkaz | Výsledok | Čas* |
|---|---|---|
| `cargo check` | kontrola kódu | sekundy |
| `cargo run` | debug build a spustenie (ikona v Docku sa nastaví za behu) | ~4 min prvýkrát, potom sekundy |
| `cargo build --release` | `target/release/crow-browser` (~1,8 MB) | ~5–6 min |
| `./scripts/bundle.sh` | `target/Crow.app` s ikonou, Info.plist a ad-hoc podpisom | ~6 min |
| `./scripts/make_dmg.sh [arm64\|x86_64\|universal\|all]` | `dist/Crow-<verzia>-<arch>.dmg` (inštalátor), predvolene `universal` | ~6 min na architektúru |
| `./scripts/make_dmg.sh <arch> --skip-build` | len znova zabalí existujúce binárky z `target/<triple>/release` do DMG | ~20 s na DMG |

\* na MacBooku Air 2015 (i5 Broadwell, 4 GB RAM)

## Profily (`Cargo.toml`)

| Profil | Nastavenie | Prečo |
|---|---|---|
| `dev` | `opt-level = 0`, `debug = "line-tables-only"`, `incremental`, bez LTO | rýchla kompilácia, málo RAM |
| `dev` závislosti | `opt-level = 1`, bez debug info | použiteľná rýchlosť WebKit väzieb |
| `dev` argon2 / blake2 | `opt-level = 3` | neoptimalizovaný Argon2 by odomykal desiatky sekúnd |
| `release` | `opt-level = 3`, `lto = "thin"`, `codegen-units = 4`, `panic = "abort"`, `strip` | malá a rýchla binárka pri rozumnej spotrebe RAM počas buildu |

`.cargo/config.toml` obmedzuje paralelné kompilácie na `jobs = 2`, aby build na 4 GB RAM nezačal swapovať.

## `scripts/bundle.sh`

1. `MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release`
2. Vytvorí `target/Crow.app/Contents/{MacOS,Resources}`.
3. Skopíruje binárku ako `Crow` a ikonu `Crow.icns`.
4. Vygeneruje `Info.plist`: `com.crow.browser`, verzia z `Cargo.toml`, `LSMinimumSystemVersion 12.0`, Retina.
5. Aplikáciu podpíše ad-hoc podpisom (`codesign --sign -`).

## `scripts/make_dmg.sh`

Architektúra ako prvý argument: `arm64` (Apple Silicon), `x86_64` (Intel), `universal` (predvolené, `lipo` z oboch) alebo `all` (všetky tri DMG z jednej dvojice buildov). Updater si z release vyberie DMG podľa názvu: `arm64` / `x86_64` len pre danú architektúru, `universal` pre obe.

1. (okrem `--skip-build`) `MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --target <aarch64|x86_64>-apple-darwin`, pri `universal` spojí binárky cez `lipo`
2. spustí `bundle.sh <binárka>`
3. pripraví obsah: `Crow.app`, odkaz na `/Applications`, `.background/background.tiff`
4. vytvorí zapisovateľný obraz (HFS+) a pripojí ho
5. cez AppleScript vo Finderi nastaví okno: ikony 112 px, okno 660 × 400, bez panela nástrojov, pozadie, pozície ikon (Crow vľavo, Aplikácie vpravo). Uloží sa do `.DS_Store`.
6. pridá `.VolumeIcon.icns` a nastaví príznak vlastnej ikony. Robí sa to až po kroku s Finderom, pretože Finder pri prepise `.DS_Store` ikonu zmaže.
7. odpojí obraz a skomprimuje ho (UDZO, zlib-9) do `dist/`
8. nastaví ikonu samotného `.dmg` súboru (platí len lokálne)

> Pri prvom spustení môže macOS požiadať o povolenie, aby Terminál ovládal Finder. Bez neho DMG funguje, ale okno nebude mať pozadie ani rozloženie.

## Assety (`assets/`)

| Súbor | Popis |
|---|---|
| `logo.svg` | zdroj loga (vrana pred zemeguľou, macOS squircle 1024 × 1024) |
| `logo-1024.png` | PNG s priehľadnými rohmi |
| `icon-256.png` | ikona vložená do binárky (Dock pri `cargo run`) |
| `Crow.icns` | ikona aplikácie, všetky veľkosti 16–1024 px vrátane @2x |
| `dmg-background.svg` | zdroj bannera DMG (660 × 400) |
| `dmg-background.tiff` | banner 1× a 2× v jednom súbore (HiDPI) |

### Obnovenie assetov po úprave SVG

Logo sa renderuje cez QuickLook (`qlmanage`). QuickLook vypĺňa pozadie bielou, preto sa priehľadnosť počíta z dvoch renderov (na bielom a na čiernom pozadí). Ikony sa potom zmenšia cez `sips` a spoja cez `iconutil -c icns`. Banner sa renderuje ako štvorec, oreže na 1320 × 800, zmenší na 660 × 400 a obe verzie sa spoja cez `tiffutil -cathidpicheck`.

## Podpis a notarizácia

DMG je podpísaný len **ad-hoc**. Na vlastnom Macu to stačí. Na iných Macoch sa po stiahnutí zobrazí upozornenie Gatekeepera a aplikáciu treba prvýkrát otvoriť cez pravý klik a voľbu **Otvoriť**. Distribúcia bez upozornenia vyžaduje Apple Developer ID (99 USD/rok): `codesign` s certifikátom Developer ID, `xcrun notarytool submit` a `xcrun stapler staple`.

## Architektúra procesora

Build je pre **x86_64** (Intel), keďže cieľom je MacBook Air 2015. Na Apple Silicon beží cez Rosettu 2. Univerzálnu binárku vytvoríš takto:

```sh
rustup target add aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
lipo -create -output Crow \
  target/x86_64-apple-darwin/release/crow-browser \
  target/aarch64-apple-darwin/release/crow-browser
```

## Linux (WebKitGTK)

Rovnaký kód, platformovo závislá vrstva je v `src/wk.rs` (macOS) a `src/wk_gtk.rs` (Linux, rovnaké API). Vyberá sa cez `#[cfg_attr(target_os = "linux", path = "wk_gtk.rs")]` v `main.rs`.

Balíky:

```sh
sudo pacman -S webkit2gtk-4.1 gtk3 pkg-config libsoup3                                  # Arch / CachyOS
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config libsoup-3.0-dev build-essential  # Debian / Ubuntu
sudo dnf install webkit2gtk4.1-devel gtk3-devel pkgconf libsoup3-devel                  # Fedora
```

| Príkaz | Výsledok |
|---|---|
| `cargo run` | debug spustenie (Wayland aj X11, `GDK_BACKEND=x11` vynúti X11) |
| `./scripts/install_linux.sh` | release build do `~/.local/bin/crow-browser`, `.desktop` súbor a ikona (`com.crow.browser`) |
| `./scripts/install_linux.sh --uninstall` | odstráni binárku, `.desktop` aj ikonu (trezor zostane) |
| `CROW_DECORATIONS=1 crow-browser` | natívny titulok okna namiesto kreslených „semaforov“ |
| `CROW_DEV=1 cargo run` | samostatná vývojová inštancia (`app_id` `com.crow.browser.dev`). GTK inak pustí len jednu inštanciu a druhé spustenie iba aktivuje prvé okno |

Rozdiely oproti macOS:

| | macOS | Linux |
|---|---|---|
| WebKit | WKWebView | WebKitGTK 4.1 (wry `build_gtk` do `GtkLayout`) |
| Úložisko | `WKWebsiteDataStore.nonPersistent` | ephemeral `WebKitWebContext` (nič sa nezapisuje na disk), `CacheModel::DocumentViewer` |
| Adblock | `WKContentRuleList` | `WebKitUserContentFilter`, rovnaký JSON; skompilované pravidlá v `~/.cache/crow/filters` |
| Okno | priehľadný titulok, systémové semafory | bez rámu, semafory kreslí `toolbar.html` / `lock.html` |
| Menu a skratky | `muda` (systémové menu) | `key-press-event` na GTK okne (`wk_gtk::shortcut`) |
| Dáta | `~/Library/Application Support/Crow` | `$XDG_DATA_HOME/crow` (`~/.local/share/crow`) |
| Stiahnuté | wry handler; po potvrdení sa URL znova vyžiada (wry handler je synchrónny) | `decide-policy` spustí sťahovanie cez `download_uri` (WebKitGTK inak nevyšle `download-started`), cieľ sa nastaví až po potvrdení, `$XDG_DOWNLOAD_DIR` |
| Certifikát (info o stránke) | `WKWebView.serverTrust` → Security.framework | `webkit_web_view_get_tls_info` → `GTlsCertificate`; DER parsuje `src/cert.rs` |
| Zobraziť súbor | `open -R` | D-Bus `org.freedesktop.FileManager1.ShowItems`, inak `xdg-open` priečinka |

Trezor má rovnaký formát na oboch systémoch, `vault.bin` sa dá preniesť.

Kontrola macOS kódu z Linuxu (bez SDK, len typová kontrola): `rustup target add x86_64-apple-darwin` a `cargo check --target x86_64-apple-darwin` s `CC_x86_64_apple_darwin` nastaveným na prázdny kompilátor (objc2 kompiluje malý `.m` súbor).
