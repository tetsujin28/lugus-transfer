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
//! 8. transfert des blocs (crate `transfer`, pas encore défini ici)
//! 9. récepteur → `Receipt` : accusé de réception signé avec le hash final
//!
//! L'émetteur ne révèle sa preuve qu'après avoir vérifié celle du récepteur :
//! un attaquant qui se trompe de code n'apprend rien.

use serde::{Deserialize, Serialize};

/// Hash BLAKE3 de 32 octets.
pub type Hash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello { version: u8 },
    Pake { payload: Vec<u8> },
    Confirm { tag: [u8; 32] },
    Manifest(Manifest),
    Select(Selection),
    Receipt(Receipt),
    Error(ProtocolError),
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
    pub final_hash: Hash,
    pub signature: Vec<u8>,
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
