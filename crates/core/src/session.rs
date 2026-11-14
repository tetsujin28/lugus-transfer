//! États d'une session, côté émetteur et côté récepteur.
//!
//! Chaque transition sera enregistrée dans le journal de session (crate `store`),
//! ce qui rend la reprise fiable après une coupure ou un redémarrage.

/// États de l'émetteur : préparation → publié → attente → authentification →
/// transfert → terminé (ou expiré, ou échec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderState {
    Preparing,
    Published,
    Waiting,
    Authenticating,
    Transferring,
    Done,
    Expired,
    Failed,
}

/// États du récepteur : résolution → connexion → authentification → sélection →
/// réception → vérification → terminé (ou échec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverState {
    Resolving,
    Connecting,
    Authenticating,
    Selecting,
    Receiving,
    Verifying,
    Done,
    Failed,
}

impl SenderState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Expired | Self::Failed)
    }
}

impl ReceiverState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states() {
        assert!(!SenderState::Waiting.is_terminal());
        assert!(SenderState::Expired.is_terminal());
        assert!(!ReceiverState::Receiving.is_terminal());
        assert!(ReceiverState::Done.is_terminal());
    }
}
