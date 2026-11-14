# Lugus Transfer

Transfert pair à pair de très gros fichiers vidéo, sans serveur à maintenir.

L'émetteur partage un lien (`lugus://r/<clé>`) et un code (`4821-lanterne-7`).
Le récepteur clique sur le lien, tape le code, et les fichiers arrivent en direct
d'une machine à l'autre, chiffrés de bout en bout et vérifiés par blocs.

Lugus est le dieu messager des Celtibères. Noctiluca, la déesse lunaire de la
côte tartessienne, donne son nom au module de découverte réseau.

## État

Étape 1 : deux machines se trouvent par la DHT et échangent un message.
Le code n'est pas encore vérifié par l'émetteur, et aucun fichier n'est transféré.

| Crate | Rôle | État |
|---|---|---|
| `core` | lien, code d'accès, limiteur d'essais, messages, états de session | écrit, non compilé |
| `cli` | binaire `lugus` (`send`, `receive`, `code`) | écrit, non compilé |
| `noctiluca` | découverte par la DHT (pkarr), endpoint iroh, chemin utilisé | écrit, non compilé |
| `auth` | SPAKE2 lié à l'identité de la connexion | à faire |
| `store` | BLAKE3, blocs, reprise, MHL | à faire |
| `transfer` | sessions et ordonnanceur de blocs | à faire |

## Règles du code d'accès

- Format : `NNNN-mot-C` (4 chiffres, un mot de la liste, un chiffre de contrôle).
- 3 essais par lien, tous destinataires confondus, avec un délai croissant.
- Le chiffre de contrôle et le mot sont vérifiés en local avant de consommer un essai.
- Après 3 échecs, seul le code est régénéré, le lien reste valable.

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

Test de l'étape 1, sur deux machines :

```
lugus send <fichier>                         # affiche le lien et le code
lugus receive lugus://r/<clé> --code <code>  # trouve l'émetteur et échange un message
```
