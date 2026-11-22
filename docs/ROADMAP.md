# Lugus Transfer : où j'en suis et ce qui vient

Mes notes de travail sur Lugus Transfer : l'état du projet, mes choix et la suite.
Dernière mise à jour : 22 novembre 2026 (badge de version et réglages).

## Le projet

Lugus Transfer est mon application gratuite de transfert de rushes, pour Windows et macOS. Elle envoie des fichiers vidéo très lourds directement d'un ordinateur à l'autre, avec un lien et un code, sans serveur à maintenir. Chaque fichier est relu sur le disque du destinataire et comparé à la source, et un MHL est écrit à l'arrivée.

Je ne voulais pas d'un énième service de transfert avec un serveur au milieu et un stockage temporaire : je voulais que les rushes aillent d'une machine à l'autre, et rien d'autre, avec le même niveau de vérification que sur le plateau. Le fonctionnement ressemble à un petit client BitTorrent privé : un lien, une recherche dans la DHT, puis une connexion directe.

Lugus est le dieu messager des Celtibères, dieu des chemins et des échanges. Noctiluca, la déesse lunaire de la côte tartessienne, donne son nom au module de découverte réseau et au mode de transfert de nuit.

C'est l'application sœur de **Bandua** (déchargement), de **Bandua MHL Companion** (contrôle des rushes reçus) et d'**Ataecina** (récupération et réparation).

- Dépôt : https://github.com/tetsujin28/lugus-transfer (public)
- Site : https://tetsujin28.github.io
- Version : v0.2.0 (pré-version bêta), publiée le 16 novembre 2026

## Feuille de route, par ordre de priorité

### 1. Première version (V0.1) ✅ fait
- ✅ Découverte par la DHT Mainline (pkarr), connexion QUIC avec iroh, directe quand c'est possible, sinon par les relais publics d'iroh, chiffrée de bout en bout
- ✅ Lien `lugus://r/<clé>` et code `NNNN-mot-C` (liste de 2048 mots, chiffre de contrôle de Damm), 3 essais par code avec un délai croissant, nouveau code sans changer de lien, lien valable 24 h
- ✅ Authentification SPAKE2 liée aux clés des deux machines et à la session TLS
- ✅ Transfert par blocs de 4 Mio, 4 fichiers en parallèle, proxys et annexes d'abord
- ✅ Écriture en `.lugus-part` avec journal `.lugus-journal`, relecture sans cache reprise de Bandua, comparaison de chaque bloc et du fichier entier au BLAKE3 de la source, renommage seulement après vérification
- ✅ Reprise exacte après n'importe quelle coupure, avec le même avancement affiché des deux côtés
- ✅ MHL 1.1 en xxHash64 à l'arrivée, qui n'écrase jamais un MHL existant ; accusé de réception authentifié
- ✅ Choix des fichiers à l'envoi et à la réception (originaux, proxys, annexes, fichier par fichier)
- ✅ Dépôt d'un dossier, d'un fichier ou de plusieurs fichiers d'un même dossier ; disque sans nom reçu sous `Rushes_<date>`
- ✅ Information claire à chaque étape : étapes numérotées, phrase d'état, badge « En ligne » quand le lien est publié, vérification visible après 100 %, MHL et accusé
- ✅ Recherche de l'expéditeur relancée toute seule pendant 90 s (au lieu de l'erreur « No addressing information »)
- ✅ Anti-veille, mode Noctiluca (réception programmée, puis veille ou extinction)
- ✅ Interface en français, anglais et espagnol, y compris les messages d'erreur du moteur
- ✅ README, mode d'emploi avec captures annotées, page sur le site

### 1 bis. Version 0.2 : plusieurs sources
- ✅ Envoyer des rushes qui viennent de plusieurs endroits (deux disques, ou deux dossiers placés à des endroits différents) en un seul lien
- ✅ Chaque dépôt ou choix s'ajoute à ce qui est déjà là au lieu de le remplacer ; une ✕ par source, et « Tout retirer » pour recommencer
- ✅ À l'arrivée : un dossier `Rushes_<date>` avec un sous-dossier par source, nommé comme le dossier ou le disque (`_2` si deux sources ont le même nom)
- ✅ Un dossier déjà inclus dans un dossier ajouté n'est pas envoyé deux fois
- ✅ La ligne de commande accepte plusieurs chemins : `lugus send <chemin> <chemin>`

