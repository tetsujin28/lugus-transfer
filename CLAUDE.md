# Lugus Transfer

Application gratuite de Domingo Olmo Martin (Dom), directeur de la photographie : transfert de rushes directement d'une machine à l'autre, sans serveur à maintenir, avec un lien et un code. Chaque fichier est relu sur le disque de destination, sans le cache du système, et comparé à la source ; un MHL 1.1 est écrit à l'arrivée.

Windows et macOS (binaire universel). Le code doit rester compatible Linux. Licence MIT. Version : v0.3.0 (bêta). App sœur de [Bandua](https://github.com/tetsujin28/bandua) (déchargement) et d'[Ataecina](https://github.com/tetsujin28/ataecina) (récupération).

**Lire `docs/ROADMAP.md`** avant tout travail de fond : état du projet, priorités, choix et pistes écartées.

## Règles de travail

- **Ne jamais lancer de compilation GitHub Actions sans que Dom le demande explicitement.** Cela inclut « Run workflow » et la publication d'une Release, qui déclenche aussi la compilation.
- **Ne rien compiler non plus en local sans son accord.** Vérifier la syntaxe avec `rustfmt` seulement.
- **Toujours passer par une branche et une pull request**, fusionnée par Dom sur GitHub.
- Quand une tâche peut être faite par un modèle moins puissant (Sonnet, Haiku), **le signaler à Dom avant de la faire**.
- **Aucun tiret cadratin (caractère U+2014)** dans les textes écrits pour Dom : interface, README, messages de commit. Utiliser virgules, points ou parenthèses.
- Échanges, commentaires et messages de commit en français.
- Interface **toujours en trois langues** (français, anglais, espagnol) : toute nouvelle chaîne s'ajoute dans `ui/i18n.js`, dans les trois langues. Les erreurs du moteur restent en français ; l'interface les reconnaît (`ENGINE_ERRORS` dans `ui/app.js`) et les traduit : un nouveau message d'erreur destiné à l'utilisateur s'y ajoute.
- README : `README.md` (anglais, langue principale), `README.fr.md`, `README.es.md`, à tenir synchronisés, comme le mode d'emploi (`docs/GUIDE.md`, `.fr.md`, `.es.md`). Notes de version en anglais, avec FR et ES repliés.
- Captures du mode d'emploi : `docs/tools/captures.py` puis `docs/tools/annoter.py` (mode démo, horloge figée). À refaire quand l'interface change. La page d'aide du site se fabrique ensuite avec `tools/build-guides.py` du dépôt `tetsujin28.github.io`.
- Interface sombre, accent bleu Noctiluca (`#39c6ff`). Le vert et le rouge sont réservés à « vérifié » et « erreur ».
- Crédits « Lugus Transfer by Domingo Olmo Martin » partout, avertissement de non-responsabilité conservé.
- Avant d'écrire du code contre une bibliothèque qui évolue vite (iroh notamment), **lire son code source** dans la version utilisée.

## La fiabilité d'abord

Toute modification du chemin de réception doit préserver :
- l'écriture en `.lugus-part` avec journal `.lugus-journal`, et le renommage final **seulement** après vérification ;
- la relecture sans cache (`crates/store/src/rawio.rs`, repris de Bandua : une correction faite ici est à reporter dans Bandua, et inversement) ;
- la comparaison de chaque bloc et du fichier entier au hash BLAKE3 de la source ;
- l'effacement du journal en cas d'échec de vérification ;
- les tests de pannes simulées. Ajouter un test pour tout nouveau cas.

La liste de mots `crates/core/src/wordlist_fr.txt` ne doit **jamais être réordonnée ni modifiée** : la position du mot entre dans le chiffre de contrôle du code.

## Organisation

```
crates/core        lien, code d'accès (Damm), limiteur d'essais, messages, encodage
crates/noctiluca   découverte par la DHT (pkarr), endpoint iroh, trames, chemin utilisé
crates/auth        SPAKE2 lié à la connexion, étiquettes de confirmation
crates/store       manifeste, réception par blocs, reprise, relecture sans cache, MHL
crates/transfer    service et réception des fichiers ; session.rs : sessions complètes
                   et événements ; power.rs : anti-veille, mode Noctiluca
crates/cli         binaire `lugus` (send, receive, code)
src-tauri/         application Tauri 2 (commandes, événement « lugus »)
ui/                index.html, app.js, i18n.js : HTML/CSS/JS sans npm ni framework
docs/              GUIDE (3 langues), ROADMAP, guide/ (captures annotées),
                   screenshots/ (images du README), tools/ (captures.py, annoter.py)
```

L'interface parle au moteur par `invoke` et reçoit le déroulement par l'événement `lugus` (`lugus_transfer::session::Event`). Sans `window.__TAURI__`, `ui/` tourne en **mode démo** dans un navigateur.

## Publier une version

Changer le numéro dans `Cargo.toml` (version du workspace) et `src-tauri/tauri.conf.json` (la valeur par défaut de `ui/app.js` peut suivre). Mettre à jour `docs/ROADMAP.md` et la version écrite en tête des modes d'emploi. Dom crée ensuite la Release `vX.Y.Z` lui-même : la compilation se lance et y attache les fichiers (`Lugus_Transfer_x.y.z_portable.exe`, `_x64-setup.exe`, `.msi`, `_universal.dmg`, et la ligne de commande `lugus_x.y.z_windows.exe` / `_macos`).

## Commandes

```
cargo test --workspace --exclude lugus-app     # moteur, sans l'application
cargo run -p lugus-cli -- send <dossier>
cargo run -p lugus-cli -- receive <lien> --code <code> --dest <dossier>
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev                # lancer l'application
```
