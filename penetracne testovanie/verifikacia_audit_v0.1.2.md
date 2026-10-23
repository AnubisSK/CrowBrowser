# Verifikačný bezpečnostný audit a regresná revízia architektúry: CrowBrowser v0.1.2

**Dátum auditu:** 23. október 2026  
**Predmet auditu:** Overenie odstránenia zistení z predchádzajúceho auditu (`audit_architektura_robustnost.md` – SEC-01 až SEC-06), kontrola nových vektorov zraniteľností, pamäťových a procesných hraníc a posúdenie pripravenosti na vydanie verzie `v0.1.2`.  
**Hodnotený kód:** CrowBrowser v0.1.2 (vetva `main`)  
**Výsledok testov:** 13/13 unit testov úspešných (`cargo test`)

---

## 1. Matica stavu zistení (Status Matrix SEC-01 – SEC-06)

| ID | Názov zistenia | Stav | Presná referencia v kóde | Poznámka k overeniu |
|---|---|:---:|---|---|
| **SEC-01** | Privileged UI Navigation Guard | **RESOLVED** | [`src/app.rs:324`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L324) | V `ui_view` je pridaný `.with_navigation_handler(\|url\| url == "about:blank")`. Akákoľvek externá navigácia je WebKitom okamžite zablokovaná, čím je `window.ipc` neprenosný na webový obsah. |
| **SEC-02** | WebProcess Termination & Recovery | **RESOLVED** | [`src/wk.rs:368`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk.rs#L368), [`src/wk.rs:454`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk.rs#L454), [`src/wk_gtk.rs:460`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk_gtk.rs#L460), [`src/app.rs:690`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L690), [`src/app.rs:1415`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L1415), [`src/app.rs:2421`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L2421) | Pád alebo OOM renderovacieho procesu na macOS (`webViewWebContentProcessDidTerminate:`) aj Linuxe (`web-process-terminated`) je prepojený na `UserEvent::WebProcessTerminated`. Na aktívnej karte sa vykreslí statická in-memory stránka `crash_page` s bezpečným tlačidlom na opätovné načítanie. Neaktívne karty sú uspané (`drop_view`). |
| **SEC-03** | IPC Typing & Protocol Hygiene | **PARTIALLY RESOLVED** | [`src/app.rs:980`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L980), [`src/app.rs:1776`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L1776) | Jednotlivé príkazy (`set`, `adblock-allow`, `delete-vault-session`, `reveal-file`, `change-pw`) majú prísnu sanitizáciu a whitelisting vstupov. Globálny protokol však naďalej používa manuálny string split (`cmd.split_once(':')`) namiesto unifikovaného Serde enumu. Riziko zneužitia je eliminované vďaka SEC-01. |
| **SEC-04** | Global Scope Leaks & Fingerprinting | **RESOLVED** | [`src/wk.rs:189`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk.rs#L189), [`src/wk_gtk.rs:266`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk_gtk.rs#L266), [`src/app.rs:483`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L483), [`src/app.rs:1355`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L1355) | Načítanie a extrakcia favikony beží cez `eval_isolated` v samostatnom svete WebKitu (`WKContentWorld` na macOS, `WebKitScriptWorld("crow")` na Linuxe). Skripty bežiace na webovej stránke premennú `window.__crowIcon` vôbec nevidia; fingerprinting bol odstránený. |
| **SEC-05** | Local File Scheme Hardening | **RESOLVED** | [`src/wk.rs:125`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk.rs#L125), [`src/wk.rs:131`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk.rs#L131), [`src/wk_gtk.rs:497`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk_gtk.rs#L497), [`src/wk_gtk.rs:498`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/wk_gtk.rs#L498) | V nastaveniach WebKitu na macOS aj Linuxe sú explicitne vypnuté direktívy `allow_file_access_from_file_urls` a `allow_universal_access_from_file_urls`. Lokálny súbor načítaný cez `file://` nemá prístup k iným lokálnym súborom ani sieťovým pôvodom. |
| **SEC-06** | CSP Headers & Frame Ancestors | **RESOLVED** | [`src/changelog.html:5`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/changelog.html#L5) | V `src/changelog.html` bola do Content-Security-Policy doplnená direktíva `frame-ancestors 'none'`. V kombinácii so `script-src 'none'` a `default-src 'none'` je stránka plne chránená pred vložením do rámcov. |

---

## 2. Regresná analýza a revízia nových komponentov

### 2.1 Analýza DOM XSS v šablóne obnovy po páde (`crash_page`)
- **Implementácia:** [`src/app.rs:2421-2460`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L2421-L2460).
- **Overenie sanitizácie:**
  - Vstupná URL je kontrolovaná podmienkou `lower.starts_with("https://") || lower.starts_with("http://")`. Nebezpečné schémy (`javascript:`, `data:`, `file:`) sú vyradené a reload tlačidlo sa pre ne nevytvorí.
  - Generovanie obsluhy kliknutia: `html_escape(&format!("window.location.replace({})", js(url)))`. Funkcia `js()` serializuje reťazec do JSON (zabezpečuje escape úvodzoviek, spätných lomiek a riadiacich znakov) a `html_escape` nahradí `"` za `&quot;`, `<` za `&lt;`, `>` za `&gt;` a `'` za `&#39;`.
  - Všetky textové preklady (`title`, `desc`, `back`, `lang`) a zobrazená URL (`shown`) prechádzajú cez `html_escape`.
  - Stránka obsahuje striktnú CSP: `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline';`.
- **Záver:** Šablóna `crash_page` neobsahuje žiadny DOM XSS vektor.

### 2.2 Bezpečnosť FFI a práca so smerníkmi v `src/wk.rs`
- **Modernizácia FFI:** Surové pretypovania smerníkov boli refaktorované na bezpečné konštrukcie `&raw const` a `.cast::<c_void>()` v súlade so štandardom Rust 2024.
- **Asociované objekty (`objc_setAssociatedObject`):**
  - Kľúč `TAB_KEY` používa statickú adresu, ktorá sa nikdy nedereferencuje.
  - Priradenie `OBJC_ASSOCIATION_RETAIN_NONATOMIC` korektne spravuje životný cyklus asociovaného `NSString`.
  - Pri zatvorení/odpojení karty volanie `set_tab(..., None)` s `null_mut()` objekt korektne uvoľní.
- **Manažment certifikátov (`cert_chain`):** Volanie `SecCertificateCopyData` vytvára vlastnený `CFDataRef`, ktorý je po skopírovaní bajtov explicitne uvoľnený cez `CFRelease(data)`. Nedochádza k úniku pamäte (memory leak) ani pri opakovanom otváraní popoveru s informáciami o certifikáte.
- **Záver:** FFI vrstva na macOS je pamäťovo bezpečná a odolná voči pretečeniu či dangling smerníkom.

### 2.3 Ephemerality a Zero-Knowledge model pamäte
- **Perzistentná izolácia:** WebKit na macOS používa `WKWebsiteDataStore::nonPersistentDataStore` a na Linuxe `WebContext::new_ephemeral()`. Žiadne súbory cookies, localStorage, IndexedDB ani HTTP cache sa nezapisujú na disk.
- **Zeroizácia citlivých štruktúr:** Trait `Drop` s volaním `zeroize()` je striktne zachovaný na štruktúrach `HistoryEntry`, `Bookmark`, `Download`, `CookieRec`, `Urls` a `Vault` (kľúč zabalený v `Zeroizing<[u8; 32]>`).
- **Dočasné buffery:** Heslo prijaté cez IPC je okamžite po spracovaní vynulované (`msg.zeroize()`).
- **Záver:** Zaručená nulová stopa na disku a bezpečná hygiena RAM zostávajú plne intaktné.

### 2.4 Asynchrónne správanie ťahania okna (Drag Events) a IPC vstupy
- **Eliminácia race condition pri ťahaní okna:**
  - V [`src/toolbar.html:200`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/toolbar.html#L200) pomocník `dragOn` kontroluje `e.button !== 0 || e.detail > 1 || e.target.closest('button, input, .tab, .no-drag')` a volá `e.preventDefault()`. Tým zabraňuje duplicitnému spracovaniu kliku WebKitom.
  - V [`src/app.rs:1000`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L1000) sa pred volaním `w.drag_window()` overuje `wk::left_button_down()`. Ak používateľ stihol tlačidlo myši uvoľniť skôr, ako asynchrónna IPC správa dorazila do Rustu, volanie sa bezpečne preskočí a nezobrazí sa systémové varovanie AppKit.
- **Preusporiadanie kariet (Tab Reordering):**
  - Ak počas preťahovania karty dorazí aktualizácia stavu z backendu, v [`src/toolbar.html:245`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/toolbar.html#L245) sa ukladá do `pendingState` a DOM sa neprekreslí uprostred gesta, čo zabraňuje prerušeniu DOM elementov.

---

## 3. Identifikované okrajové prípady (Edge Cases)

Počas hĺbkovej previerky bol identifikovaný jeden drobný funkčný okrajový prípad:
- **Tlačidlo Reload na lište pri zobrazenej crash stránke:**
  - V [`src/app.rs:1902`](file:///Volumes/ExternyDisk/Projekty/Github/CrowBrowser/src/app.rs#L1902) podmienka pre špeciálne reloadovanie ošetruje iba `s.cur().is_some_and(|t| t.tls_error || t.internal)`. Príznak `t.crashed` v tejto podmienke chýba.
  - Ak používateľ po páde procesu klikne na tlačidlo „Znovu načítať stránku“ na samotnej crash stránke (`crash_page`), stránka sa korektne a bezpečne načíta cez `window.location.replace(...)`.
  - Ak však používateľ namiesto toho klikne na ikonu reload na hlavnej lište prehliadača, vykoná sa štandardné `v.reload()`, ktoré opätovne načíta in-memory crash stránku namiesto pôvodnej URL.
  - *Dopad:* Zanedbateľný (používateľ má k dispozícii výrazné tlačidlo priamo v strede obrazovky). Odporúča sa doplniť `|| t.crashed` do obsluhy príkazu `reload` v ďalšom bežnom cykle.

---

## 4. Záverečný verdikt (Final Verdict)

### **HODNOTENIE: PASS (PRIPRAVENÉ NA VYDANIE v0.1.2)**

Všetky bezpečnostné nálezy vysokej a strednej závažnosti z auditu architektúry boli plne vyriešené. Privilegované interné rozhranie je bezpečne izolované pred vonkajším webom, manipulácia s procesmi pri preťažení pamäte/grafiky je ošetrená na úrovni oboch podporovaných platforiem a ephemeral model nezaznamenal žiadne regresie. 

CrowBrowser verzia `v0.1.2` spĺňa prísne kritériá bezpečnostného auditu a je spôsobilá na produkčné nasadenie.
