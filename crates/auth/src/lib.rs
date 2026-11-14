//! Authentification par code, sans entrée-sortie.
//!
//! Le code d'accès sert de mot de passe à un échange SPAKE2 (courbe Ed25519).
//! SPAKE2 garantit qu'un attaquant n'obtient qu'**une supposition par échange** :
//! il ne peut pas tester des codes hors ligne à partir de ce qu'il a vu passer.
//!
//! L'échange est lié à la connexion de trois façons :
//!
//! - les identités SPAKE2 sont les clés publiques iroh des deux pairs, toutes
//!   deux authentifiées par TLS ;
//! - la clé dérivée intègre un secret exporté de la session TLS
//!   (`channel_binding`, RFC 5705), propre à cette connexion ;
//! - chaque côté prouve qu'il a obtenu la même clé avec une étiquette de
//!   confirmation différente selon son rôle, ce qui empêche de renvoyer à
//!   l'émetteur sa propre étiquette.
//!
//! Le récepteur envoie sa preuve en premier. L'émetteur ne révèle la sienne
//! qu'après l'avoir vérifiée : un attaquant qui se trompe de code n'apprend rien.
//! La règle des 3 essais est appliquée par l'appelant (`lugus_core::limiter`).

use spake2::{Ed25519Group, Identity, Password, Spake2};
use thiserror::Error;

/// Étiquette de l'export de clé TLS (RFC 5705), à passer à
/// `Connection::export_keying_material` avec un contexte vide.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-lugus/1-pake";

/// Contexte de dérivation BLAKE3, unique à Lugus et à cette version.
const KDF_CONTEXT: &str = "lugus-transfer 2026-11 handshake v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("message SPAKE2 invalide : {0}")]
    BadPakeMessage(String),
}

/// Rôle dans l'échange. L'émetteur est le côté A de SPAKE2, le récepteur le côté B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Sender,
    Receiver,
}

/// Ce qui lie l'échange à la connexion en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// Clé publique iroh de l'émetteur (celle du lien).
    pub sender_id: [u8; 32],
    /// Clé publique iroh du récepteur.
    pub receiver_id: [u8; 32],
    /// Secret exporté de la session TLS avec `EXPORTER_LABEL`.
    pub channel_binding: [u8; 32],
}

/// Échange SPAKE2 en cours.
pub struct Handshake {
    side: Side,
    context: Context,
    state: Spake2<Ed25519Group>,
}

impl Handshake {
    /// Démarre l'échange. Renvoie l'état et le message à envoyer au pair.
    pub fn start(side: Side, password: &[u8], context: Context) -> (Self, Vec<u8>) {
        let password = Password::new(password);
        let id_a = Identity::new(&context.sender_id);
        let id_b = Identity::new(&context.receiver_id);
        let (state, message) = match side {
            Side::Sender => Spake2::<Ed25519Group>::start_a(&password, &id_a, &id_b),
            Side::Receiver => Spake2::<Ed25519Group>::start_b(&password, &id_a, &id_b),
        };
        (
            Self {
                side,
                context,
                state,
            },
            message,
        )
    }

    /// Termine l'échange avec le message du pair.
    ///
    /// Un mauvais code ne provoque pas d'erreur ici : les deux côtés obtiennent
    /// simplement des clés différentes, ce que révèle la vérification des
    /// étiquettes de confirmation.
    pub fn finish(self, peer_message: &[u8]) -> Result<Session, AuthError> {
        let spake_key = self
            .state
            .finish(peer_message)
            .map_err(|e| AuthError::BadPakeMessage(e.to_string()))?;

        let mut material = Vec::with_capacity(spake_key.len() + 3 * 32);
        material.extend_from_slice(&spake_key);
        material.extend_from_slice(&self.context.channel_binding);
        material.extend_from_slice(&self.context.sender_id);
        material.extend_from_slice(&self.context.receiver_id);
        let base = blake3::derive_key(KDF_CONTEXT, &material);

        Ok(Session {
            side: self.side,
            sender_tag: blake3::keyed_hash(&base, b"confirm sender"),
            receiver_tag: blake3::keyed_hash(&base, b"confirm receiver"),
            key: *blake3::keyed_hash(&base, b"session key").as_bytes(),
        })
    }
}

/// Résultat de l'échange : les étiquettes de confirmation et la clé de session.
pub struct Session {
    side: Side,
    sender_tag: blake3::Hash,
    receiver_tag: blake3::Hash,
    key: [u8; 32],
}

impl Session {
    /// Étiquette à envoyer au pair pour prouver qu'on connaît le code.
    pub fn own_tag(&self) -> [u8; 32] {
        match self.side {
            Side::Sender => *self.sender_tag.as_bytes(),
            Side::Receiver => *self.receiver_tag.as_bytes(),
        }
    }

    /// Vérifie l'étiquette reçue du pair (comparaison en temps constant).
    pub fn verify_peer(&self, tag: &[u8; 32]) -> bool {
        let expected = match self.side {
            Side::Sender => self.receiver_tag,
            Side::Receiver => self.sender_tag,
        };
        expected == blake3::Hash::from_bytes(*tag)
    }

    /// Clé de session partagée, utilisable plus tard (signature de l'accusé, etc.).
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            sender_id: [1u8; 32],
            receiver_id: [2u8; 32],
            channel_binding: [3u8; 32],
        }
    }

    fn run(
        sender_pw: &[u8],
        receiver_pw: &[u8],
        sender_ctx: Context,
        receiver_ctx: Context,
    ) -> (Session, Session) {
        let (sender, msg_a) = Handshake::start(Side::Sender, sender_pw, sender_ctx);
        let (receiver, msg_b) = Handshake::start(Side::Receiver, receiver_pw, receiver_ctx);
        let sender = sender.finish(&msg_b).unwrap();
        let receiver = receiver.finish(&msg_a).unwrap();
        (sender, receiver)
    }

    #[test]
    fn same_code_authenticates_both_sides() {
        let (sender, receiver) = run(b"4821-lanterne", b"4821-lanterne", context(), context());
        assert!(sender.verify_peer(&receiver.own_tag()));
        assert!(receiver.verify_peer(&sender.own_tag()));
        assert_eq!(sender.key(), receiver.key());
    }

    #[test]
    fn wrong_code_fails() {
        let (sender, receiver) = run(b"4821-lanterne", b"4812-lanterne", context(), context());
        assert!(!sender.verify_peer(&receiver.own_tag()));
        assert!(!receiver.verify_peer(&sender.own_tag()));
        assert_ne!(sender.key(), receiver.key());
    }

    #[test]
    fn different_connection_fails() {
        let mut other = context();
        other.channel_binding = [4u8; 32];
        let (sender, receiver) = run(b"4821-lanterne", b"4821-lanterne", context(), other);
        assert!(!sender.verify_peer(&receiver.own_tag()));
    }

    #[test]
    fn own_tag_cannot_be_reflected() {
        let (sender, _) = run(b"4821-lanterne", b"4821-lanterne", context(), context());
        assert!(!sender.verify_peer(&sender.own_tag()));
    }

    #[test]
    fn corrupt_message_is_an_error() {
        let (sender, _) = Handshake::start(Side::Sender, b"code", context());
        assert!(sender.finish(&[0u8; 3]).is_err());
    }
}
