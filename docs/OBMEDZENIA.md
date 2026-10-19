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
- Bez viacerých okien, všetko je v kartách jedného okna.
- Karty sa nedajú presúvať ťahaním.
- Bez ikon webov (favicon) na kartách, namiesto nich je bodka alebo indikátor načítavania.
- Bez vývojárskych nástrojov (Web Inspector).
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
- Bez automatických aktualizácií.

## Možné ďalšie kroky

- Správca hesiel uložený v trezore (s automatickým vypĺňaním)
- Šifrované ukladanie localStorage
- Výnimky adblocku pre jednotlivé weby a počítadlo blokovaní
- Kontextové menu s položkou „Otvoriť v novej karte“
- Presúvanie kariet ťahaním a ikony webov (favicony)
- Automatické zamknutie po nečinnosti
- Univerzálny build a notarizácia
