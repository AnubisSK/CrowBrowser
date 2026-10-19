# Architektúra

## Technológie

| Vrstva | Technológia |
|---|---|
| Jazyk | Rust, edícia 2024 |
| Okno, event loop | [`tao`](https://crates.io/crates/tao) 0.37 |
| Webový engine | [`wry`](https://crates.io/crates/wry) 0.57 → systémový **WKWebView** (WebKit z macOS) |
| Natívne menu | [`muda`](https://crates.io/crates/muda) 0.21 |
| Priamy prístup k WebKitu | `objc2`, `objc2-web-kit`, `objc2-foundation`, `objc2-app-kit`, `block2` |
| Alokátor | [`mimalloc`](https://crates.io/crates/mimalloc) (globálny, menej fragmentácie pamäte) |
| Šifrovanie | `argon2`, `chacha20poly1305`, `zeroize` |
| Serializácia | `serde`, `serde_json` |
| UI prehliadača | HTML/CSS/JS bez frameworkov, vložené do binárky cez `include_str!` |

Žiadny Chromium, CEF ani Electron: renderovanie zabezpečuje WebKit, ktorý je súčasťou macOS.

## Súbory

```
CrowBrowserMac/
├── Cargo.toml              závislosti a profily (dev: rýchla kompilácia, release: thin LTO)
├── .cargo/config.toml      jobs = 2 (build na 4 GB RAM)
├── src/
│   ├── main.rs             štart, natívne menu, ikona v Docku, event loop
│   ├── app.rs              jadro: stav, karty, príkazy, panely, ukladanie
│   ├── vault.rs            šifrovaný trezor a dátové štruktúry
│   ├── wk.rs               priamy WebKit: pamäťové úložisko, cookies, content blocker
│   ├── adblock.rs          generovanie pravidiel blokovania
│   ├── settings.rs         nastavenia, vyhľadávače, cesta k dátam
│   ├── toolbar.html        panel s kartami, adresou a hľadaním
│   ├── panel.html          Nastavenia / História / Obľúbené / Stiahnuté
│   └── lock.html           obrazovka vytvorenia a odomknutia trezora
├── assets/                 logo, ikony, pozadie DMG
├── scripts/                bundle.sh (.app), make_dmg.sh (.dmg)
├── dist/                   hotové inštalátory
└── docs/                   táto dokumentácia
```

## Rozloženie okna

Okno obsahuje niekoľko WebView ako potomkov (`build_as_child`):

```
┌──────────────────────────────────────────────────┐  y = 0
│ ● ● ●  [karta] [karta] [+]                       │  toolbar WebView
│ ‹ › ⟳ ⌂ [ adresa ...................☆] 🛡 🔖 🕘 ↓ ⚙ │  76 px (+36 px s hľadaním)
├──────────────────────────────────────────────────┤
│                                                  │
│   obsah: WebView aktívnej karty                  │  ostatné karty: skryté
│   (alebo panel WebView navrchu)                  │  alebo uspaté (None)
│                                                  │
└──────────────────────────────────────────────────┘
```

Keď je prehliadač zamknutý, existuje iba jeden WebView (`lock.html`) cez celé okno.

## Stav aplikácie (`app.rs`)

```
App
├── ctx: TabCtx            okno (Rc), EventLoopProxy, počítadlo ID kariet
├── cfg: Settings          aktuálne nastavenia (po zamknutí sa vrátia na predvolené)
├── lock: Option<WebView>  obrazovka odomknutia
└── s: Option<Session>     existuje len keď je odomknuté
    ├── vault: Vault           salt + kľúč (Zeroizing)
    ├── data: VaultData        história, obľúbené, sťahovania, cookies, relácia, nastavenia
    ├── store                  zdieľaný WKWebsiteDataStore (len v pamäti)
    ├── toolbar: WebView
    ├── tabs: Vec<Tab>         { id, url, title, view: Option<WebView>, hidden_since, loading }
    ├── panel: Option<WebView> vytvorí sa pri otvorení, pri zatvorení zanikne
    └── dirty / saving / ready príznaky ukladania
```

## Tok udalostí

Všetko beží na hlavnom vlákne v `tao` event loope. Callbacky WebKitu posielajú `UserEvent` cez `EventLoopProxy`:

| Udalosť | Zdroj |
|---|---|
| `Menu(id)` | `muda`: klik alebo skratka v menu |
| `Ipc(msg)` | `window.ipc.postMessage` z toolbar/panel/lock HTML |
| `PageLoad`, `Title` | navigácia a zmena názvu v karte |
| `NewTab`, `AdoptTab` | `target=_blank` / `window.open` |
| `DownloadStarted`, `DownloadDone` | sťahovanie |
| `FindResult` | výsledok JS hľadania na stránke |
| `Cookies(vec, After)` | dokončený export cookies → zápis trezora → nič / ukončiť / zamknúť |
| `CookiesRestored` | cookies vrátené do WebKitu → otvoria sa karty |
| `RulesReady` | skompilované pravidlá adblocku |

Menu aj IPC používajú rovnaké textové príkazy, napríklad `new-tab`, `close-tab:3`, `go:<text>`, `panel:history`, `set:theme=light`, `find:{"q":"…","dir":"next"}`, `lock`, `quit`.

**Časovač:** `ControlFlow::WaitUntil` sa ozve každých 30 s. Uspí neaktívne karty a uloží trezor, ak je `dirty`. Ak sa udalosti nedejú, proces nespotrebúva CPU.

## Životný cyklus

```
štart ─► lock.html ─► heslo ─► Argon2id ─► dešifrovanie ─► nastavenia, motív
                                                        │
                       import cookies do WebKitu ◄──────┘
                                   │
                         CookiesRestored ─► karty (načíta sa len aktívna)
                                   │
         … prehliadanie, autosave každých 30 s (export cookies → šifrovanie → zápis) …
                                   │
            ⌘Q ─► export cookies ─► uloženie ─► koniec
           ⌃⌘L ─► export cookies ─► uloženie ─► vymazanie WebKit úložiska ─► lock.html
```

## Pamäť

- **mimalloc** ako globálny alokátor.
- Neaktívne karty sa po N minútach uspia: `WebView` sa zruší a s ním aj proces WebContent danej karty.
- Pri obnovení relácie sa načíta len aktívna karta.
- Panel (nastavenia, história…) sa vytvorí pri otvorení a pri zatvorení sa zruší.
- Pravidlá blokovania sú skompilované WebKitom (bytecode) a zdieľané všetkými kartami.
- Typická spotreba: hlavný proces okolo 35 MB a k tomu WebKit proces pre každú načítanú kartu.

## Vyskakovacie okná (OAuth)

`with_new_window_req_handler` vytvorí nový `WebView` s konfiguráciou od WebKitu (`opener.target_configuration`) a vráti ho ako `NewWindowResponse::Create`. Nové okno tak má platný `window.opener`. Do zoznamu kariet sa pridá cez `PENDING` a udalosť `AdoptTab`.

## Hľadanie na stránke

Spustí sa JavaScript vo WebView karty: `window.find()` pre označenie a posun a počítanie výskytov v `document.body.innerText`. Výsledok sa vráti cez `evaluate_script_with_callback` a zobrazí sa v paneli.
