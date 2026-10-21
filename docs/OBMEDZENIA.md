# Známe obmedzenia

Úprimný zoznam toho, čo Crow (0.1.0) zatiaľ nevie alebo robí inak než veľké prehliadače.

## Dáta a prihlásenia

| Obmedzenie | Dôsledok | Prečo |
|---|---|---|
| **localStorage / IndexedDB sa medzi spusteniami neukladajú** | Weby, ktoré si pamätajú prihlásenie len tam (nie v cookies), budú chcieť nové prihlásenie | WebKit beží s úložiskom len v pamäti, aby nič nezapisoval na disk nešifrovane. Ukladajú sa iba cookies, šifrovane |
| Cache sa neukladá | Prvé načítanie stránky po spustení je pomalšie | rovnaký dôvod |
| Ak sa Crow nezatvorí cez ⌘Q, prídeš o posledné zmeny (najviac 30 s) | Napr. pri páde | Ukladá sa každých 30 s |
| Stiahnuté súbory nie sú šifrované | Ležia v ~/Downloads ako bežné súbory | Majú byť použiteľné aj v iných aplikáciách |
| Bez správcu hesiel a automatického vypĺňania | Heslá na weby nezapamätá. Prihlásenie drží cez cookies | Zatiaľ neimplementované |
| Heslo trezora sa nedá obnoviť | Zabudnuté heslo = strata dát | Zámer: žiadne zadné dvierka |

## Prehliadanie

- Bez rozšírení (extensions).
- Bez súkromného okna (celý prehliadač je šifrovaný, ale nie je režim „nič neukladaj“).
- Favicon sa zobrazí len vtedy, keď ho stránka dovolí prečítať (rovnaký pôvod alebo CORS). Inak zostane bodka. Uspané karty po spustení ikonu nemajú, kým sa nenačítajú.
- Bez tlače (⌘P).
- Bez ponuky „Otvoriť odkaz v novej karte“ v kontextovom menu. Funguje však ⌘+klik v paneloch a odkazy `target=_blank`.
- Hľadanie na stránke rozlišuje diakritiku (`š` ≠ `s`), počíta len viditeľný text a nehľadá v iframoch.
- História sa neaktualizuje pri navigácii v jednostránkových aplikáciách (SPA) bez načítania stránky.
- Crow sa nedá nastaviť ako predvolený prehliadač systému (nespracúva otváranie odkazov z iných aplikácií).

## Blokovanie reklám

- Vstavaný zoznam má 105 domén, nie desaťtisíce ako EasyList. Väčší zoznam si môžeš pridať do `blocklist.txt` (najviac 50 000 pravidiel).
- Pravidlá v `blocklist.txt` podporujú len celé domény. Filtre na cesty, výnimky (`@@`) ani kozmetické filtre v štýle uBlock nie sú podporované.
- Nedá sa vypnúť pre jeden web, iba globálne.
- Nepočíta sa, koľko požiadaviek bolo zablokovaných.

## Distribúcia

- Aplikácia nie je notarizovaná, takže na iných Macoch zobrazí upozornenie Gatekeepera.
- Build je len pre Intel (x86_64). Na Apple Silicon beží cez Rosettu, univerzálny build je opísaný v [BUILD.md](BUILD.md).
- Aktualizácie sa len zistia a stiahnu (po kliknutí, overené SHA-256 z GitHubu). Inštaláciu robíš sám, Crow inštalátor nespúšťa. Kontrola potrebuje systémový `curl` a vydanie (Release) na GitHube s tagom `vX.Y.Z` a inštalátorom (`.dmg`; na Linuxe `.pkg.tar.zst`/`.deb`/`.rpm`/`.AppImage`/`.tar.gz`).

## Možné ďalšie kroky

- Správca hesiel uložený v trezore (s automatickým vypĺňaním)
- Šifrované ukladanie localStorage
- Výnimky adblocku pre jednotlivé weby a počítadlo blokovaní
- Kontextové menu s položkou „Otvoriť v novej karte“
- Automatické zamknutie po nečinnosti
- Univerzálny build a notarizácia

## Linux

- Sťahovanie po potvrdení urobí novú GET požiadavku na rovnakú adresu. Súbory vytvorené odoslaním formulára (POST) sa preto nemusia stiahnuť správne. Na macOS to isté platí pre `blob:` odkazy.
- Informácie o stránke na Linuxe neukazujú zmiešaný obsah (HTTP prvky na HTTPS stránke), na macOS áno.

- Okno bez rámu sa nedá zväčšovať ťahaním za okraj (v tiling WM ako Hyprland to nevadí; inak Super+pravé tlačidlo alebo `CROW_DECORATIONS=1`).
- „Semafory“ sú nakreslené v HTML. Zelené tlačidlo okno maximalizuje (na macOS prepína celú obrazovku), celá obrazovka je F11.
- Motív Svetlý/Tmavý sa nastavuje cez GTK (`gtk-application-prefer-dark-theme` a názov témy). Podľa neho sa riadi aj `prefers-color-scheme` stránok.
- Neexistuje systémový panel menu, všetky príkazy sú dostupné cez tlačidlá a skratky.
