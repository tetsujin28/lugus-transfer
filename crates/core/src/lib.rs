//! Cœur de Lugus Transfer.
//!
//! Ce crate ne fait ni réseau ni accès disque. Il contient le vocabulaire commun
//! de l'application : le format du lien, le code d'accès, la règle des essais,
//! les messages du protocole et les états d'une session.

pub mod code;
pub mod limiter;
pub mod link;
pub mod protocol;
pub mod session;

/// Version du protocole, annoncée dans le message `Hello`.
pub const PROTOCOL_VERSION: u8 = 1;

/// Identifiant de protocole (ALPN) utilisé par la connexion QUIC.
pub const ALPN: &[u8] = b"lugus/1";

/// Schéma des liens : `lugus://r/<clé>`.
pub const LINK_SCHEME: &str = "lugus";

/// Nombre d'essais de code autorisés par lien, tous destinataires confondus.
pub const MAX_CODE_ATTEMPTS: u32 = 3;

/// Durée de validité par défaut d'un lien, en secondes (24 h).
pub const DEFAULT_LINK_TTL_SECS: u64 = 24 * 60 * 60;
