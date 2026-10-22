# Hĺbkový bezpečnostný, logický a záťažový audit CrowBrowser (v0.1.0) – stav opráv vo v0.1.1

**Rola:** Principal Security Architect & Rust Systems Auditor  
**Dátum auditu:** 22. október 2026  
**Stav opráv:** doplnený 22. októbra 2026 pre verziu 0.1.1. Pri každom náleze je blok **Stav**: čo sa zmenilo, kde a ako to bolo overené. Kde sa predpoklad auditu v kóde nepotvrdil, je to uvedené.  
**Cieľ projektu:** Extrémne nízka spotreba pamäte, beh WebKitu čisto v RAM (ephemeral kontext), žiadny nešifrovaný zápis na disk a lokálny trezor chránený pomocou Argon2id + XChaCha20-Poly1305 + zeroize.

---

## Manažérske zhrnutie (Executive Summary)

Projekt **CrowBrowser** implementuje inovatívny prístup k správe pamäte a súkromia postavený na Rust knižniciach `tao`, `wry` a priamych väzbách na systémový WebKit (macOS cez `objc2-web-kit`, Linux cez `webkit2gtk-4.1`). 

Napriek prísnemu bezpečnostnému zámeru odhalil audit **3 kritické zraniteľnosti**, **6 vysoko závažných zraniteľností**, **4 stredné** a **2 nízke zraniteľnosti**. Najvážnejšie problémy zahŕňajú:
1. **Zlyhanie požiadavky na beh bez zápisu na disk:** Interné UI pohľady na macOS nepoužívajú ephemeral kontext a WebKit pre ne vytvára nešifrovanú perzistentnú cache a storage priamo na disku.
2. **Kritický únik procesov na Linuxe:** Widgety `WebKitWebView` sa pri zatvorení alebo uspaní neodstraňujú z rodičovského `GtkLayout`, čo spôsobuje permanentný beh inštancií `WebKitWebProcess` a akumuláciu pamäte RAM.
3. **Absencia varovania pri zlyhaní TLS:** Prehliadač nezobrazuje blokujúcu bezpečnostnú stránku pri neplatnom/podvrhnutom certifikáte, čo vystavuje používateľa MITM útokom.
4. **Zraniteľnosť IPC:** Absencia striktnej CSP a schémovej validácie v UI webviewoch otvára vektor na eskaláciu privilégií.

---

# 1. Pokus o prelomenie bezpečnosti a logické zraniteľnosti

---

### [Nález 1.1] Zlyhanie ephemerality na macOS – UI WebViews zapisujú nešifrované dáta a históriu na disk
- **Závažnosť:** Kritická
- **Kde:** `src/wk.rs` – funkcia `pub fn ui_builder`
- **Popis problému a scenár zlyhania:**
  Projekt deklaruje beh WebKitu čisto v RAM (ephemeral kontext) bez nešifrovaného zápisu na disk. Pre klientske taby sa v `wk::config` korektne vytvára `WKWebsiteDataStore::nonPersistentDataStore`. Avšak pre interné UI pohľady (`lock.html`, `toolbar.html`, `panel.html`, `siteinfo.html`, `shield.html`) funkcia `ui_builder()` v `src/wk.rs` vracia:
  ```rust
  pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
      WebViewBuilder::new()
  }
  ```
  Na macOS `WebViewBuilder::new()` bez explicitnej konfigurácie použije predvolený perzistentný dátový sklad (`[WKWebsiteDataStore defaultDataStore]`). WebKit pre tieto interné pohľady vytvára súbory cache, LocalStorage, IndexedDB a sieťové logy priamo na disku v `~/Library/WebKit/` a `~/Library/Caches/`. Keďže `panel.html` zobrazuje celú históriu, heslá a cookies, a `lock.html` spracúva heslo do trezora, dochádza k neoprávnenému zápisu citlivých dát na disk v čitateľnej podobe.
- **Konkrétny návrh opravy / kód:**
  Aj UI webview musí povinne dediť ephemeral konfiguráciu alebo explicitne nastaviť `nonPersistentDataStore`:
  ```rust
  // src/wk.rs
  pub fn ui_builder<'a>() -> WebViewBuilder<'a> {
      unsafe {
          let c = WKWebViewConfiguration::new(mtm());
          let store = WKWebsiteDataStore::nonPersistentDataStore(mtm());
          c.setWebsiteDataStore(&store);
          WebViewBuilder::new().with_webview_configuration(c)
      }
  }
  ```

- **Stav (v0.1.1): Opravené**
  `wk::ui_builder()` používa `with_incognito(true)`. wry v tom prípade nastaví `WKWebsiteDataStore::nonPersistentDataStore` (overené v zdrojovom kóde wry 0.57, `wkwebview/mod.rs`). Všetky UI pohľady (zamknutie, panel nástrojov, panel, popovery) teda bežia bez zápisu cache a úložiska na disk.
  - **Overenie:** samostatný test s wry 0.57 na macOS 12.7.6: UI webview s `incognito` a CSP má funkčné `window.ipc`.

---

