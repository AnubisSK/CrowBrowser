# Známe obmedzenia

Úprimný zoznam toho, čo Crow (0.1.2) zatiaľ nevie alebo robí inak než veľké prehliadače.

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

- Filtre sa prevádzajú na pravidlá WebKitu, takže fungujú len tie, ktoré sa dajú vyjadriť v jeho formáte. Procedurálne kozmetické filtre uBlock (`:has-text`, scriptlety) nie sú podporované a reklamy vložené priamo do videa (YouTube) sa nezablokujú.
- Výnimky (`@@`) platia len v rámci svojho zdroja: výnimka z EasyListu nezruší blokovanie z EasyPrivacy ani zo StevenBlack hosts.
- Karanténny atribút pre Gatekeeper existuje len na macOS. Na Linuxe sa súbory ukladajú bez práva spustenia.
- Prísna kontrola MIME (stránka bez `Content-Type` sa nevykreslí) je len na Linuxe. Na macOS rozhoduje wry/WebKit sám (`nosniff` rešpektuje, chýbajúci typ však WebKit dourčí).
- Počítanie zablokovaných požiadaviek používa neverejné rozhranie WebKitu na macOS (`_webView:contentRuleListWithIdentifier:performedAction:forURL:`). Ak ho budúci macOS zmení, počty zostanú na nule, blokovanie funguje ďalej. Na Linuxe WebKitGTK také rozhranie nemá, počty sa nezobrazujú.
- Ušetrené dáta a čas sú odhad (50 kB a 50 ms na požiadavku), nie meranie.

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
