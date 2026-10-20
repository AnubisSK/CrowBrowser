# Funkcie

Prehľad všetkého, čo Crow vie (verzia 0.1.0).

## Obsah

- [Obrazovka odomknutia](#obrazovka-odomknutia)
- [Okno a panel nástrojov](#okno-a-panel-nástrojov)
- [Karty](#karty)
- [Adresný riadok a vyhľadávanie](#adresný-riadok-a-vyhľadávanie)
- [Hľadanie na stránke](#hľadanie-na-stránke)
- [Obľúbené](#obľúbené)
- [História](#história)
- [Sťahovanie súborov](#sťahovanie-súborov)
- [Blokovanie reklám a sledovačov](#blokovanie-reklám-a-sledovačov)
- [Priblíženie](#priblíženie)
- [Nastavenia](#nastavenia)
- [Ukladanie a obnovenie relácie](#ukladanie-a-obnovenie-relácie)
- [Menu aplikácie](#menu-aplikácie)

---

## Obrazovka odomknutia

Pri každom spustení sa Crow otvorí zamknutý.

| Situácia | Čo uvidíš |
|---|---|
| Prvé spustenie | „Vitaj v Crow“: zadáš nové heslo dvakrát (min. 8 znakov) a vytvorí sa šifrovaný trezor |
| Ďalšie spustenia | „Crow je zamknutý“: zadáš heslo |
| Nesprávne heslo | Chybové hlásenie a zatrasenie formulára, môžeš skúsiť znova |
| Zabudnuté heslo | Odkaz **„Zabudol som heslo – vymazať všetky dáta“** (treba kliknúť dvakrát). Trezor sa zmaže a začneš odznova |

- Kým je prehliadač zamknutý, riadi sa systémovým motívom, aby nebolo vidieť ani tvoj zvolený motív.
- Zamknúť sa dá kedykoľvek: **⌃⌘L** alebo **Nastavenia → Šifrovanie → Zamknúť teraz**.

## Okno a panel nástrojov

Okno má priehľadnú macOS lištu, takže semafor (zavrieť/minimalizovať/zväčšiť) sedí priamo v riadku kariet.

**Riadok kariet:** karty, tlačidlo **+** (nová karta). Za prázdne miesto sa dá okno ťahať, dvojklik naň otvorí novú kartu.

**Navigačný riadok** (zľava doprava):

| Tlačidlo | Funkcia |
|---|---|
| ‹ | Späť |
| › | Vpred |
| ⟳ / ✕ | Obnoviť stránku; počas načítavania sa zmení na Zastaviť |
| ⌂ | Domovská stránka |
| Adresný riadok | URL alebo vyhľadávanie. Adresa sa zmení až po dokončení načítania (ochrana pred podvrhnutím adresy) |
| 🔒 / ⚠ Nezabezpečené (v adresnom riadku) | HTTPS / nešifrované HTTP. Klik otvorí **Informácie o stránke**: stav pripojenia, certifikát (vydaný pre, vydavateľ, platnosť, SHA-256, reťazec), cookies stránky s tlačidlom **Vymazať**, stav blokovania reklám |
| ☆ (v adresnom riadku) | Pridať / odobrať z obľúbených (žltá = uložené) |
| 🛡 | Stav blokovania reklám (vyblednutý = vypnuté), klik otvorí nastavenia |
| 🔖 | Obľúbené |
| 🕘 | História |
| ↓ | Stiahnuté (modrá bodka = práve sa sťahuje) |
| ⚙ | Nastavenia |

- Pod panelom beží modrý pásik, kým sa stránka načítava.
- Krátke hlásenia (napr. „Pridané do obľúbených“, „Stiahnuté: súbor.pdf“) sa ukážu vpravo hore.
- Panel sa prispôsobuje tmavému aj svetlému motívu.

## Karty

| Funkcia | Ako |
|---|---|
| Nová karta | **⌘T**, tlačidlo **+**, dvojklik na prázdne miesto v riadku kariet. Otvorí domovskú stránku |
| Zavrieť kartu | **⌘W**, **×** na karte, klik stredným tlačidlom myši |
| Znova otvoriť zatvorenú | **⌘⇧T** (pamätá si všetky zatvorené karty v relácii) |
| Prepnúť | Klik, **⌘⇧]** / **⌘⇧[**, **⌘1 … ⌘8**, **⌘9** = posledná |
| Odkazy do nového okna | `target="_blank"` a `window.open` sa otvoria ako nová karta |

- **Uspávanie kariet:** skryté karty sa po nastavenom čase (predvolene 5 min) uvoľnia z pamäte. Uspatá karta má vyblednutý názov a po kliknutí sa znova načíta.
- **Úsporné spustenie:** pri obnovení relácie sa načíta len aktívna karta, ostatné až po kliknutí.
- **Vyskakovacie okná na prihlásenie** („Prihlásiť cez Google/Apple/Facebook“) si zachovajú prepojenie na pôvodnú stránku (`window.opener`), takže prihlásenie funguje.
- Zavretím poslednej karty sa otvorí nová s domovskou stránkou, aplikácia sa nezavrie.
- Počas načítavania sa namiesto bodky na karte točí indikátor.

## Adresný riadok a vyhľadávanie

| Zadáš | Výsledok |
|---|---|
| `https://…`, `http://…`, `file:…`, `about:…`, `data:…`, `blob:…`, `mailto:…` | Otvorí sa presne táto adresa |
| `example.sk`, `sub.domena.com/cesta` | Doplní sa `https://` |
| `localhost:3000`, `127.0.0.1`, `[::1]`, `tlaciaren.local` | Doplní sa `http://` (lokálne servery bez TLS) |
| Text s medzerou alebo bez bodky | Vyhľadá sa predvoleným vyhľadávačom |

- **⌘L** skočí do adresného riadku a označí text.
- **Enter** potvrdí, **Esc** vráti pôvodnú adresu.

Podporované vyhľadávače: **DuckDuckGo** (predvolený), **Google**, **Bing**, **Brave Search**, **Startpage**, **Ecosia**.

## Hľadanie na stránke

- **⌘F** otvorí lištu hľadania pod panelom.
- Hľadá sa priebežne počas písania, bez ohľadu na veľké a malé písmená.
- Zobrazuje počet výsledkov („1 výsledok“, „3 výsledky“, „12 výsledkov“, „Žiadne výsledky“).
- **Enter** / **⌘G** = ďalší, **⇧Enter** / **⌘⇧G** = predchádzajúci, ↑ ↓ tlačidlá.
- **Esc** alebo **✕** lištu zavrie a zruší označenie.
- Vyhľadávanie dookola: po poslednom výsledku pokračuje od začiatku.

## Obľúbené

- **⌘D** alebo hviezdička v adresnom riadku pridá alebo odoberie aktuálnu stránku.
- **⌘⌥B** alebo tlačidlo 🔖 otvorí zoznam.
- Klik otvorí stránku v aktuálnej karte, **⌘+klik** v novej karte.
- **×** pri položke ju odstráni.
- Obľúbené sú uložené v šifrovanom trezore.

## História

- **⌘Y** alebo tlačidlo 🕘.
- Zoskupená podľa dní („Dnes“, „Včera“, dátum) s časom návštevy.
- **Vyhľadávanie** v názvoch aj adresách.
- Klik otvorí stránku, **⌘+klik** v novej karte, **×** odstráni jednu položku.
- **Vymazať všetko** (potvrdzuje sa druhým klikom).
- Ukladá sa až 20 000 posledných návštev (staršie sa automaticky mažú). Panel zobrazuje posledných 3 000, z nich vypíše najviac 1 000 výsledkov.
- Rovnaká adresa po sebe sa nezapisuje dvakrát.
- História je uložená v šifrovanom trezore.

## Sťahovanie súborov

- Pred každým sťahovaním sa pod adresným riadkom zobrazí výzva **„Stiahnuť „súbor“ z domény?“** s tlačidlami **Uložiť / Zrušiť**. Bez potvrdenia sa nič nezapíše na disk.
- Súbory sa ukladajú do **~/Downloads**. Ak už súbor s rovnakým názvom existuje, pridá sa číslo, napr. `dokument (1).pdf`.
- Pri štarte a dokončení sa zobrazí hlásenie, tlačidlo ↓ má počas sťahovania modrú bodku.
- **⌘⌥L** alebo tlačidlo ↓ otvorí zoznam so stavmi *Sťahuje sa… / Hotovo / Zlyhalo*.
- Stiahnuté súbory sa nikdy nespúšťajú. Klik na hotový súbor alebo **Zobraziť vo Finderi** ho iba ukáže v priečinku.
- **Vymazať zoznam** odstráni len záznamy, súbory na disku zostanú.
- Zoznam stiahnutých je v trezore, **samotné súbory šifrované nie sú**.

## Blokovanie reklám a sledovačov

- Používa natívny **WKContentRuleList**: WebKit zablokuje požiadavku vo svojej sieťovej vrstve ešte pred pripojením. Je to rýchlejšie a úspornejšie než blokovanie cez JavaScript.
- Vstavaný zoznam obsahuje **105 domén**, napríklad:
  - reklamné siete Google (DoubleClick, AdSense, Tag Manager, Analytics)
  - reklamné burzy (Criteo, Taboola, Outbrain, Rubicon, PubMatic, AppNexus…)
  - sledovače a telemetria (Hotjar, Mixpanel, Segment, Clarity, Yandex Metrika, TikTok/LinkedIn/Twitter/Pinterest pixely…)
  - slovenské a české siete (eTarget, R2B2, Seznam SSP, Gemius)
- Navyše sa skrývajú bežné reklamné prvky na stránkach (`.adsbygoogle`, `div-gpt-ad`, `[data-ad-slot]`…).
- **Vlastný zoznam:** súbor `~/Library/Application Support/Crow/blocklist.txt`. Každý riadok obsahuje jednu doménu v jednom z formátov:
  ```
  # komentár
  example-ads.com
  0.0.0.0 tracker.example.net
  ||ads.example.org^
  ```
  Podporuje teda bežné hosts súbory aj zjednodušený AdBlock formát. Zoznam môže mať najviac 50 000 pravidiel a načíta sa pri štarte.
- Blokovanie sa zapína a vypína v **Nastavenia → Ochrana súkromia** a platí okamžite pre všetky karty. Stav ukazuje ikona štítu v paneli.

## Priblíženie

- **⌘+** priblížiť, **⌘−** oddialiť, **⌘0** vrátiť na predvolené.
- Kroky: 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300 %.
- Platí pre všetky karty. Predvolená hodnota sa nastavuje v Nastaveniach.

## Nastavenia

Otvoríš ich cez **⌘,** alebo ⚙. Panel má bočné menu: **Nastavenia · História · Obľúbené · Stiahnuté**. Zavrieš ho cez **Esc** alebo **✕**. Zmeny sa uložia okamžite do šifrovaného trezora.

| Sekcia | Nastavenie | Možnosti | Predvolené |
|---|---|---|---|
| Vyhľadávanie | Predvolený vyhľadávač | DuckDuckGo, Google, Bing, Brave Search, Startpage, Ecosia | DuckDuckGo |
| Spustenie | Domovská stránka | ľubovoľná URL | `https://duckduckgo.com` |
| Spustenie | Pri spustení otvoriť | Domovskú stránku / Posledné karty | Posledné karty |
| Vzhľad | Motív | Tmavý / Svetlý / Systém | Tmavý |
| Vzhľad | Predvolené priblíženie | 50 % – 200 % | 100 % |
| Výkon | Uspávať neaktívne karty po | 1 / 5 / 15 / 30 / 60 min / Nikdy | 5 min |
| Ochrana súkromia | Blokovanie reklám a sledovačov | Zapnuté / Vypnuté | Zapnuté |
| Ochrana súkromia | Vymazať históriu | (dvojklik na potvrdenie) | |
| Ochrana súkromia | Vymazať cookies a prihlásenia | odhlási zo všetkých webov | |
| Ochrana súkromia | Vymazať zoznam stiahnutých | súbory zostanú | |
| Šifrovanie | Zamknúť teraz | ⌃⌘L | |
| Šifrovanie | Zmeniť heslo | aktuálne + nové 2× (min. 8 znakov) | |

Motív **Systém** sleduje nastavenie macOS. Stránky podporujúce `prefers-color-scheme` (napr. DuckDuckGo) sa prepnú tiež.

## Ukladanie a obnovenie relácie

- Otvorené karty sa ukladajú do trezora. Pri voľbe **Posledné karty** sa po odomknutí obnovia na rovnakom mieste a s rovnakou aktívnou kartou.
- Ukladanie prebieha:
  - automaticky každých **30 sekúnd**, ak sa niečo zmenilo
  - okamžite pri zmene nastavení alebo hesla
  - pri **⌘Q** a zatvorení okna (Crow počká na export cookies, najviac 3 s)
  - pri zamknutí (⌃⌘L)
  - pri ukončení systémom (Dock, odhlásenie), vtedy s poslednými známymi cookies
- Ukladajú sa len karty s adresou `http(s)`.

## Menu aplikácie

| Menu | Položky |
|---|---|
| **Crow** | Settings… (⌘,), Lock Crow (⌃⌘L), Hide, Hide Others, Show All, Quit Crow (⌘Q) |
| **File** | New Tab (⌘T), Reopen Closed Tab (⌘⇧T), Close Tab (⌘W), Open Location (⌘L) |
| **Edit** | Undo, Redo, Cut, Copy, Paste, Select All, Find… (⌘F), Find Next (⌘G), Find Previous (⌘⇧G) |
| **View** | Reload (⌘R), Zoom In (⌘+), Zoom Out (⌘−), Actual Size (⌘0), Enter Full Screen |
| **History** | Home (⌘⇧H), Back (⌘[), Forward (⌘]), Show History (⌘Y) |
| **Bookmarks** | Bookmark This Page (⌘D), Show Bookmarks (⌘⌥B) |
| **Window** | Minimize, Zoom, Next Tab (⌘⇧]), Previous Tab (⌘⇧[), Downloads (⌘⌥L), Tab 1–8 (⌘1–⌘8), Last Tab (⌘9) |

Kompletný zoznam skratiek je v [SKRATKY.md](SKRATKY.md).