### [Nález 1.2] Nechránený IPC kanál v UI pohľadoch a absencia CSP / doménovej izolácie
- **Závažnosť:** Vysoká
- **Kde:** `src/app.rs` – funkcia `ui_view`, `command`; `src/panel.html`, `src/toolbar.html`
- **Popis problému a scenár zlyhania:**
  Webové stránky v bežných kartách nemajú `window.ipc` priamo zaregistrované (do `TabCtx::build` sa `with_ipc_handler` nepridáva). Avšak interné panely (`toolbar.html`, `panel.html`, `siteinfo.html`, `lock.html`) majú plný prístup k `window.ipc.postMessage`.
  1. Žiadny z interných HTML súborov neobsahuje hlavičku ani meta značku `<meta http-equiv="Content-Security-Policy">`.
  2. V `ui_view` nie je nastavený žiadny navigačný filter ani blokovanie načítania externých rámcov (`iframe`). Ak by útočník dokázal cez DOM injekciu (napr. cez podvrhnutý názov stiahnutého súboru či histórie) vložiť `iframe` alebo presmerovať UI okno, akýkoľvek kód v danom okne získa prístup k `window.ipc`.
  3. Spracovanie správ v `app.rs` (`command`) neoveruje pôvod správy:
     ```rust
     "reset" => { Vault::destroy(); self.show_lock(); }
     "clear" => { ... s.data.history.clear(); ... }
     "dl-allow" => { ... }
     ```
     Útočník môže zmazať trezor (`reset`), vymazať históriu, schváliť ľubovoľné sťahovanie alebo povoliť domény v AdBlocku.
- **Konkrétny návrh opravy / kód:**
  1. Do všetkých UI HTML súborov (`lock.html`, `panel.html`, `toolbar.html`, `siteinfo.html`, `shield.html`) pridať striktnú CSP politiku:
     ```html
     <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'none';">
     ```
  2. V Ruste zaviesť typovo bezpečný deserializátor pre IPC správy namiesto voľného parsovania reťazcov `name:arg`:
     ```rust
     // src/app.rs
     #[derive(Deserialize)]
     #[serde(tag = "action", content = "payload")]
     enum IpcMessage {
         NewTab,
         CloseTab { id: u32 },
         Go { url: String },
     }
     ```

- **Stav (v0.1.1): Čiastočne opravené**
  - **CSP:** pridáva sa centrálne v `ui_view()` do všetkých interných stránok: `default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; base-uri 'none'; form-action 'none'`. `connect-src` a `frame-src` pokrýva `default-src 'none'`.
  - **Overenie:** test s wry na macOS: `fetch` aj externý skript sú zablokované, inline štýly, obrázky `data:` a `window.ipc` fungujú.
  - **Izolácia:** wry vkladá `window.ipc` len do WebView s IPC handlerom. Karty ho nemajú a okná z `window.open` dedia konfiguráciu karty, tiež bez IPC.
  - **Nezavedené:** typovaný `IpcMessage` (serde enum). Správy prichádzajú len z vlastného UI, ktoré po CSP nemôže načítať cudzí kód, a každý príkaz si svoje argumenty overuje (čísla kariet, povolené znaky domén, JSON cez serde pri `change-pw` a `find`). Prepis približne 60 príkazov by bezpečnostnú hranicu neposunul a niesol by riziko regresií. Ponechané ako samostatná úloha.

---

### [Nález 1.3] Race condition pri zamykaní/ukladaní cookies a riziko prepísania novej relácie starou
- **Závažnosť:** Vysoká
- **Kde:** `src/app.rs` – funkcie `persist`, `saved`, `after_save`
- **Popis problému a scenár zlyhania:**
  Exportovanie cookies z WebKitu je asynchrónne (`wk::export_cookies`).
  Scenár:
  1. Používateľ vyvolá uzamknutie (`After::Lock`). Spustí sa `persist(After::Lock)`.
  2. Nastaví sa `s.saving = true` a odošle sa asynchrónna požiadavka do WebKitu.
  3. Skôr, ako WebKit odpovie, alebo ak zlyhá watchdog, dôjde k uzamknutiu cez timeout.
  4. Používateľ ihneď odomkne prehliadač novým heslom. Vytvorí sa nová inštancia `Session`.
  5. Oneskorená udalosť `UserEvent::Cookies(stare_cookies, After::Lock)` z predchádzajúcej session dorazí do frontu udalostí.
  6. Metóda `saved` neobsahuje žiadny identifikátor generácie relácie (token / generation ID). Bez overenia vezme staré cookies a zavolá `s.save_now(&self.cfg)`, čím prepíše novú databázu starými údajmi a okamžite vyvolá ďalší `After::Lock`.
- **Konkrétny návrh opravy / kód:**
  Pridať `session_token: u64` do `Session` a do udalosti `UserEvent::Cookies`:
  ```rust
  // src/app.rs
  pub enum UserEvent {
      Cookies { token: u64, cookies: Vec<CookieRec>, after: After },
  }

  // Pri volaní persist:
  let token = s.session_token;
  wk::export_cookies(&s.store, move |c| {
      let _ = p.send_event(UserEvent::Cookies { token, cookies: c, after });
  });

  // V obsluhe udalosti:
  fn saved(&mut self, token: u64, cookies: Vec<CookieRec>, after: After) {
      let Some(s) = self.s.as_mut() else { return };
      if s.session_token != token {
          return; // Ignorovať oneskorené cookies zo starej relácie
      }
      // ...
  }
  ```

- **Stav (v0.1.1): Opravené**
  - Naraz beží len jeden export cookies. Ďalšie požiadavky sa zaradia do fronty a spojí sa ich nasledná akcia (Quit > Lock > Nothing).
  - Pred mazaním cookies (odhlásenie z webu, vymazanie cookies) sa zvýši `cookie_epoch`. Export spustený predtým sa zahodí a zopakuje.
  - **v0.1.1:** `Session::token` je jedinečný pre každé odomknutie (počítadlo v `App`, nikdy sa neopakuje). `UserEvent::Cookies` nesie token aj epochu a oneskorený export zo zamknutej relácie sa do novej nezapíše. Presne tento scenár z auditu predtým nebol ošetrený, lebo `cookie_epoch` začínal v každej relácii od nuly.

