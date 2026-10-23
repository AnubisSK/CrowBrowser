# Funkcie

Prehľad všetkého, čo Crow vie (verzia 0.1.2).

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
- Karta ukazuje ikonu webu (favicon), bez nej bodku. Počas načítavania sa točí indikátor.
- **Presúvanie:** kartu chyť a ťahaj vľavo alebo vpravo. Poradie sa uloží aj do relácie.

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
- **Bezpečný názov:** z názvu sa odstránia cesty, neviditeľné a smer meniace znaky (napr. U+202E, ktorý z `gpj.exe` urobí na pohľad `exe.jpg`), úvodné bodky a výplň medzerami. Maskovacia prípona pred spustiteľnou sa zneškodní: `faktura.pdf.exe` sa uloží ako `faktura_pdf.exe`.
- **Spustiteľné súbory** (`.exe`, `.app`, `.command`, `.sh`, `.desktop`, `.deb`, `.rpm`, `.dmg`, `.pkg`, skripty…) majú vo výzve červené upozornenie „⚠ Spustiteľný súbor“.
- **Karanténa (macOS):** dokončený súbor dostane atribút `com.apple.quarantine`, takže ho pri otvorení skontroluje Gatekeeper (rovnako ako pri Safari). Platí aj pre inštalátor aktualizácie.
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
  Podporuje teda bežné hosts súbory aj plnú syntax Adblock Plus a načíta sa pri štarte.
