//! Messages échangés entre l'émetteur et le récepteur (ALPN `lugus/1`).
//!
//! Poignée de main, sur un flux bidirectionnel ouvert par le récepteur :
//!
//! 1. récepteur → `Hello` : version du protocole
//! 2. récepteur → `Pake` : son message SPAKE2 (rôle B)
//! 3. émetteur → `Pake` : son message SPAKE2 (rôle A), ou `Error`
//! 4. récepteur → `Confirm` : preuve qu'il connaît le code
//! 5. émetteur → `Confirm` : sa propre preuve, ou `Error(WrongCode)`
//! 6. émetteur → `Manifest` : le contenu, envoyé uniquement après authentification
//! 7. récepteur → `Select` : fichiers choisis et ordre
//! 8. transfert : pour chaque fichier, le récepteur ouvre un nouveau flux
//!    bidirectionnel et envoie `Request`. L'émetteur répond par une suite de
//!    `Block`, chacun suivi de ses octets bruts, puis par `FileDone`.
//!    Pendant ce temps, le récepteur envoie régulièrement `Status` sur le flux
//!    de contrôle (octets reçus et vérifiés).
//! 9. récepteur → `Receipt` sur le flux de contrôle : accusé de réception
//!
//! L'émetteur ne révèle sa preuve qu'après avoir vérifié celle du récepteur :
//! un attaquant qui se trompe de code n'apprend rien.

/// Taille d'un bloc de données. Chaque bloc a son hash BLAKE3.
pub const BLOCK_SIZE: u64 = 4 * 1024 * 1024;

use serde::{Deserialize, Serialize};

/// Hash BLAKE3 de 32 octets.
pub type Hash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello {
        version: u8,
    },
    Pake {
        payload: Vec<u8>,
    },
    Confirm {
        tag: [u8; 32],
    },
    Manifest(Manifest),
    Select(Selection),
    /// Demande d'un fichier (indice dans le manifeste) à partir d'un décalage,
    /// toujours multiple de `BLOCK_SIZE` (reprise).
    Request {
        file: u32,
        offset: u64,
    },
    /// En-tête d'un bloc : `len` octets bruts suivent immédiatement sur le flux.
    /// `hash` est le BLAKE3 de ces octets, calculé par l'émetteur à la lecture.
    Block {
        offset: u64,
        len: u32,
        hash: Hash,
    },
    /// Fin du fichier. `full_hash` est le BLAKE3 du fichier source entier :
    /// en reprise, l'émetteur relit aussi la partie déjà envoyée pour le
    /// calculer. Le récepteur le compare à sa relecture du disque.
    FileDone {
        full_hash: Option<Hash>,
    },
    Receipt(Receipt),
    Error(ProtocolError),
    /// Avancement côté récepteur, envoyé régulièrement sur le flux de contrôle
    /// pendant le transfert : octets reçus et octets déjà relus et vérifiés sur
    /// son disque. Permet à l'émetteur d'afficher la vérification à distance.
    Status {
        received: u64,
        verified: u64,
    },
}

/// Rôle d'un fichier dans le manifeste, pour envoyer les proxys d'abord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Original,
    Proxy,
    Sidecar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Chemin relatif, séparateur `/`, jamais absolu et sans `..`.
    pub path: String,
    pub size: u64,
    /// Date de modification, en secondes depuis l'epoch Unix.
    pub modified_unix: Option<i64>,
    pub kind: FileKind,
    /// Hash racine BLAKE3, absent tant que le hashage en flux n'est pas terminé.
    pub root_hash: Option<Hash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Choix du récepteur : indices dans `Manifest::files`, dans l'ordre voulu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub files: Vec<u32>,
    pub proxies_first: bool,
}

/// Accusé de réception, signé par le récepteur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub file_count: u32,
    pub total_bytes: u64,
    /// BLAKE3 de la liste des hash (format `b3sum`) écrite par le récepteur.
    pub final_hash: Hash,
    /// Authentifiant : BLAKE3 avec clé (clé de session) de `final_hash`.
    /// Prouve à l'émetteur que l'accusé vient bien du destinataire authentifié.
    /// Ce n'est pas une signature vérifiable par un tiers.
    pub mac: Hash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolError {
    UnsupportedVersion {
        supported: u8,
    },
    /// Code refusé. `remaining` indique les essais restants pour ce lien.
    WrongCode {
        remaining: u32,
    },
    /// Trop tôt après un essai raté : réessayer dans `wait_secs` secondes.
    TooSoon {
        wait_secs: u64,
    },
    /// Les essais sont épuisés, l'émetteur doit générer un nouveau code.
    CodeLocked,
    /// Message inattendu ou mal formé.
    BadMessage,
    /// Le fichier source a changé depuis la création du manifeste.
    SourceChanged,
    /// Le fichier source ne peut pas être lu.
    FileUnavailable,
    LinkExpired,
    Busy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_total_bytes() {
        let manifest = Manifest {
            name: "J04".to_string(),
            files: vec![
                FileEntry {
                    path: "A001/clip1.mov".to_string(),
                    size: 100,
                    modified_unix: None,
                    kind: FileKind::Original,
                    root_hash: None,
                },
                FileEntry {
                    path: "A001/clip1_proxy.mp4".to_string(),
                    size: 20,
                    modified_unix: Some(1_731_000_000),
                    kind: FileKind::Proxy,
                    root_hash: Some([7u8; 32]),
                },
            ],
        };
        assert_eq!(manifest.total_bytes(), 120);
    }
}