---

### [Nález 1.4] Chýbajúci `Zeroize` na dešifrovaných dátach trezora v RAM (`VaultData`, `CookieRec`)
- **Závažnosť:** Vysoká
- **Kde:** `src/vault.rs` – štruktúry `VaultData`, `CookieRec` a `src/app.rs` – `After::Lock`
- **Popis problému a scenár zlyhania:**
  `Vault.key` je síce chránený cez `Zeroizing<[u8; 32]>`, ale samotné dešifrované citlivé dáta v štruktúre `VaultData` nie sú chránené:
  1. `CookieRec` obsahuje autentifikačné tokeny, `sessionid` a citlivé cookies.
  2. Pri uzamknutí prehliadača (`After::Lock`) sa vykoná `self.s.take()`, čo dealokuje `Session` a `VaultData` štandardnými dealokátormi (`mimalloc`).
  3. Pamäť haldy (heap), kde boli uložené dešifrované cookies a heslá, sa **neprepíše nulami**. Dáta zostávajú v RAM a útočník s prístupom k pamäťovému dumpu (alebo cez zraniteľnosti typu use-after-free v C knižniciach) dokáže extrahovať všetky session tokeny uzamknutého prehliadača.
- **Konkrétny návrh opravy / kód:**
  Implementovať `Zeroize` a `ZeroizeOnDrop` pre všetky citlivé štruktúry:
  ```rust
  // src/vault.rs
  use zeroize::{Zeroize, ZeroizeOnDrop};

  #[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
  pub struct CookieRec {
      pub name: String,
      pub value: String,
      pub domain: String,
      pub path: String,
      #[zeroize(skip)]
      pub secure: bool,
      #[zeroize(skip)]
      pub http_only: bool,
      pub same_site: Option<String>,
      #[zeroize(skip)]
      pub expires: Option<f64>,
  }

  #[derive(Serialize, Deserialize, Default, Zeroize, ZeroizeOnDrop)]
  #[serde(default)]
  pub struct VaultData {
      pub history: Vec<HistoryEntry>,
      pub bookmarks: Vec<Bookmark>,
      pub downloads: Vec<Download>,
      pub cookies: Vec<CookieRec>,
      pub session: Vec<String>,
      #[zeroize(skip)]
      pub active: usize,
      #[zeroize(skip)]
      pub settings: Option<Settings>,
      #[zeroize(skip)]
      pub blocked_total: u64,
  }
  ```

- **Stav (v0.1.1): Opravené**
  - Pri uvoľnení sa nulami prepíšu `CookieRec` (`name`, `value`, `domain`), `HistoryEntry`, `Bookmark`, `Download` aj URL relácie (`Urls`, `serde(transparent)`, takže formát trezora sa nezmenil). Platí to pri zamknutí, nahradení zoznamu aj orezaní histórie.
  - Namiesto `derive(ZeroizeOnDrop)` je to ručný `Drop`: `serde(default)` na `VaultData` nedovoľuje `Drop` priamo na tejto štruktúre.
  - **Zostávajúce riziko:** posledná kópia cookies zostáva v RAM počas behu, lebo je potrebná na synchrónne uloženie pri ukončení zo systému. Kópie, ktoré si drží WebKit, Crow vymazať nevie.

---

### [Nález 1.5] Falošná indikácia zabezpečenia (TLS Green Lock Spoofing) a chýbajúce ošetrenie IDN homografov
- **Závažnosť:** Vysoká
- **Kde:** `src/toolbar.html` (riadky 284–287); `src/app.rs` – funkcia `host_of`
- **Popis problému a scenár zlyhania:**
  1. V `toolbar.html` je stav zámku riadený jednoduchým regulárnym výrazom:
     ```javascript
     const proto = /^https:/i.test(st.url) ? 'https' : /^http:/i.test(st.url) ? 'http' : '';
     wrap.classList.toggle('https', proto === 'https');
     ```
     Prehliadač zobrazí používateľovi ikonu zamknutého TLS zámku len preto, že URL začína na `https:`, **bez ohľadu na to, či je certifikát platný**, či nebol expirovaný, self-signed alebo či stránka neobsahuje zmiešaný obsah (mixed content).
  2. Funkcia `host_of` v `src/app.rs` parsuje doménu vlastnou implementáciou (`split_once("://")`), ktorá nepodporuje IDN (Internationalized Domain Names) ani ochranu pred homografickým útokom (napr. zámena latinského `a` za cyrilické `а` v `pаypal.com`). Používateľ vidí v paneli dôveryhodnú adresu s platným HTTPS zámkom.
- **Konkrétny návrh opravy / kód:**
  1. Stav ikony zabezpečenia sa musí určovať na backende v Ruste z overených výsledkov TLS (`cert_chain`) a posielať v `state`:
     ```rust
     // src/app.rs v sync()
     let tls_status = if !url.starts_with("https:") {
         "insecure"
     } else if let Some((_, errors)) = s.view().and_then(wk::cert_chain) {
         if errors.is_empty() { "secure" } else { "warning" }
     } else {
         "warning"
     };
     ```
  2. Použiť štandardizovanú knižnicu pre URL parsing a punycode konverziu (napr. `url::Url` a `idna` crate).