- **EasyList a EasyPrivacy** ([easylist.to](https://easylist.to)): filtre v syntaxi Adblock Plus vrátane ciest, výnimiek (`@@`) a kozmetických pravidiel (`##`). Spracúva ich knižnica [adblock-rust](https://github.com/brave/adblock-rust) od Brave a prevedie na pravidlá WebKitu, takže sa blokuje stále v sieťovej vrstve, pred pripojením.
- **StevenBlack hosts:** [verejný zoznam](https://github.com/StevenBlack/hosts) reklamných a škodlivých domén (~70 000), subdomény už blokovaných domén sa vynechajú.
- Zoznamy sa po odomknutí stiahnu, ak chýbajú alebo sú staršie ako týždeň. Zapínajú sa, aktualizujú a ich počty pravidiel sú v **Nastavenia → Ochrana súkromia → Zoznamy blokovania**. Ležia v priečinku dát Crow (`easylist.txt`, `easyprivacy.txt`, `hosts-stevenblack.txt`).
- WebKit dovolí najviac 150 000 pravidiel v jednom zozname, preto má každý zdroj vlastný zoznam (vstavaný + `blocklist.txt`, EasyList, EasyPrivacy, hosts). Ak by zdroj limit prekročil, rozdelí sa na viac zoznamov a výnimky (`@@`) sa skopírujú do každého z nich, takže sa žiadne pravidlo nestratí. Prevod a kompilácia (~10 s na pozadí) bežia len keď sa niektorý zdroj zmení, inak sa načíta skompilovaná kópia.
- **Obchádzanie detekcie blokovania** („Blokovače reklám sú zakázané“): do stránok sa pred ich skriptami vloží malý skript. Zablokovaný reklamný skript či obrázok sa stránke javí ako načítaný, `fetch` na reklamnú adresu vráti prázdnu odpoveď 200, nastavia sa „návnadové“ premenné (`canRunAds` …) a skryté návnadové prvky (`.adsbox`, `ins.adsbygoogle` …) hlásia nenulovú veľkosť. Ak web aj tak trvá na vypnutí, pomôže výnimka cez štít.
- Blokovanie sa zapína a vypína v **Nastavenia → Ochrana súkromia** a platí okamžite pre všetky karty.
- **Štít v paneli:** modrý = blokuje sa, sivý = vypnuté. Číslo na ňom je počet zablokovaných požiadaviek na aktuálnej stránke (nová stránka začína od nuly). **⌥+klik** (Linux **Alt+klik**) prepne blokovanie všade.
- **Klik na štít** otvorí okno v štýle Brave Shields: doména, prepínač pre tento web (výnimka platí aj pre subdomény, karty webu sa znova načítajú), počet zablokovaných reklám a sledovačov, ušetrené dáta a čas a súhrn za celé používanie. Dáta a čas sú odhad: 50 kB a 50 ms na každú zablokovanú požiadavku. Zatvorí sa klikom mimo neho alebo **Esc**. Celkový počet sa ukladá do šifrovaného trezora.
- Prepínač pre web je aj v informáciách o stránke (zámok).
- Výnimky sú uložené v šifrovanom trezore a dajú sa odstrániť v Nastaveniach.

## Prihlásenia a relácie

- Panel **Prihlásenia** (⚙ → bočné menu) ukazuje weby, ktoré majú v trezore cookies: doménu, poslednú návštevu (z histórie), počet cookies a značku **Aktívna relácia**, ak má web prihlasovaciu cookie (podľa názvu, napr. `session`, `sid`, `auth`, `token`).
- **Odhlásiť a vymazať** (dvojklik na potvrdenie) vymaže všetky cookies webu vrátane subdomén: najprv z trezora (bezpečný zápis cez `.vault.tmp`), potom z bežiaceho prehliadača. Web ťa po ďalšom načítaní odhlási.
- Do panela sa neposielajú hodnoty ani názvy cookies. Crow neukladá heslá ani mená, preto je stĺpec používateľa prázdny.
- Ak si na aktuálnom webe prihlásený, v adresnom riadku je ikona kľúča. Klik na ňu otvorí tento panel.

## Čo je nové

- Zoznam zmien aktuálnej verzie je vstavaný a funguje bez pripojenia: odkaz **Čo je nové** dole v bočnom menu panela, alebo adresa `crow:changelog` (aj `about:changelog`) v adresnom riadku. Stránka nemá skripty ani prístup k sieti.

## Video na celú obrazovku

- Tlačidlo celej obrazovky vo videách (YouTube a iné prehrávače, Fullscreen API) funguje. Na macOS WebKit zobrazí video vo vlastnom okne na celú obrazovku, na Linuxe sa okno Crow prepne na celú obrazovku a video ho vyplní (panel sa skryje). **Esc** alebo tlačidlo v prehrávači to ukončí, rovnako prepnutie na inú kartu.

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
| Vzhľad | Jazyk | Slovenčina / Čeština / English / Deutsch (prepne sa hneď vo všetkých častiach rozhrania) | Slovenčina |
| Vzhľad | Motív | Tmavý / Svetlý / Systém | Tmavý |
| Vzhľad | Predvolené priblíženie | 50 % – 200 % | 100 % |
| Výkon | Uspávať neaktívne karty po | 1 / 5 / 15 / 30 / 60 min / Nikdy | 5 min |
| Ochrana súkromia | Blokovanie reklám a sledovačov | Zapnuté / Vypnuté | Zapnuté |
| Ochrana súkromia | Vymazať históriu | (dvojklik na potvrdenie) | |
| Ochrana súkromia | Vymazať cookies a prihlásenia | odhlási zo všetkých webov | |
| Ochrana súkromia | Vymazať zoznam stiahnutých | súbory zostanú | |
| Šifrovanie | Zamknúť teraz | ⌃⌘L | |
| Šifrovanie | Zmeniť heslo | aktuálne + nové 2× (min. 8 znakov) | |
| O aplikácii | Automaticky kontrolovať aktualizácie | Zapnuté / Vypnuté | Zapnuté |
| O aplikácii | Skontrolovať aktualizácie | ručná kontrola | |

Motív **Systém** sleduje nastavenie macOS. Stránky podporujúce `prefers-color-scheme` (napr. DuckDuckGo) sa prepnú tiež.

## Aktualizácie

- Po odomknutí sa Crow raz opýta GitHub Releases (`AnubisSK/CrowBrowser`), či existuje novšia verzia. Ide to cez systémový `curl` a odošle sa len požiadavka na GitHub API, žiadne údaje z trezora.
- Ak je novšia, pod adresným riadkom sa zobrazí lišta **Čo je nové · Neskôr · Stiahnuť**.
- **Čo je nové:** zoznam commitov medzi tvojou verziou a novým vydaním. Klik na commit ho otvorí na GitHube.
- **Stiahnuť:** inštalátor pre tvoj systém sa uloží do Stiahnutých a overí sa podľa SHA-256 z GitHubu. Ak nesedí, súbor sa zmaže. Overený inštalátor sa ukáže vo Finderi (správcovi súborov), **nikdy sa nespustí sám**.

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
