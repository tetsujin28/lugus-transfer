# Lugus Transfer

Transfert pair à pair de très gros fichiers vidéo, sans serveur à maintenir.

L'émetteur partage un lien (`lugus://r/<clé>`) et un code (`4821-lanterne-7`).
Le récepteur clique sur le lien, tape le code, et les fichiers arrivent en direct
d'une machine à l'autre, chiffrés de bout en bout et vérifiés par blocs.

Lugus est le dieu messager des Celtibères. Noctiluca, la déesse lunaire de la
côte tartessienne, donne son nom au module de découverte réseau.

## État

Étape 2 : les deux machines se trouvent par la DHT, s'authentifient avec le
code, puis l'émetteur envoie le manifeste et le récepteur sa sélection.
Aucun fichier n'est encore transféré (étape 3).

| Crate | Rôle | État |
|---|---|---|
| `core` | lien, code (Damm), limiteur d'essais, messages, encodage | écrit, non compilé |
| `noctiluca` | découverte par la DHT, endpoint iroh, trames, chemin utilisé | écrit, non compilé |
| `auth` | SPAKE2 lié à la connexion, étiquettes de confirmation | écrit, non compilé |
| `store` | manifeste (BLAKE3, blocs, reprise et MHL à venir) | en partie |
| `cli` | binaire `lugus` (`send`, `receive`, `code`) | écrit, non compilé |
| `transfer` | sessions et ordonnanceur de blocs | à faire |

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

Test de l'étape 2, sur deux machines :

```
lugus send <fichier ou dossier>              # affiche le lien, le code et le contenu
lugus receive lugus://r/<clé> --code <code>  # s'authentifie et reçoit le manifeste
```

Pour tester la règle des essais, lancez `receive` avec un code valide mais
différent (par exemple généré par `lugus code`).
