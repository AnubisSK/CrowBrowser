# Bezpečnosť a šifrovanie

## Zhrnutie

Všetky osobné dáta prehliadača sú na disku v jednom zašifrovanom súbore, **trezore**. Bez hesla sa nedajú prečítať ani upraviť. Heslo sa nikde neukladá a bez neho sa dáta nedajú obnoviť.

## Čo je šifrované

| Dáta | Šifrované | Poznámka |
|---|---|---|
| História prehliadania | ✅ | až 20 000 záznamov |
| Obľúbené stránky | ✅ | |
| Cookies a prihlásenia na weboch | ✅ | WebKit ich na disk vôbec nezapisuje, pozri nižšie |
| Otvorené karty (relácia) | ✅ | |
| Zoznam stiahnutých súborov | ✅ | |
| Nastavenia (vyhľadávač, motív, domovská stránka…) | ✅ | |
| **Stiahnuté súbory** v ~/Downloads | ❌ | bežné súbory na disku |
| `blocklist.txt` | ❌ | verejný zoznam domén, nie je citlivý |
| `easylist.txt`, `easyprivacy.txt`, `hosts-stevenblack.txt`, `crow-adblock.json`, `crow-hosts.json` | ❌ | stiahnuté verejné zoznamy filtrov a kontrolné súčty skompilovaných pravidiel, nie sú citlivé |
| Cache WebKitu, localStorage, IndexedDB | (neukladá sa) | existujú len v pamäti počas behu |

## Trezor

**Súbor:** `~/Library/Application Support/Crow/vault.bin` (oprávnenia `0600`, číta ho len vlastník).

**Formát:**

```
┌────────────┬──────────────┬───────────────┬──────────────────────────────┐
│ "CROWV1"   │ salt         │ nonce         │ ciphertext + Poly1305 tag     │
│ 6 B        │ 16 B         │ 24 B          │ N + 16 B                      │
└────────────┴──────────────┴───────────────┴──────────────────────────────┘
  └── hlavička (22 B) je autentizovaná ako AAD
```

| Časť | Algoritmus / parametre |
|---|---|
| Odvodenie kľúča z hesla | **Argon2id** (v1.3), pamäť 64 MiB, 3 iterácie, paralelizmus 1, 256-bit kľúč |
| Šifrovanie | **XChaCha20-Poly1305** (AEAD), 192-bit náhodný nonce pri každom uložení |
| Salt | 128-bit náhodný (OS CSPRNG), nový pri vytvorení trezora aj pri zmene hesla |
| Integrita | Poly1305 tag a autentizovaná hlavička: akákoľvek zmena súboru = odmietnutie |
| Obsah | JSON (`VaultData`) |

**Zápis je atomický:** dáta sa zapíšu do `vault.tmp`, vykoná sa `fsync` a potom sa súbor premenuje na `vault.bin`. Pád počas ukladania nepoškodí existujúci trezor.

**Argon2id s 64 MiB pamäte** výrazne spomaľuje skúšanie hesiel hrubou silou: každý pokus stojí desiatky až stovky ms a 64 MB RAM. Odomknutie preto trvá približne 0,5 – 1 s.

## Cookies a prihlásenia

Bežný prehliadač ukladá cookies nešifrovane do súborov WebKitu. Crow to robí inak:

1. Všetky karty zdieľajú jedno úložisko WebKitu **len v pamäti** (`WKWebsiteDataStore.nonPersistentDataStore`). WebKit tak na disk nezapíše žiadne cookies, cache ani localStorage.
2. Pri ukladaní Crow cookies z úložiska vyexportuje (`WKHTTPCookieStore.getAllCookies`) a uloží ich do trezora. Zachovajú sa všetky atribúty: doména vrátane úvodnej bodky, cesta, `Secure`, `HttpOnly`, `SameSite` a expirácia.
3. Po odomknutí sa cookies vrátia do WebKitu (`setCookie`). Karty sa začnú načítavať až potom, takže weby ťa hneď poznajú.
4. Exspirované cookies sa pri obnovení zahodia.

## Zamknutie

Po **⌃⌘L** (alebo z Nastavení) Crow:

1. vyexportuje cookies a uloží všetko do trezora,
2. vymaže úložisko WebKitu v pamäti (`removeDataOfTypes`),
3. zavrie všetky karty a panely a zahodí dešifrované dáta aj kľúč (kľúč sa pred uvoľnením vynuluje),
4. prepne okno na systémový motív a zobrazí obrazovku odomknutia.

## Práca s heslom v pamäti

- Kľúč je uložený v `Zeroizing<[u8; 32]>`, pri uvoľnení sa prepíše nulami.
- Správa s heslom z obrazovky odomknutia sa po spracovaní prepíše nulami.
- Pri zmene hesla sa prepíšu nulami staré aj nové heslo.
- Cookies do trezora: naraz beží len jeden export z WebKitu, ďalšie požiadavky sa zaradia za neho. Export spustený pred mazaním cookies (odhlásenie, vymazanie) sa zahodí a zopakuje, takže starší zoznam neprepíše novší ani nevráti vymazané prihlásenie. Každé odomknutie má vlastný jedinečný token: oneskorený export zo zamknutej relácie sa do novej nikdy nezapíše.
- Pri uvoľnení (zamknutie, nahradenie, orezanie) sa nulami prepíše aj história, obľúbené, zoznam stiahnutých a URL otvorených kariet, nielen cookies.
- Zoznam zatvorených kariet (na ⌘⇧T) má najviac 50 položiek, zoznam stiahnutých najviac 500 (najstaršie dokončené idú preč).
- Dešifrovaný obsah trezora sa drží v `Zeroizing` bufferi. Cookies (názov, hodnota, doména) sa prepíšu nulami vždy, keď sa ich zoznam nahradí alebo uvoľní (po exporte z WebKitu, importe, vymazaní, zamknutí). Posledná kópia cookies zostáva v pamäti počas behu, aby sa dali uložiť aj pri nečakanom ukončení. Kópie vo WebKite Crow vymazať nevie.
- **Zmena hesla** (`Vault::rotate_key`): nová náhodná soľ, nový kľúč Argon2id, zápis do `.vault.tmp`, `sync_all` a premenovanie. Nový kľúč sa začne používať až po úspešnom zápise, inak zostáva pôvodné heslo na disku aj v pamäti.

