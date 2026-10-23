# Defenzívny bezpečnostný audit, robustnosť architektúry a penetračná revízia: CrowBrowser

**Dátum auditu:** 23. október 2026  
**Cieľ auditu:** Hĺbková previerka IPC komunikačných kanálov (WebKit bridge), izolácie schém a medzipôvodových hraníc (cross-origin), správania pri extrémnom zaťažení (DOM / WebGL / Canvas stress) a hygieny injektovaných používateľských skriptov.  
**Hodnotený repozitár:** CrowBrowser (vetva `main`)  
**Metodika:** Defenzívna analýza zdrojového kódu (Rust, JavaScript, HTML), modelovanie hrozieb (Threat Modeling), revízia pamäťových hraníc a audit súladu s `CLAUDE.md`.

---

## 1. Manažérske zhrnutie (Executive Summary)

Projekt **CrowBrowser** vykazuje v kľúčových architektonických bodoch vysokú úroveň zabezpečenia:
- **Separácia IPC:** Webové taby (`TabCtx::build`) **nikdy neinjektujú `window.ipc`** ani nepripájajú `with_ipc_handler`. Tým je zabezpečené, že bežný webový obsah nemá natívny most do Rust backendu.
- **Ephemeral úložisko:** WebKit beží v plne efemérnom režime bez ukladania nešifrovaných vyrovnávacích pamätí na disk (`nonPersistentDataStore` na macOS a `WebContext::new_ephemeral()` na Linuxe).
- **DOM XSS ochrana interného UI:** Interné HTML komponenty (`toolbar.html`, `panel.html`, `siteinfo.html`, `shield.html`, `lock.html`) striktne používajú DOM API (`textContent`, `createElement`, `replaceChildren`) a neobsahujú nebezpečné vstupy typu `innerHTML` ani `document.write`.

Napriek týmto silným stránkam audit identifikoval niekoľko architektonických rizík a oblastí vyžadujúcich defenzívne spevnenie (hardening), najmä v oblasti chýbajúcej navigácie v privileged UI kontextoch, absencie typovej schémy pre IPC príkazy, chýbajúceho ošetrenia pádu WebProcessu pri záťaži a znečistenia globálneho menného priestoru v `FAV_JS`.

---

## 2. Podrobná analýza auditovaných oblastí

### 2.1 IPC / Bridge Message Sanitization (`src/wk.rs` / `src/app.rs`)

#### Stav implementácie:
1. Privilegované rozhranie `window.ipc` je registrované výhradne v `ui_view`:
   ```rust
   let b = wk::ui_builder()
       .with_html(wk::ui_html(html))
       .with_focused(true)
       .with_ipc_handler(move |req| {
           let _ = p.send_event(UserEvent::Ipc(req.into_body()));
       });
   ```
2. Správy z UI prichádzajú do `App::command(&mut self, cmd: &str)` a následne do `session_cmd`.
3. Validácia je ad-hoc reťazcová: `let (name, arg) = cmd.split_once(':').unwrap_or((cmd, ""));`. Len pri `find` a `change-pw` sa používa štruktúrovaná deserializácia cez `serde_json::from_str`.

#### Slabiny:
- **Absencia typovej Serde schémy pre IPC príkazy:** Celý komunikačný most spolieha na manuálne parsovanie stringov oddelených dvojbodkou alebo rovná sa (`set:theme=dark`). V prípade neštandardného formátu alebo neočakávaných znakov môže dochádzať k tichému zlyhaniu alebo nepredvídateľnému stavu.
- **Zdieľaný IPC kanál bez validácie odosielateľa (Sender Origin):** Všetky interné WebViews (`toolbar`, `panel`, `shield`, `siteinfo`, `lock`) posielajú správy do jedného spoločného kanála `UserEvent::Ipc(String)`. `shield` popover tak teoreticky môže poslať príkazy patriace panelu (`change-pw`, `clear:cookies`, `reset`).
- **Chýbajúci `with_navigation_handler` v `ui_view`:** `ui_view` obsahuje prísnu CSP hlavičku `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; ...">`, avšak nemá zaregistrovaný `with_navigation_handler`. Ak by došlo k navigácii (napr. drag-and-drop externého URL na lištu alebo klik na odkaz bez `target`), WebKit by mohol načítať externú stránku priamo v inštancii WebView, ktorá má aktívny `window.ipc`.