- **Stav (v0.1.1): Opravené**
  - Stav zabezpečenia určuje backend z overenia WebKitom (`serverTrust` + `hasOnlySecureContent` na macOS, `tls_info` bez chýb na Linuxe) ako `secure` / `mixed` / `broken` / `http`. Panel nástrojov aj okno informácií o stránke zobrazia zámok len pri `secure`.
  - **IDN homografy:** WebKit vracia URL v punycode. Overené na macOS: `https://аpple.com/` (cyrilské „а“) sa zobrazí ako `https://xn--pple-43d.com/`, takže homograf v adresnom riadku nevyzerá ako latinská doména. Na Linuxe neoverené.

---

# 2. Úniky pamäte a správa procesov (Memory Leaks & Lifecycle)

---

### [Nález 2.1] Permanentný únik WebProcess inštancií na Linuxe (WebKitGTK)
- **Závažnosť:** Kritická
- **Kde:** `src/wk_gtk.rs` – funkcia `attach`; `src/app.rs` – funkcia `drop_view`
- **Popis problému a scenár zlyhania:**
  Na Linuxe `attach` vloží `webkit2gtk::WebView` priamo do kontajnera `gtk::Layout`:
  ```rust
  let layout = LAYOUT.with(|l| l.borrow().clone()).expect("init_window not called");
  let v = b.build_gtk(&layout)?;
  ```
  V GTK pridanie widgetu do kontajnera (`layout`) zvýši referenčný počet (ref count) daného GtkWidgetu. Keď sa v Ruste zatvorí alebo uspí karta, v `Tab::drop_view` sa vykoná `self.view.take()`, čím sa dropne len Rust obálka `wry::WebView`. 
  Pretože sa nezavolá `layout.remove(&widget)` ani `widget.destroy()`, GTK kontajner si naďalej drží referenciu na `WebKitWebView`. **Celý WebProcess, DOM strom, JavaScript kontext a grafické buffery karty zostávajú navždy alokované v RAM.** Pri uspatí 10 kariet nedôjde k žiadnemu uvoľneniu pamäte.
- **Konkrétny návrh opravy / kód:**
  V `wk_gtk.rs` vytvoriť funkciu na explicitné zničenie GTK widgetu a volať ju v `drop_view`:
  ```rust
  // src/wk_gtk.rs
  pub fn destroy_view(v: &WebView) {
      let w = v.webview();
      if let Some(parent) = w.parent().and_then(|p| p.downcast::<gtk::Layout>().ok()) {
          parent.remove(&w);
      }
      unsafe {
          gtk::prelude::WidgetExt::destroy(&w);
      }
  }

  // src/wk.rs (macOS dummy verzia)
  pub fn destroy_view(_v: &WebView) {}

  // src/app.rs
  impl Tab {
      fn drop_view(&mut self) {
          if let Some(v) = self.view.take() {
              wk::forget_blocked(&v);
              wk::destroy_view(&v);
          }
      }
  }
  ```

- **Stav (v0.1.1): Opravené**
  - wry pri zrušení GTK webview volal `gtk_widget_destroy` aj predtým (`webkitgtk/mod.rs`, `impl Drop for InnerWebView`), takže permanentný únik, ako ho audit opisuje, nenastával.
  - Napriek tomu `wk::forget()` teraz výslovne odoberie widget z `gtk::Layout` a zavolá `destroy()`. `impl Drop for Tab` zaručí rovnaké uvoľnenie pri každom zániku karty (zatvorenie, uspanie, zamknutie, ukončenie).
  - **Neoverené:** linuxová časť nebola skompilovaná ani spustená.

---

### [Nález 2.2] Nekontrolované hromadenie stavu v RAM (`s.closed` a `s.data.downloads`)
- **Závažnosť:** Stredná
- **Kde:** `src/app.rs` – funkcie `close_tab`, `download_started`
- **Popis problému a scenár zlyhania:**
  Kým história v `s.data.history` je striktne ohraničená konštantou `MAX_HISTORY = 20_000`, zoznam zatvorených kariet `s.closed` a zoznam stiahnutí `s.data.downloads` nemajú žiadny limit:
  ```rust
  // src/app.rs
  if tab.url.starts_with("http") {
      s.closed.push(tab.url.clone()); // Nekonečný rast
  }
  ```
  Každá zatvorená karta a každé začaté sťahovanie sa neustále ukladajú do pamäte. Navyše `s.data.downloads` sa pri každom uložení trezora serializuje do `vault.bin`, čo spôsobuje neustály nárast veľkosti trezora na disku a zbytočnú záťaž pri šifrovaní.
- **Konkrétny návrh opravy / kód:**
  Zastropovať obe kolekcie pevnou veľkosťou (FIFO):
  ```rust
  // src/app.rs
  const MAX_CLOSED_TABS: usize = 50;
  const MAX_DOWNLOADS: usize = 500;

  // v close_tab():
  if tab.url.starts_with("http") {
      s.closed.push(tab.url.clone());
      if s.closed.len() > MAX_CLOSED_TABS {
          s.closed.remove(0);
      }
  }

  // v download_started():
  s.data.downloads.push(Download { ... });
  if s.data.downloads.len() > MAX_DOWNLOADS {
      s.data.downloads.remove(0);
  }
  ```

- **Stav (v0.1.1): Opravené (v0.1.1)**
  `MAX_CLOSED_TABS = 50` (najstaršia URL ide preč) a `MAX_DOWNLOADS = 500`. Odstraňujú sa najstaršie dokončené položky, prebiehajúce sťahovania zostávajú, lebo podľa nich sa dohľadáva ich dokončenie.

