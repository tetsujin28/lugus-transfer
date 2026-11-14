# Lugus Transfer

Transfert pair à pair de très gros fichiers vidéo, sans serveur à maintenir.

L'émetteur partage un lien (`lugus://r/<clé>`) et un code (`4821-lanterne-7`).
Le récepteur clique sur le lien, tape le code, et les fichiers arrivent en direct
d'une machine à l'autre, chiffrés de bout en bout et vérifiés par blocs.

Lugus est le dieu messager des Celtibères. Noctiluca, la déesse lunaire de la
côte tartessienne, donne son nom au module de découverte réseau.

## État

Étape 3 : découverte par la DHT, authentification par code, puis transfert
complet des fichiers avec reprise et vérification par relecture du disque.
L'interface Tauri et le MHL viendront ensuite.

| Crate | Rôle | État |
|---|---|---|
| `core` | lien, code (Damm), limiteur d'essais, messages, encodage | écrit, non compilé |
| `noctiluca` | découverte par la DHT, endpoint iroh, trames, chemin utilisé | écrit, non compilé |
| `auth` | SPAKE2 lié à la connexion, étiquettes de confirmation | écrit, non compilé |
| `store` | manifeste, réception par blocs, journal de reprise, liste `b3sum` | écrit, non compilé |
| `transfer` | service des fichiers, réception parallèle, accusé de réception | écrit, non compilé |
| `cli` | binaire `lugus` (`send`, `receive`, `code`) | écrit, non compilé |

## Transfert et intégrité

- Blocs de 4 Mio. L'émetteur calcule le BLAKE3 de chaque bloc et du fichier
  entier pendant la lecture, et les envoie avec les données.
- Le récepteur écrit dans `<fichier>.lugus-part` et note le hash de chaque bloc
  dans `<fichier>.lugus-journal`. Après une coupure, il suffit de relancer
  `lugus receive` avec le même lien et le même code : la réception reprend au
  dernier bloc écrit. L'émetteur reste en attente pendant ce temps.
- À la fin de chaque fichier, il est **relu depuis le disque** : chaque bloc et
  le hash complet sont comparés à ceux de la source. Le fichier reçoit ensuite
  la date de modification d'origine et prend son nom définitif.
- Un fichier déjà présent à destination avec la bonne taille est hashé et
  comparé à la source au lieu d'être téléchargé.
- 4 fichiers en parallèle par défaut (`--parallel`), proxys et fichiers annexes
  avant les originaux.
- À la fin, `<nom>.lugus.b3` liste les hash (vérifiable avec `b3sum --check`).
  Le récepteur envoie un accusé contenant l'empreinte de cette liste,
  authentifié avec la clé de session.
- Le MHL (ASC MHL) viendra plus tard : il n'accepte pas BLAKE3 et demandera un
  hash xxHash en plus.

## Règles du code d'accès

- Format : `NNNN-mot-C` (4 chiffres, un mot de la liste, un chiffre de contrôle).
- Chiffre de contrôle de Damm : toute erreur sur un chiffre et toute inversion
  de deux chiffres voisins sont repérées en local, sans consommer d'essai.
- 3 essais par lien, tous destinataires confondus, avec un délai croissant.
  Chaque essai est réservé dès son début : abandonner en cours de route le
  consomme quand même.
- Après 3 échecs, seul le code est régénéré, le lien reste valable.

## Authentification

1. récepteur → `Hello`, puis `Pake` (SPAKE2, côté B)
2. émetteur → `Pake` (côté A), après avoir réservé un essai
3. récepteur → `Confirm` : sa preuve
4. émetteur → `Confirm` s'il a vérifié la preuve, sinon `Error(WrongCode)`
5. émetteur → `Manifest`, récepteur → `Select`
6. un flux par fichier : `Request`, puis `Block` + données, puis `FileDone`
7. récepteur → `Receipt`

L'échange est lié à la connexion : identités SPAKE2 = clés iroh des deux pairs,
et clé dérivée avec un secret exporté de la session TLS (RFC 5705).
L'émetteur ne révèle sa preuve qu'après avoir vérifié celle du récepteur.

## Connectivité

- Découverte : DHT Mainline uniquement (`iroh-mainline-address-lookup`), aucun
  serveur DNS iroh.link.
- Ports : ouverture automatique par iroh (UPnP, PCP, NAT-PMP).
- Hole punching : coordonné par les relais publics d'iroh, qui servent aussi de
  repli. Le trafic y reste chiffré de bout en bout.

## Développement

Rust 1.91 au minimum (exigé par iroh 1.4).
Pas de compilation automatique : le projet est compilé à la main, sur demande.

```
cargo build --release
```

Tests unitaires (sans réseau) : `cargo test`.

Test de l'étape 3, sur deux machines :

```
lugus send <fichier ou dossier>                             # affiche le lien et le code
lugus receive lugus://r/<clé> --code <code> --dest <dossier>  # reçoit et vérifie
```

Pour tester la reprise, interrompez `receive` (Ctrl+C) en cours de route, puis
relancez la même commande.

Pour tester la règle des essais, lancez `receive` avec un code valide mais
différent (par exemple généré par `lugus code`).
