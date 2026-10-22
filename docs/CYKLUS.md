## Životný cyklus a stratégia podpory platforiem (Platform Support Matrix)

Projekt sa zameriava na maximálnu efektivitu hardvérových prostriedkov pri zachovaní moderných bezpečnostných štandardov. Staršie platformy (najmä macOS Monterey) slúžia ako prísny referenčný benchmark pre sub-300MB rozpočet RAM, no ich podpora je časovo ohraničená limitmi systémového WebKitu a Rust toolchainu.

### 1. Plánovaná matica podpory (Roadmap 2026 – 2028)

| Platforma | Architektúra | 2026 (v0.1.x) | 2027 (v0.2.x) | 2028 (v1.0 LTS) | Dôvod plánovaného ukončenia / Zmeny |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **macOS 12 (Monterey)** | `x86_64` | **Plná podpora** | *Deprecated* | **Ukončená** | Zastarané jadro WebKitu, chýbajúce moderné API (`WKWebExtension`), vyradenie z Xcode SDK. |
| **macOS 13 (Ventura)** | `x86_64`, `arm64` | **Plná podpora** | **Plná podpora** | *Deprecated* | Prechodná fáza; postupný ústup podpory pre staršie Intel čipy. |
| **macOS 14 (Sonoma)+** | `x86_64`, `arm64` | **Plná podpora** | **Plná podpora** | **Plná podpora** | Moderný štandard; natívne systémové API pre rozšírenia a izoláciu. |
| **Linux (WebKitGTK 4.1)** | `x86_64`, `aarch64` | **Plná podpora** | *Legacy/Fallback* | **Ukončená** | Starší GTK3 stack; obmedzená správa procesov a chýbajúci GPU kompozitor GSK. |
| **Linux (WebKitGTK 6.0)** | `x86_64`, `aarch64` | *Experimentálne* | **Plná podpora** | **Primárne jadro** | Čistý GTK4/Wayland stack, natívna správa `WebKitMemoryPressureSettings`, GSK rendering. |
| **Windows 10 / 11** | `x86_64` | *Neplánované* | *Experimentálne* | **Voliteľné (WebView2)** | Sekundárny cieľ; striktná ephemeral relácia cez Microsoft Edge WebView2 runtime. |

---

### 2. Kľúčové míľniky a podmienky ukončenia podpory (Deprecation Policy)

- **macOS Monterey (macOS 12.x):**
  - **Fáza podpory (2026):** Slúži ako primárny nízkoenergetický testovací benchmark (Intel Core i5, 4 GB RAM). Udržiava sa `MACOSX_DEPLOYMENT_TARGET=12.0`.
  - **Ukončenie (2027/2028):** Podpora bude oficiálne ukončená v momente, keď novšie vydanie crate `wry`/`tao` alebo Rust toolchainu začne vyžadovať symboly z macOS 13+, prípadne keď moderné weby prestanú na starom WebKite fungovať bez rozsiahlych polyfillov. Finálny stav pre Monterey bude zakonzervovaný v samostatnom vydaní (napr. `v0.2.x-legacy-monterey`).

- **Linux – Prechod z WebKitGTK 4.1 na 6.0:**
  - **Fáza 4.1:** Zabezpečuje spätnú kompatibilitu so staršími distribúciami (Ubuntu 22.04 LTS). Správa pamäte sa opiera o ručné ukončovanie podprocesov pri uspaní kariet.
  - **Prechod na 6.0:** Akonáhle bude GTK4 stack vo `wry` plne stabilizovaný, verzia 6.0 prevezme rolu predvoleného jadra vďaka modernej správe procesov a nižšej réžii pri vykresľovaní pod Waylandom.