---

### [Nález 2.3] Únik referencií v `WATCHED` mape pri uzamknutí prehliadača
- **Závažnosť:** Stredná
- **Kde:** `src/wk.rs` – `watch_blocked`, `forget_blocked`; `src/app.rs` – `after_save(After::Lock)`
- **Popis problému a scenár zlyhania:**
  V `src/wk.rs` sa surové smerníky na `WKWebView` registrujú do thread-local mapy:
  ```rust
  static WATCHED: RefCell<HashMap<usize, u32>> = RefCell::new(HashMap::new());
  ```
  Pri uzamknutí prehliadača (`After::Lock`) kód vykoná `self.s.take()`, čo okamžite dealokuje `Session` a celý vektor `s.tabs`. Jednotlivé karty sa však nerušia cez `drop_view()`, takže `wk::forget_blocked(&v)` sa **nikdy nezavolá**.
  Surové adresy dealokovaných objektov zostávajú visieť v `WATCHED`. Pri opätovnom odomknutí a vytvorení nových `WKWebView` môže systémový alokátor prideliť rovnakú pamäťovú adresu (ABA problém), čo spôsobí nesprávne mapovanie ID kariet a počítanie blokovaných elementov.
- **Konkrétny návrh opravy / kód:**
  V `after_save` pri uzamknutí explicitne uvoľniť všetky pohľady:
  ```rust
  // src/app.rs
  After::Lock => {
      self.restore_by = None;
      if let Some(mut s) = self.s.take() {
          for t in &mut s.tabs {
              t.drop_view();
          }
          for a in &s.asks {
              wk::decide_download(a.key, false, &a.url, None);
          }
          wk::clear_store(&s.store);
      }
      // ...
  }
  ```

- **Stav (v0.1.1): Opravené**
  Mapa `WATCHED` podľa adresy pamäte je preč. Číslo karty sa ukladá priamo na WKWebView (`objc_setAssociatedObject`) a zaniká spolu s ním, takže recyklovaná adresa udalosť nesprávnej karte nepriradí. Čísla kariet sa nikdy nepoužijú znova. Zamknutie uvoľní všetky karty cez `impl Drop for Tab`.

---

# 3. Overenie AdBlock enginu a sieťového filtra

---

### [Nález 3.1] Zlyhanie filtrovania pred odoslaním požiadavky pri navigácii a popup oknách
- **Závažnosť:** Vysoká
- **Kde:** `src/app.rs` – funkcie `apply_rules`, `page_load`, `popup`; `src/wk_gtk.rs` – `config`
- **Popis problému a scenár zlyhania:**
  Prehliadač deklaruje, že sieťové požiadavky sú zachytávané a blokované priamo pred odoslaním paketu na sieť. V implementácii sa však nachádzajú dve zásadné medzery:
  1. **WebKitGTK (Linux) v `config` ignoruje parameter `adblock`:**
     V `src/wk_gtk.rs` vyzerá funkcia takto:
     ```rust
     pub fn config(_store: &Store, _adblock: bool) -> Config {
         Config(root())
     }
     ```
     Na Linuxe sa pri vytvorení webview pravidlá do `UserContentManager` vôbec nezaregistrujú. Pravidlá sa aplikujú až ex-post v `activate()` volaním `apply_rules()`. Ak sa karta otvorí na pozadí alebo cez navigáciu pred aktiváciou, **úvodné HTTP požiadavky, skripty tretieho rádu a trackery sa odošlú do siete bez akéhokoľvek filtrovania**.
  2. **Popup okná (`window.open` / `target=_blank`):**
     V `TabCtx::popup` sa vytvára pohľad s `wk::popup_config(features)`. Až následne sa v `adopt()` volá `apply_rules(&self.cfg, &mut tab, "")`. V čase medzi vytvorením WebKitu a priradením pravidiel WebKit už začal načítavať cieľovú URL a jej sub-resourcy bez aktívnych pravidiel.
- **Konkrétny návrh opravy / kód:**
  Pravidlá musia byť naviazané priamo v momente inicializácie konfigurácie WebKitu ešte pred vytvorením inštancie `WebView`:
  ```rust
  // src/wk_gtk.rs
  pub fn config(_store: &Store, adblock: bool) -> Config {
      let view = webkit2gtk::WebView::with_context(&root().context().unwrap());
      if adblock {
          if let Some(manager) = view.user_content_manager() {
              RULES.with(|r| {
                  for (_, f) in &*r.borrow() {
                      unsafe { webkit_user_content_manager_add_filter(manager.as_ptr() as *mut c_void, f.0) };
                  }
              });
          }
      }
      Config(view)
  }
  ```
  A v `TabCtx::popup` v `src/app.rs`:
  ```rust
  // Nastaviť pravidlá ešte pred odoslaním NewWindowResponse
  if self.cfg.adblock {
      wk::set_rules(&view, true);
  }
  ```

- **Stav (v0.1.1): Opravené**
  `TabCtx::build` vytvorí webview bez adresy, pripojí pravidlá (`wk::set_rules`) a až potom zavolá `load_url`. Platí to pre karty aj pre okná z `window.open`, ktoré dostanú pravidlá skôr, ako ich WebKit začne načítavať. Na Linuxe to v `config()` nejde, lebo wry vytvára vlastný `UserContentManager` až pri vytvorení webview. Pravidlá sa preto pripájajú hneď za ním, pred prvou požiadavkou.

---