### 1 ter. Version et réglages ✅ fait (à publier)
- ✅ Numéro de version en haut à droite ; au lancement, une seule requête à GitHub (pré-versions comprises) ; « vX.Y.Z disponible » et clic vers la page des Releases ; silence hors ligne
- ✅ Fenêtre de réglages (roue dentée) avec « Vérifier les nouvelles versions », désactivable
- Même fonction à reporter dans Bandua, Bandua MHL Companion et Ataecina

### 2. Tests en conditions réelles ✅ premiers tests faits
- Transferts entre mon PC et mon Mac, à distance, sur des réseaux différents (fibre, 4G, réseau d'hôtel ou de bureau très fermé) pour voir quand le relais prend le relais
- Provoquer les pannes : couper le réseau, mettre en veille, débrancher le disque de destination, fermer la fenêtre, relancer très vite
- Gros volumes : une journée de tournage complète, plusieurs centaines de Go, la nuit en mode Noctiluca
- Faire tester une bêta à quelques collègues (monteurs, DIT)

### 3. Prochaines étapes
- Codes d'erreur dans le moteur, plutôt que la reconnaissance des messages français par l'interface
- Limite de débit réglable (dans les Réglages), pour ne pas saturer le réseau d'un plateau ou d'un bureau
- Temps restant plus lisible : lissé, estimation de la vérification après 100 %, heure d'arrivée prévue

### 4. Mis de côté
- Signature des applications (Windows et macOS), pour supprimer l'avertissement au premier lancement
- Une boîte aux lettres facultative (sur un serveur à soi) pour déposer les rushes quand les deux machines ne peuvent pas être allumées en même temps
- Historique des envois et des réceptions
- Remettre Linux le jour où je pourrai le tester

## Mes choix

- **Aucun serveur à maintenir.** La découverte passe par la DHT publique de BitTorrent ; les relais publics d'iroh ne servent qu'en dernier recours, et ne voient que des données chiffrées.
- **Le code ne voyage jamais.** SPAKE2 : un attaquant qui a le lien n'a que 3 essais, et l'expéditeur ne révèle sa preuve qu'après avoir vérifié celle du destinataire.
- **La vérification se fait sur le disque, pas en mémoire.** Même relecture sans cache que Bandua : une correction faite dans `rawio.rs` est à reporter dans Bandua, et inversement.
- **La liste de mots ne change jamais** : la position du mot entre dans le chiffre de contrôle du code.
- **Deux canaux pour le lien et le code**, rappelés dans l'interface.
- **Un lien sert à une livraison**, d'une machine à une seule autre.

## Pistes écartées

- **QR code du lien** : retiré, le lien se copie et s'envoie, le QR code n'apportait rien.
- **Stockage temporaire sur un serveur** façon service de transfert : contraire à l'idée de départ.
- **Contre-vérification par l'expéditeur** de la liste des empreintes de l'accusé : la comparaison est déjà faite par le destinataire avec les empreintes de l'expéditeur. À rouvrir si besoin.
- **Plusieurs destinataires pour un même lien, ou entraide entre destinataires** façon BitTorrent : écarté, Lugus reste strictement de machine à machine, pour la sécurité.
- **Mise à jour automatique en un clic** : demande une clé de signature propre à l'updater (à ne jamais perdre), des fichiers en plus dans la compilation, et ne marche pas pour la version portable. Le badge qui ouvre la page de téléchargement suffit.
- **Phrase sur le capot des portables** dans l'interface : jugée incompréhensible, l'explication est dans le mode d'emploi.

## Aide-mémoire

```
cargo test --workspace --exclude lugus-app     # moteur, sans l'application
cargo run -p lugus-cli -- send <dossier>
cargo run -p lugus-cli -- receive <lien> --code <code> --dest <dossier>
cd src-tauri && cargo tauri dev                # lancer l'application
python docs/tools/captures.py                  # captures du mode d'emploi (mode démo)
python docs/tools/annoter.py                   # cadres et pastilles, images du README
```

Publier une version : changer le numéro dans `Cargo.toml` (version du workspace) et `src-tauri/tauri.conf.json` (la valeur par défaut de `ui/app.js` peut suivre), mettre à jour ce fichier, puis Dom crée la Release `vX.Y.Z` sur GitHub : la compilation se lance et y attache les fichiers.
