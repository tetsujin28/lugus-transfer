//! Authentification par code.
//!
//! Ce crate fera l'échange SPAKE2 à partir de `AccessCode::password()`, lié à
//! l'identité de la connexion QUIC, et appliquera la règle des essais de
//! `lugus_core::limiter`. Rien n'est écrit pour l'instant.