### [Nález 3.2] Narušenie integrity pravidiel pri orezaní (Trunkácia na `MAX_RULES`)
- **Závažnosť:** Stredná
- **Kde:** `src/adblock.rs` – funkcia `build`
- **Popis problému a scenár zlyhania:**
  WebKit obmedzuje počet pravidiel v jednom zozname na 150 000 (`MAX_RULES`). Súčet pravidiel z EasyList + EasyPrivacy + vlastných pravidiel tento limit bežne presahuje (často > 180 000 pravidiel).
  Kód v `adblock.rs` rieši prekročenie takto:
  ```rust
  let over = rules.len().saturating_sub(MAX_RULES);
  rules.drain(..over);
  ```
  Zoznam pravidiel vygenerovaný knižnicou `adblock` obsahuje pravidlá zoradené podľa typu a špecifickosti. Jednoduché orezanie prvých $N$ pravidiel cez `drain(..over)`:
  1. Náhodne odstráni kritické blokovacie pravidlá na začiatku poľa.
  2. Ponechá výnimky (`@@` – unblock pravidlá), ktorých pôvodné blokovacie pravidlá boli zmazané, čím vzniká nekonzistentný stav filtra.
  3. Pri aktualizácii zoznamov sa poradie mení, čo vedie k nepredvídateľnému správaniu (raz reklama prejde, inokedy nie).
- **Konkrétny návrh opravy / kód:**
  Namiesto slepého odrezania začiatku poľa implementovať deterministickú prioritizáciu podľa závažnosti a rozdeliť pravidlá do viacerých zoznamov (WebKit povoľuje 150 000 pravidiel **na jeden zoznam**, pričom jeden `UserContentController` môže obsahovať viacero zoznamov):
  ```rust
  // src/adblock.rs
  pub const LISTS: [&str; 3] = ["crow-easylist", "crow-easyprivacy", "crow-hosts"];
  ```

- **Stav (v0.1.1): Opravené**
  Každý zdroj (vstavaný + `blocklist.txt`, EasyList, EasyPrivacy, hosts) má vlastnú skupinu zoznamov. Skupina nad 150 000 pravidiel sa rozdelí na viac zoznamov a výnimky (`ignore-previous-rules`) sa skopírujú do každého z nich. Nič sa neodrezáva: kozmetické pravidlá by vypadli len vtedy, keby nestačil ani maximálny počet zoznamov. Overené testom so zníženým limitom.

---

### [Nález 3.3] Zraniteľnosť SSRF a DoS pri aktualizácii zoznamov cez systémový `curl`
- **Závažnosť:** Stredná
- **Kde:** `src/adblock.rs` – funkcia `fetch`; `src/update.rs` – funkcia `curl`
- **Popis problému a scenár zlyhania:**
  Sťahovanie zoznamov filtrov a aktualizácií používa priame volanie externého binárneho súboru `curl`:
  ```rust
  let ok = update::curl()
      .args(["--max-time", "120", "--max-filesize"])
      .arg(MAX_LIST_BYTES.to_string())
      .arg("-o")
      .arg(&part)
      .arg(s.url)
      .status()
  ```
  1. Ak je v prostredí používateľa nastavená premenná `ALL_PROXY`, `HTTP_PROXY` alebo existuje konfiguračný súbor `~/.curlrc`, `curl` ich automaticky prevezme, čo môže viesť k presmerovaniu sťahovania zoznamov cez neoverený lokálny proxy server.
  2. Chýba parameter `--noproxy "*"` a parameter `-q` (ktorý ignoruje `~/.curlrc`).
  3. Prebieha zápis priamo do `data_dir().join("*.part")` bez bezpečných oprávnení `0o600` – ak iný proces na zdieľanom stroji sleduje priečinok, môže modifikovať obsah zoznamu pred tým, ako ho Rust načíta do RAM.
- **Konkrétny návrh opravy / kód:**
  V `update.rs` zabezpečiť izoláciu procesu `curl`:
  ```rust
  // src/update.rs
  pub fn curl() -> Command {
      let mut c = Command::new("curl");
      c.arg("-q"); // Ignorovať ~/.curlrc
      c.args([
          "-fsSL",
          "--proto", "=https",
          "--proto-redir", "=https",
          "--connect-timeout", "10",
      ]);
      c.arg("-A").arg(format!("Crow/{CURRENT}"));
      c
  }
  ```

- **Stav (v0.1.1): Opravené**
  `curl -q` (ignoruje `~/.curlrc`), `-fsSL`, `--proto =https`, `--proto-redir =https`, `--noproxy *`, `--connect-timeout 10` a limit veľkosti. Súbory `.part` sa vytvárajú s právami `0600` ešte pred spustením `curl`.

---

# 4. Chýbajúce komponenty a slepé miesta (Architektonické medzery)

---

### [Nález 4.1] Úplná absencia varovnej obrazovky pri chybách certifikátov (Bypass HTTPS Certificate Validation)
- **Závažnosť:** Kritická
- **Kde:** `src/app.rs` – spracovanie chýb TLS; `src/wk_gtk.rs` – `load_failed_with_tls_errors`
- **Popis problému a scenár zlyhania:**
  Moderné prehliadače pri neplatnom certifikáte (expirovaný, self-signed, nesprávne CN, neznáma CA) okamžite zablokujú zobrazenie stránky a zobrazia používateľovi plnoobrazovkové varovanie (Interstitial Warning Page).
  1. V CrowBrowser na Linuxe (WebKitGTK) nie je implementovaný signál `load-failed-with-tls-errors`. WebKitGTK v predvolenom nastavení navigáciu buď úplne preruší generickou chybou, alebo v závislosti od verzie dovolí prechod.
  2. Ak dôjde k MITM útoku, používateľ nedostane žiadne kontextové varovanie s vysvetlením rizika. V `siteinfo.html` sa chyba zobrazí až vtedy, keď používateľ **manuálne klikne na zámok** v adresnom riadku.