---

### 2.2 Izolácia schém a medzipôvodové hranice (Scheme Isolation & Cross-Origin Boundaries)

#### Stav implementácie:
1. `crow:changelog` je implementovaná interne cez funkciu `internal_page()` a priamu injekciu statického HTML (`v.load_html(&html)`). Keďže schéma `crow:` nie je registrovaná ako sieťový protokol, externé stránky ju nemôžu načítať cez `fetch` ani vložiť cez `<iframe>`.
2. V `to_url` je definovaný zoznam povolených explicitných schém:
   ```rust
   const SCHEMES: &[&str] = &[
       "http://", "https://", "file:", "about:", "data:", "blob:", "mailto:",
   ];
   ```

#### Slabiny:
- **Povoľovanie schémy `file:` bez reštrikcií lokálneho kontextu:** Ak používateľ zadá alebo otvorí lokálny súbor `file:///...`, WebKit na macOS aj Linuxe štandardne inicializuje lokálny bezpečnostný kontext. Pokiaľ nie sú explicitne vypnuté direktívy `allowFileAccessFromFileURLs` a `allowUniversalAccessFromFileURLs`, lokálne HTML dokumenty môžu čítať okolité súbory na disku.
- **Chýbajúci `with_navigation_handler` na úrovni tabov:** Taby nemajú záchytný filter na blokovanie nebezpečných pseudoprotokolov (napr. `javascript:`, systémových URI handlerov ako `tel:`, `facetime:`, `itms:` a pod.). Zatiaľ čo `to_url` neznáme schémy presmeruje na vyhľadávač, volania `open:` a `open-new:` v `session_cmd` priamo odovzdávajú URL do `v.load_url(&url)`.
- **Chýbajúca direktíva `frame-ancestors 'none'` v `changelog.html`:** CSP v `changelog.html` obsahuje reštrikcie pre zdroje (`default-src 'none'`), no neobsahuje `frame-ancestors 'none'`.

---

### 2.3 Zaťaženie DOM / WebGL / Canvas a odolnosť procesov (Process Stress Handling)

#### Stav implementácie:
1. WebKit využíva viacprocesovú architektúru: JavaScript a renderovanie bežia v samostatnom procese (`com.apple.WebKit.WebContent` na macOS, `WebKitWebProcess` na Linuxe).
2. Taby, ktoré bežia na pozadí, sú agresívne uspávané a uvoľňované (`hidden_since`, uvoľnenie `v` pri limite).
3. Linux nastavuje `ctx.set_cache_model(CacheModel::DocumentViewer)`.

#### Slabiny:
- **Neošetrené ukončenie/pád WebProcessu (Unhandled Process Crash):**
  - Na macOS nie je implementovaná delegátska metóda `webViewWebContentProcessDidTerminate:`.
  - Na Linuxe nie je pripojený signál `web-process-terminated`.
  - **Dôsledok:** Ak náročný WebGL shader, nekonečná slučka v JS Canvas alebo nedostatok pamäte (OOM) spôsobí pád WebProcessu, WebView zostane visieť ako biela/prázdna plocha („zombie tab“). Rust aplikácia nezachytí pád, nezobrazí chybovú správu ani možnosť obnovenia stránky a asynchrónne skripty (`find`, favicon) zostanú trvalo visieť.
- **Absencia limitu počtu WebProcessov na Linuxe:** Linuxový backend nevolá `set_web_process_count_limit` na `WebContext`, čo pri otvorení väčšieho počtu tabov môže na systémoch so 4 GB RAM viesť k vyčerpaniu pamäte skôr, ako zasiahne interný uvoľňovací mechanizmus prehliadača.

