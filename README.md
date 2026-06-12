# CrowBrowser

Moderný webový prehliadač pre Linux postavený na **WebKitGTK 6**, **GTK4** a **libadwaita**. Dizajn inšpirovaný Arc.net — tmavý postranný panel so záložkami.

![Platform](https://img.shields.io/badge/platform-Linux-blue)
![Language](https://img.shields.io/badge/language-Vala-purple)
![License](https://img.shields.io/badge/license-GPL--3.0-green)

---

## Funkcie

### Prehliadanie
- Tmavý postranný panel so záložkami (Arc.net štýl)
- Rýchla navigácia: Späť / Vpred / Obnoviť
- Fullscreen podpora (F11 / Escape)
- Zoom stránky, plynulé rolovanie
- Klávesové skratky pre všetky akcie

### Súkromie a bezpečnosť
- **Súkromné okno** — ephemeral session, história ani cookies sa neukladajú (Ctrl+Shift+N)
- **Tor okno** — ephemeral session + SOCKS5 proxy cez Tor (automaticky detekuje port 9050/9150) (Ctrl+Shift+T)
- **Plnohodnotný adblocker** — sieťový filter + CSS kozmetický filter + JS blokovanie pop-upov
- **Šifrovaná história** — HMAC-SHA256 CTR šifrovanie, kľúč odvodený od machine-ID
- HTTPS-only režim, blokovanie autoplay médií

### Sťahovanie videí
- Integrácia s **yt-dlp** pre sťahovanie videí
- Automatická detekcia video streamov (HLS/DASH/m3u8/mpd)
- Podpora streamovacích prehrávačov: JW Player, Video.js, HLS.js, Shaka, dash.js
- DRM/Widevine podpora (ak je nainštalovaný Chromium/Widevine CDM)

### Nastavenia (6 stránok)
| Stránka | Obsah |
|---------|-------|
| Všeobecné | Spustenie (nová karta / domovská stránka / obnoviť reláciu), vyhľadávač, plynulé rolovanie |
| Zobrazenie | Veľkosť písma, predvolené priblíženie |
| Sťahovania | Priečinok, otvoriť po stiahnutí |
| Súkromie | Adblocker, HTTPS-only, JavaScript, HW akcelerácia, blokovanie autoplay, história, vymazanie cookies/cache |
| Oprávnenia | Kamera, mikrofón, poloha, notifikácie (Spýtať sa / Povoliť / Blokovať) |
| O aplikácii | Verzia, engine, licencia |

---

## Požiadavky

```
gtk4
libadwaita-1 >= 1.5
webkitgtk-6.0
gee-0.8
vala >= 0.56
meson >= 0.62
ninja
```

### Arch Linux
```bash
sudo pacman -S gtk4 libadwaita webkitgtk-6.0 libgee vala meson ninja
```

### Ubuntu / Debian (24.04+)
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev \
                 libgee-0.8-dev valac meson ninja-build
```

### Fedora
```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 libgee-devel vala meson ninja-build
```

---

## Inštalácia

```bash
git clone https://github.com/AnubisSK/CrowBrowser.git
cd CrowBrowser
meson setup build
ninja -C build
./build/crow-browser
```

---

## Tor okno

Pre Tor okno je potrebný bežiaci Tor démon:

```bash
# Arch Linux
sudo pacman -S tor
sudo systemctl start tor

# Alebo Tor Browser (port 9150)
yay -S tor-browser
```

CrowBrowser automaticky detekuje port 9050 (systemd tor) aj 9150 (Tor Browser).

---

## Klávesové skratky

| Skratka | Akcia |
|---------|-------|
| Ctrl+T | Nová karta |
| Ctrl+W | Zatvoriť kartu |
| Ctrl+Shift+N | Nové súkromné okno |
| Ctrl+Shift+T | Nové Tor okno |
| Ctrl+L / F6 | Fokus na URL bar |
| Ctrl+R / F5 | Obnoviť stránku |
| Ctrl+H | História |
| Ctrl+J | Správca sťahovania |
| Ctrl+D | Stiahnuť video |
| Ctrl+B | Zobraziť/skryť panel |
| Ctrl+, | Nastavenia |
| F12 | Nástroje pre vývojárov |
| Alt+← / Alt+→ | Späť / Vpred |

---

## Štruktúra projektu

```
src/
├── main.vala              — vstupný bod
├── application.vala       — Adw.Application, akcie, CSS štýly, session management
├── browser-window.vala    — hlavné okno, sidebar, URL bar, tab management
├── web-tab.vala           — WebKitWebView wrapper, video detekcia
├── adblock.vala           — adblocker (sieťový filter + CSS + JS)
├── history-manager.vala   — história s šifrovaním
├── history-dialog.vala    — dialóg histórie
├── settings-manager.vala  — nastavenia (KeyFile)
├── settings-dialog.vala   — UI nastavení (6 stránok)
├── download-manager.vala  — správca sťahovania
├── downloads-dialog.vala  — UI sťahovania
├── video-downloader.vala  — integrácia yt-dlp
├── crypto-utils.vala      — HMAC-SHA256 CTR šifrovanie
└── extension-manager.vala — správca rozšírení
data/
├── crow-browser.gresource.xml
└── icons/
```

---

## Licencia

GNU General Public License v3.0 — pozri [LICENSE](LICENSE)