- **Konkrétny návrh opravy / kód:**
  Implementovať obsluhu TLS zlyhania a zobraziť internú chybovú stránku:
  ```rust
  // src/wk_gtk.rs
  w.connect_load_failed_with_tls_errors(|_w, uri, cert, errors| {
      eprintln!("TLS validation failed for {uri}: {errors:?}");
      true // true = zastaviť načítanie, nepovoliť výnimku bez explicitného zásahu
  });
  ```

- **Stav (v0.1.1): Opravené**
  - Overené na macOS 12.7.6 (badssl.com): WebKit už predtým odmietol self-signed, expired, wrong.host aj untrusted-root s kódom `-1202`, pretože wry na overenie certifikátu neodpovedá. Obchádzanie overenia teda nenastávalo, chýbala len informácia pre používateľa.
  - Teraz sa zobrazí statická chybová stránka bez možnosti pokračovať (macOS: `didFailProvisionalNavigation`, kódy -1200 až -1206). Adresný riadok ukáže odmietnutú adresu s označením „Chyba certifikátu“.
  - Linux: politika TLS chýb je výslovne `Fail` a `load-failed-with-tls-errors` vracia `true`.

---

### [Nález 4.2] Nekontrolované prepisovanie a absencia kontroly nebezpečných prípon pri sťahovaní
- **Závažnosť:** Vysoká
- **Kde:** `src/wk_gtk.rs` – funkcia `unique_path`; `src/app.rs` – funkcia `download_started`
- **Popis problému a scenár zlyhania:**
  1. **Dvojité prípony a maskovanie:** Funkcia `unique_path` parsuje len poslednú bodku:
     ```rust
     let (stem, ext) = match name.rsplit_once('.') { ... };
     ```
     Ak web pošle súbor `faktura.pdf.exe` alebo `faktura.pdf.desktop`, funkcia vytvorí `faktura.pdf (1).exe` alebo dovolí stiahnutie spustiteľných súborov priamo do priečinka `Downloads`.
  2. **Absencia karantény (macOS Quarantine Attribute):** Na macOS chýba pridanie atribútu `com.apple.quarantine`. Súbor stiahnutý cez CrowBrowser neprejde kontrolou Apple Gatekeeper pri pokuse o jeho spustenie používateľom vo Finderi.
- **Konkrétny návrh opravy / kód:**
  Na macOS pridať atribút karantény na stiahnutý súbor:
  ```rust
  #[cfg(target_os = "macos")]
  fn mark_quarantine(path: &Path) {
      let _ = std::process::Command::new("xattr")
          .args(["-w", "com.apple.quarantine", "0081;00000000;Crow;"])
          .arg(path)
          .status();
  }
  ```

- **Stav (v0.1.1): Opravené**
  - Názov súboru sa čistí (`src/download.rs`): odstránia sa cesty, riadiace a smer meniace znaky (U+202E), úvodné bodky a výplň medzerami. Maskovacia prípona sa zneškodní: `faktura.pdf.exe` sa uloží ako `faktura_pdf.exe`.
  - Spustiteľné typy majú vo výzve na stiahnutie červené upozornenie.
  - macOS: `com.apple.quarantine` = `0081;<čas>;Crow;` sa zapíše cez `setxattr`, nie cez proces `xattr` (na Monterey je to Python skript). Overené cez `xattr -p`. Platí aj pre inštalátor aktualizácie.

---

### [Nález 4.3] Absencia obrany proti MIME Sniffingu a podvrhnutiu typu obsahu
- **Závažnosť:** Stredná
- **Kde:** `src/wk_gtk.rs` – funkcia `start_downloads`
- **Popis problému a scenár zlyhania:**
  V `start_downloads` prebieha rozhodovanie na základe `Content-Disposition` a `r.is_mime_type_supported()`. Ak server odošle skript alebo HTML s hlavičkou `Content-Type: text/plain` alebo bez hlavičky, WebKit vykoná MIME sniffing. Útočník môže obísť politiku sťahovania tým, že donúti prehliadač interpretovať súbor ako vykonateľný skript v kontexte pôvodnej domény.
- **Konkrétny návrh opravy / kód:**
  Vynútiť rešpektovanie hlavičky `X-Content-Type-Options: nosniff` v sieťovom kontexte a pri nepodporovaných typoch nepovoliť automatické inline renderovanie.

- **Stav (v0.1.1): Čiastočne opravené**
  - Linux: HTTP(S) stránka bez `Content-Type`, alebo s `nosniff` a typom odlišným od deklarovaného, sa nevykreslí. Hlavná stránka ide do výzvy na stiahnutie, iframe sa zahodí.
  - macOS: `nosniff` rešpektuje WebKit sám. Chýbajúci `Content-Type` WebKit dourčí, lebo rozhodovanie o odpovedi patrí delegátovi wry.

---