---

### 2.4 Hygiena injektovaných skriptov a sandboxu (`src/defuser.js`, `FAV_JS`)

#### Stav implementácie:
1. `src/defuser.js` je vkladaný cez `with_initialization_script_for_main_only` do každého tabu. Chráni používateľa pred detekciou adblocku tým, že maskuje vlastnosti elementov (`offsetHeight`, `offsetWidth`, návratové hodnoty `Function.prototype.toString`).
2. `FAV_JS` beží na stránke a získava ikonu stránky re-enkódovaním cez `<canvas>` do 32x32 PNG data URL.

#### Slabiny:
- **Znečistenie globálneho objektu `window.__crowIcon` (Fingerprinting & State Pollution):**
  - `FAV_JS` ukladá vygenerovanú ikonu do globálnej premennej: `window.__crowIcon = cv.toDataURL('image/png');`.
  - Následne Rust periodicky spúšťa `FAV_READ` (`window.__crowIcon || ''`).
  - **Riziko:** Akýkoľvek skript bežiaci na webe môže skontrolovať existenciu `window.__crowIcon`. Tým stránka jednoznačne deteguje CrowBrowser (browser fingerprinting). Stránka navyše môže do tejto premennej podstrčiť vlastnú PNG data URL a manipulovať s ikonou tabu v lište.
- **Defuser.js a modifikácia natívnych prototypov:**
  - `defuser.js` modifikuje prototypy `HTMLElement.prototype` a `Function.prototype`. Keďže beží v rovnakom kontexte ako webová stránka (nie v izolovanom svete `WKContentWorld`), sofistikované anti-adblock skripty môžu detegovať rozdiely v deskriptoroch vlastností.

---

## 3. Zoznam nálezov podľa závažnosti

| ID | Závažnosť | Názov zraniteľnosti / slabiny | Komponent |
|---|---|---|---|
| **SEC-01** | **Medium** | Chýbajúci navigačný filter v privilegovanom internom UI (`ui_view`) | `src/app.rs` |
| **SEC-02** | **Medium** | Absencia obsluhy pádu WebProcessu pri záťaži (Zombie tab / Unhandled OOM) | `src/wk.rs`, `src/wk_gtk.rs` |
| **SEC-03** | **Medium** | Ad-hoc nestruktúrované IPC správy bez overenia odosielateľa | `src/app.rs` |
| **SEC-04** | **Low** | Znečistenie globálneho okna `window.__crowIcon` a detekovateľnosť prehliadača | `src/app.rs` (`FAV_JS`) |
| **SEC-05** | **Low** | Nekontrolovaná schéma `file:` a chýbajúce vypnutie lokálneho prístupu k súborom | `src/app.rs`, `src/wk.rs` |
| **SEC-06** | **Low** | Chýbajúca direktíva `frame-ancestors 'none'` v `changelog.html` | `src/changelog.html` |

---

## 4. Odporúčané defenzívne nápravy a architektúrne záplaty

### Náprava 1 (Kritické pre SEC-01): Striktný navigačný filter v `ui_view`
Všetky interné WebViews (`toolbar`, `panel`, `shield`, `siteinfo`, `lock`) musia explicitne odmietnuť akúkoľvek navigáciu na externé schémy, aby privilegovaný kontext s `window.ipc` nikdy nemohol zobraziť neoverený webový obsah.

**Defenzívny kód pre `src/app.rs` (`ui_view`):**
```rust
let b = wk::ui_builder()
    .with_html(wk::ui_html(html))
    .with_background_color(bg_of(&self.window))
    .with_focused(true)
    .with_navigation_handler(|url| {
        // Interné UI nesmie nikdy navigovať na externý web
        url.starts_with("data:") || url.starts_with("about:blank")
    })
    .with_ipc_handler(move |req| {
        let _ = p.send_event(UserEvent::Ipc(req.into_body()));
    });
```