## Zmena a strata hesla

- **Zmena:** Nastavenia → Šifrovanie. Vyžaduje aktuálne heslo. Vygeneruje sa nový salt aj kľúč a trezor sa hneď prešifruje.
- **Strata:** obnova nie je možná, je to zámer. Na obrazovke odomknutia je „Zabudol som heslo – vymazať všetky dáta“: zmaže `vault.bin` a začneš s prázdnym trezorom.

## Ochrana rozhrania prehliadača

- Rozhranie (panel, nastavenia, obrazovka odomknutia) beží v oddelených WebView s vlastným HTML. Len tieto WebView môžu posielať príkazy do Rustu cez IPC.
- **Webové stránky nemajú IPC kanál.** Žiadna stránka nemôže čítať históriu, meniť nastavenia ani zamknúť či odomknúť trezor. wry vkladá `window.ipc` len do WebView s IPC handlerom (rozhranie), karty ani okná z `window.open` ho nemajú.
- Rozhranie má prísne CSP (`default-src 'none'`, len vlastný inline kód, obrázky len `data:`), takže nemôže načítať nič zo siete ani externý skript. Beží v samostatnom dočasnom úložisku (`nonPersistentDataStore` na macOS, vlastný ephemeral kontext na Linuxe), na disk nezapisuje cache ani storage.
- Dáta vkladané do HTML panelu sú escapované (`</` → `<\/`). Názvy stránok a adresy sa vykresľujú cez `textContent`, nie `innerHTML`, takže nemôžu vložiť skript.
- Otvorenie súboru zo zoznamu stiahnutých je povolené len pre cesty, ktoré v zozname naozaj sú.

## Certifikáty a zámok

- Stránka s neplatným certifikátom (vypršaný, self-signed, iná doména, nedôveryhodná autorita) sa **nenačíta**. Crow neodpovedá na overovanie certifikátov, takže rozhoduje systémové overenie WebKitu; na Linuxe je politika TLS chýb výslovne nastavená na „zlyhať“. Výnimku pridať nejde.
- Namiesto stránky sa v karte zobrazí upozornenie „Pripojenie nie je bezpečné“ s dôvodom (statická stránka bez IPC). Adresný riadok ukazuje odmietnutú adresu s červeným označením „Chyba certifikátu“.
- Zámok sa zobrazí **len** keď WebKit pre načítanú stránku potvrdí overený certifikát (macOS `serverTrust`, Linux `tls_info` bez chýb), nikdy len podľa `https://` v adrese. Stránka s HTTP obsahom na HTTPS (macOS) má označenie „Čiastočne zabezpečené“.
- Pravidlá blokovania sa pripoja k novej karte aj k oknu otvorenému cez `window.open` ešte pred prvou požiadavkou.
- **MIME sniffing:** WebKit rešpektuje `X-Content-Type-Options: nosniff`. Na Linuxe Crow navyše nevykreslí HTTP(S) stránku bez hlavičky `Content-Type` ani stránku s `nosniff`, ktorej typ by WebKit určil inak, než deklaroval server. Hlavná stránka ide do výzvy na stiahnutie, iframe sa nevykreslí.
- Skript proti detekcii blokovania nastavuje návnadové premenné ako nemenné (odolné voči DOM clobbering cez `<img id="canRunAds">`) a jeho náhrady funkcií sa pri `toString()` tvária ako natívny kód.

## Sťahovanie a sieť mimo WebKitu

- Názov sťahovaného súboru sa pred zápisom vyčistí (cesty, neviditeľné a smer meniace znaky, maskovacie prípony ako `.pdf.exe`). Spustiteľné typy sú vo výzve na stiahnutie výrazne označené. Na macOS dostane dokončený súbor atribút `com.apple.quarantine` (Gatekeeper).
- Aktualizácie a zoznamy blokovania sťahuje systémový `curl` s `-q` (ignoruje `~/.curlrc`), len cez HTTPS aj pri presmerovaní (`--proto =https --proto-redir =https`), bez proxy z prostredia (`--noproxy *`) a s limitom veľkosti. Rozpracované `.part` súbory majú práva `0600`.

## Model hrozieb

| Hrozba | Chránené? |
|---|---|
| Niekto skopíruje tvoj disk alebo zálohu (Time Machine, cloud) | ✅ trezor je šifrovaný |
| Iný používateľ na tom istom Macu | ✅ trezor je šifrovaný a súbor má práva `0600` |
| Niekto si sadne k tvojmu odomknutému Macu, keď je Crow zamknutý | ✅ |
| Niekto si sadne k Macu, keď je Crow odomknutý | ❌ zamykaj cez ⌃⌘L |
| Malvér bežiaci pod tvojím účtom počas odomknutého Crow | ❌ môže čítať pamäť procesu |
| Keylogger | ❌ |
| Slabé heslo | ⚠️ Argon2id útok spomalí, ale nezastaví. Použi dlhé heslo |
| Sledovanie na sieti | čiastočne: HTTPS zabezpečuje WebKit, adblock blokuje známe sledovače |