### [Nález 4.4] Nekonzistentná atomickosť zápisu pri zmene hesla v `change-pw`
- **Závažnosť:** Stredná
- **Kde:** `src/app.rs` – spracovanie správy `change-pw`; `src/vault.rs` – `save`
- **Popis problému a scenár zlyhania:**
  Pri zmene hesla v `src/app.rs`:
  ```rust
  let vault = Vault::create(&r.new);
  s.snapshot();
  s.data.settings = Some(self.cfg.clone());
  match vault.save(&s.data) {
      Ok(()) => {
          s.vault = vault;
          s.dirty = false;
          (true, "Heslo bolo zmenené")
      }
      Err(e) => (false, "Heslo sa nepodarilo uložiť, platí pôvodné")
  }
  ```
  Ak `vault.save()` uspeje na disku, ale bezprostredne po premenovaní súboru proces spadne skôr, ako sa vykoná `s.vault = vault;`, v pamäti zostáva starý stav, no na disku je už nový trezor. Pri páde používateľ nevie, ktoré heslo platí.
- **Konkrétny návrh opravy / kód:**
  Implementovať rotáciu kľúča ako jednu atomickú metódu priamo v štruktúre `Vault`:
  ```rust
  // src/vault.rs
  pub fn rotate_key(&mut self, new_password: &str, data: &VaultData) -> io::Result<()> {
      let mut new_salt = [0u8; 16];
      OsRng.fill_bytes(&mut new_salt);
      let new_key = derive(new_password, &new_salt);
      let new_vault = Vault { salt: new_salt, key: new_key };
      new_vault.save(data)?;
      *self = new_vault;
      Ok(())
  }
  ```

- **Stav (v0.1.1): Opravené**
  `Vault::rotate_key`: nová soľ z `OsRng`, Argon2id, zápis do `.vault.tmp`, `sync_all`, premenovanie a `fsync` priečinka. Až potom sa kľúč vymení v pamäti (starý sa vynuluje). Pri páde medzi premenovaním a výmenou v pamäti je na disku konzistentný trezor s novým heslom, ktoré používateľ práve zadal.

---

### [Nález 4.5] Zraniteľnosť `defuser.js` voči DOM Clobberingu a detekcii anti-adblock skriptami
- **Závažnosť:** Nízka
- **Kde:** `src/defuser.js`
- **Popis problému a scenár zlyhania:**
  1. Premenné návnady (`window.canRunAds`, `window.isAdBlockActive`) sú definované bez ochrany atribútov. Stránka s DOM Clobberingom (napr. `<img id="canRunAds">`) ich môže prepísať elementom namiesto booleovskej hodnoty.
  2. Prepisovanie `HTMLElement.prototype.offsetHeight` bez zachovania natívneho reťazca funkcie je triviálne detegovateľné modernými anti-adblock skriptami.
- **Konkrétny návrh opravy / kód:**
  Použiť `Object.defineProperty` s `configurable: false` a `writable: false`.

- **Stav (v0.1.1): Opravené**
  Návnadové premenné sú `writable: false, configurable: false`. Náhrady funkcií (`offsetHeight` a ďalšie gettery, `fetch`, samotné `toString`) vracajú pri `toString()` zdroj natívnej funkcie a majú jej `name`. Overené vo WebKite: `<img id="canRunAds">` ani priradenie hodnotu nezmenia a `__lookupGetter__('offsetHeight').toString()` vráti `[native code]`.

---

## Tabuľka súhrnu auditu a priorít

| ID | Oblasť | Závažnosť | Priorita riešenia | Stav v0.1.1 |
| :--- | :--- | :--- | :--- | :--- |
| **1.1** | macOS UI WebViews nepoužívajú ephemeral kontext | **Kritická** | Okamžite (porušenie hlavného cieľa projektu) | Opravené |
| **2.1** | Linux WebProcess memory leak v `gtk::Layout` | **Kritická** | Okamžite (vedie k vyčerpaniu RAM) | Opravené |
| **4.1** | Absencia varovania pred neplatným TLS certifikátom | **Kritická** | Okamžite (riziko MITM) | Opravené |
| **1.2** | Nechránený `window.ipc` bez CSP politík | **Vysoká** | Vysoká | Čiastočne (CSP áno, typovaný IPC nie) |
| **1.3** | Race condition pri ukladaní cookies a relácie | **Vysoká** | Vysoká | Opravené |
| **1.4** | Chýbajúci `Zeroize` na dešifrovaných cookies v RAM | **Vysoká** | Vysoká | Opravené |
| **1.5** | TLS zámok indikovaný čisto podľa schémy `https:` | **Vysoká** | Vysoká | Opravené |
| **3.1** | Neaplikované AdBlock pravidlá na Linuxe a v popupoch | **Vysoká** | Vysoká | Opravené |
| **4.2** | Chýbajúca karanténa a kontrola nebezpečných prípon | **Vysoká** | Stredná | Opravené |
| **2.2** | Nekontrolovaný rast `closed` a `downloads` | **Stredná** | Stredná | Opravené |
| **2.3** | ABA problém a dangling referencie vo `WATCHED` | **Stredná** | Stredná | Opravené |
| **3.2** | Nedeterministické orezávanie zoznamu pravidiel | **Stredná** | Stredná | Opravené |
| **3.3** | Curl volanie bez izolácie prostredia | **Stredná** | Stredná | Opravené |
| **4.4** | Zlyhanie synchronizácie pri rotácii kľúča trezora | **Stredná** | Nízka | Opravené |
| **4.5** | Detekovateľnosť a zraniteľnosť `defuser.js` | **Nízka** | Nízka | Opravené |
| **4.3** | MIME sniffing bez obrany | **Stredná** | Stredná | Čiastočne (Linux áno, macOS natívne) |
