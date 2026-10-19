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
| `./scripts/make_dmg.sh` | `dist/Crow-<verzia>.dmg` (inštalátor) | ~6 min |
| `./scripts/make_dmg.sh --skip-build` | len znova zabalí existujúcu `.app` do DMG | ~20 s |

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

1. (voliteľne) spustí `bundle.sh`
2. pripraví obsah: `Crow.app`, odkaz na `/Applications`, `.background/background.tiff`
3. vytvorí zapisovateľný obraz (HFS+) a pripojí ho
4. cez AppleScript vo Finderi nastaví okno: ikony 112 px, okno 660 × 400, bez panela nástrojov, pozadie, pozície ikon (Crow vľavo, Aplikácie vpravo). Uloží sa do `.DS_Store`.
5. pridá `.VolumeIcon.icns` a nastaví príznak vlastnej ikony. Robí sa to až po kroku s Finderom, pretože Finder pri prepise `.DS_Store` ikonu zmaže.
6. odpojí obraz a skomprimuje ho (UDZO, zlib-9) do `dist/`
7. nastaví ikonu samotného `.dmg` súboru (platí len lokálne)

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
