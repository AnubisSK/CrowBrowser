Electron App
Tento projekt je jednoduchá aplikácia postavená na Electron frameworku, ktorá slúži ako základ pre desktopové aplikácie. Aplikácia načíta HTML súbor a poskytuje základné nastavenia pre webové preferencie.

Funkcie
Vytvára okno s rozmermi 800x600 pixelov.
Načítava HTML súbor index.html.
Odstraňuje predvolené menu aplikácie.
Umožňuje načítanie nezabezpečeného obsahu (pouze pre testovanie).
Vypína zabezpečenie webu (pouze pre testovanie).
Umožňuje povolenie pre prístup k médiám.
Požiadavky
Node.js (verzia 12 alebo novšia)
Nainštalovaný Electron
Inštalácia
Klonujte tento repozitár:

bash
Insert Code
Run
Copy code
git clone https://github.com/vas-uzivatel/electron-app.git
cd electron-app
Nainštalujte závislosti:

bash
Insert Code
Run
Copy code
npm install
Spustite aplikáciu:

bash
Insert Code
Run
Copy code
npm start
Konfigurácia
V súbore main.js môžete upraviť nasledujúce nastavenia:

Rozmery okna: Zmeňte hodnoty width a height v BrowserWindow.
Povolenie pre externé URL: Upravte setPermissionRequestHandler pre prispôsobenie povolení.
Bezpečnostné upozornenia
Vypnutie zabezpečenia webu a povolenie načítania nezabezpečeného obsahu je vhodné iba pre testovanie. V produkčnej aplikácii by ste mali zabezpečiť, aby ste tieto možnosti nepoužívali.
Prispievanie
Ak máte nápady na vylepšenia alebo opravy chýb, neváhajte prispieť do tohto projektu. Vytvorte pull request alebo otvorený issue.

Licencia
Tento projekt je licencovaný pod MIT licenciou. Pozrite sa na súbor LICENSE pre viac informácií.
