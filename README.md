# Lugus Transfer

Transfert pair à pair de très gros fichiers vidéo, sans serveur à maintenir.

L'émetteur glisse un dossier dans l'application et partage un lien (`lugus://r/<clé>`) et un code (`4821-lanterne-7`). Le destinataire colle le lien, tape le code, et les fichiers arrivent directement d'une machine à l'autre, chiffrés de bout en bout. Chaque fichier est relu sur le disque de destination, sans le cache du système, et comparé à la source. Un MHL est écrit à l'arrivée.

Lugus est le dieu messager des Celtibères. Noctiluca, la déesse lunaire de la côte tartessienne, donne son nom au module de découverte réseau et au mode de transfert de nuit.

Lugus Transfer by Domingo Olmo Martin. Application sœur de [Bandua](https://github.com/tetsujin28/bandua) et d'[Ataecina](https://github.com/tetsujin28/ataecina).

## Fonctionnement

- **Découverte** : l'émetteur publie ses adresses, signées par une clé éphémère, dans la DHT Mainline de BitTorrent. Aucun serveur à maintenir.
- **Connexion** : QUIC (iroh), directe quand c'est possible (réseau local, IPv6, ports ouverts automatiquement en UPnP, PCP ou NAT-PMP, hole punching). Sinon, repli sur les relais publics d'iroh, où le trafic reste chiffré de bout en bout.
- **Code** : `NNNN-mot-C`, 4 chiffres, un mot parmi 2048 et un chiffre de contrôle (algorithme de Damm). Les fautes de frappe sont détectées en local, sans consommer d'essai. 3 essais par lien, avec un délai croissant. Après 3 échecs, seul le code est régénéré.
- **Authentification** : SPAKE2, liée aux clés des deux machines et à la session TLS. L'émetteur ne prouve qu'il connaît le code qu'après avoir vérifié la preuve du destinataire.
- **Transfert** : blocs de 4 Mio, 4 fichiers en parallèle, proxys et fichiers annexes en premier. Reprise exacte après une coupure. L'émetteur reste en attente, et il suffit de relancer la réception.
- **Vérification** : chaque fichier est relu sans cache, chaque bloc et le fichier entier sont comparés au BLAKE3 de la source. Le fichier reçoit ensuite la date d'origine et son nom définitif.
- **À l'arrivée** : un MHL 1.1 en xxHash64 (`<nom>_lugus_<date>.mhl`, compatible avec Bandua MHL Companion, Silverstack, ShotPut…), et un accusé de réception authentifié envoyé à l'émetteur.
- **Énergie** : la machine reste éveillée pendant toute la session.
- **Mode Noctiluca** : réception programmée à une heure donnée, puis mise en veille ou extinction à la fin.
- **Expiration** : le lien expire au bout de 24 h.

## Application

L'interface (`ui/`) est en HTML/CSS/JS, sans npm, en français, anglais et espagnol. Ouverte dans un navigateur, elle tourne en mode démo avec des données simulées.

## Ligne de commande

```
lugus send <fichier ou dossier> [--apres veille|extinction]
lugus receive <lien> --code <code> --dest <dossier> [--depart 01:00] [--apres veille|extinction]
lugus code [code à vérifier]
```

## Développement

Rust 1.91 au minimum (exigé par iroh 1.4). Pas de compilation automatique : le workflow GitHub `Compilation` se lance à la main (onglet Actions, puis Run workflow) et produit le CLI et l'application pour Windows et macOS (binaire universel).

```
cargo test --workspace --exclude lugus-app
cd src-tauri && cargo tauri dev
```

Voir `CLAUDE.md` pour les règles de travail et l'organisation du code.

## Licence

MIT. Voir `LICENSE`.
