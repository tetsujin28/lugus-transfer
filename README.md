# Lugus Transfer

Transfert pair à pair de très gros fichiers vidéo, sans serveur à maintenir.

L'émetteur partage un lien (`lugus://r/<clé>`) et un code (`4821-lanterne-7`).
Le récepteur clique sur le lien, tape le code, et les fichiers arrivent en direct
d'une machine à l'autre, chiffrés de bout en bout et vérifiés par blocs.

Lugus est le dieu messager des Celtibères. Noctiluca, la déesse lunaire de la
côte tartessienne, donne son nom au module de découverte réseau.

## État

Squelette en cours. Rien n'est encore connecté au réseau.

| Crate | Rôle | État |
|---|---|---|
| `core` | lien, code d'accès, limiteur d'essais, messages, états de session | écrit, non compilé |
| `cli` | binaire `lugus` (`send`, `receive`, `code`) | écrit, non compilé |
| `noctiluca` | découverte pkarr/DHT, iroh, boîte aux lettres, UPnP/PCP | à faire |
| `auth` | SPAKE2 lié à l'identité de la connexion | à faire |
| `store` | BLAKE3, blocs, reprise, MHL | à faire |
| `transfer` | sessions et ordonnanceur de blocs | à faire |

## Règles du code d'accès

- Format : `NNNN-mot-C` (4 chiffres, un mot de la liste, un chiffre de contrôle).
- 3 essais par lien, tous destinataires confondus, avec un délai croissant.
- Le chiffre de contrôle et le mot sont vérifiés en local avant de consommer un essai.
- Après 3 échecs, seul le code est régénéré, le lien reste valable.

## Développement

Pas de compilation automatique : le projet est compilé à la main, sur demande.

```
cargo build --release
```