---

### Náprava 2 (Kritické pre SEC-02): Obsluha ukončenia WebProcessu
Pridať do delegáta WebKit na macOS aj Linuxe handler pre neočakávané ukončenie procesu renderingu.

**Pre macOS (`src/wk.rs`):**
Zaregistrovať delegátsku metódu `webViewWebContentProcessDidTerminate:`:
```rust
unsafe extern "C-unwind" fn web_process_crashed(
    _this: *mut AnyObject,
    _cmd: Sel,
    web_view: *mut AnyObject,
) {
    if let Some(tab) = tab_of(web_view) {
        // Notifikovať Rust aplikáciu o páde WebProcessu daného tabu
        eprintln!("Varovanie: WebProcess pre tab {tab} neočakávane spadol (OOM/Crash).");
    }
}
```
A pridať `add_method(cls, sel!(webViewWebContentProcessDidTerminate:), imp, c"v@:@");`.

**Pre Linux (`src/wk_gtk.rs`):**
Pripojiť signál `web-process-terminated` na `webkit2gtk::WebView`:
```rust
w.connect_web_process_terminated(|_view, reason| {
    eprintln!("Varovanie: WebKit WebProcess bol ukončený. Dôvod: {reason:?}");
});
```

---

### Náprava 3 (Kritické pre SEC-03): Typovo bezpečný Serde IPC protokol
Nahradiť ad-hoc delenie reťazcov typovaným JSON enumom:
```rust
#[derive(serde::Deserialize)]
#[serde(tag = "cmd", content = "payload")]
pub enum IpcMessage {
    Drag,
    Quit,
    Unlock { password: Zeroizing<String> },
    Navigate { url: String },
    CloseTab { id: Option<u32> },
    ChangePassword { old: Zeroizing<String>, new: Zeroizing<String> },
    SetSetting { key: String, val: String },
    // Ďalšie striktne typované varianty...
}
```

---

### Náprava 4 (Kritické pre SEC-04): Odstránenie `window.__crowIcon` z globálneho scope
Namiesto ukladania ikony do `window.__crowIcon` spúšťať favicon extrakciu v anonymnej samovykonávacej funkcii, ktorej výsledok je vrátený priamo v callbacku asynchrónneho vyhodnotenia:

```javascript
// Namiesto window.__crowIcon použiť anonymnú návratovú hodnotu
(() => {
  const link = document.querySelector('link[rel~="icon"]');
  return link ? link.href : (location.origin + '/favicon.ico');
})()
```
Tým sa zabráni fingerprintingu a webová stránka nemá prístup k žiadnej špecifickej premennej prehliadača.

---

### Náprava 5 (Kritické pre SEC-05 & SEC-06): Zabezpečenie `file://` a `changelog.html`
1. V `src/wk.rs` explicitne nastaviť pre tab konfigurácie:
   ```rust
   // Zakázať prístup z file:// na iné lokálne súbory
   let prefs: *mut AnyObject = msg_send![c, preferences];
   let _: () = msg_send![prefs, setAllowFileAccessFromFileURLs: false];
   let _: () = msg_send![prefs, setAllowUniversalAccessFromFileURLs: false];
   ```
2. V `src/changelog.html` doplniť direktívu `frame-ancestors 'none'`:
   ```html
   <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'none'; img-src data:; frame-ancestors 'none';">
   ```

---

## 5. Záver auditu

CrowBrowser disponuje solídnym a premysleným bezpečnostným jadrom (striktná absencia `window.ipc` na webových taboch, plne atomický a zeroizovaný trezor, transparentná izolácia dát). Implementáciou odporúčaných defenzívnych záplat (najmä navigačných bariér v `ui_view` a obsluhy pádov WebProcessu) dosiahne prehliadač robustnú úroveň ochrany odolnú voči moderným technikám obchádzania sandboxu a záťažovým zlyhaniam.
