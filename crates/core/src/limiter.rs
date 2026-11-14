//! Règle des essais de code : 3 essais par lien, tous destinataires confondus,
//! avec un délai croissant entre deux essais.
//!
//! Le compteur vit côté émetteur. Le temps est passé en paramètre pour que la
//! logique reste testable sans horloge réelle.

use std::time::{Duration, Instant};

use crate::MAX_CODE_ATTEMPTS;

/// Délai imposé après le 1er échec, puis après le 2e. Au-delà, on garde le dernier.
const DELAYS_SECS: [u64; 2] = [2, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Un essai peut être tenté maintenant.
    Allowed,
    /// Il faut attendre avant le prochain essai.
    WaitFor(Duration),
    /// Les essais sont épuisés : l'émetteur doit générer un nouveau code.
    Locked,
}

#[derive(Debug, Clone)]
pub struct AttemptLimiter {
    max_failures: u32,
    failures: u32,
    last_failure: Option<Instant>,
}

impl Default for AttemptLimiter {
    fn default() -> Self {
        Self::new(MAX_CODE_ATTEMPTS)
    }
}

impl AttemptLimiter {
    pub fn new(max_failures: u32) -> Self {
        Self {
            max_failures,
            failures: 0,
            last_failure: None,
        }
    }

    /// Un essai est-il possible à l'instant `now` ?
    pub fn check(&self, now: Instant) -> Verdict {
        if self.is_locked() {
            return Verdict::Locked;
        }
        if let Some(last) = self.last_failure {
            let index = (self.failures as usize)
                .saturating_sub(1)
                .min(DELAYS_SECS.len() - 1);
            let delay = Duration::from_secs(DELAYS_SECS[index]);
            let elapsed = now.saturating_duration_since(last);
            if elapsed < delay {
                return Verdict::WaitFor(delay - elapsed);
            }
        }
        Verdict::Allowed
    }

    /// Enregistre un échec et renvoie le nombre d'essais restants.
    pub fn record_failure(&mut self, now: Instant) -> u32 {
        self.failures += 1;
        self.last_failure = Some(now);
        self.remaining()
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn remaining(&self) -> u32 {
        self.max_failures.saturating_sub(self.failures)
    }

    pub fn is_locked(&self) -> bool {
        self.failures >= self.max_failures
    }

    /// Nouveau code généré par l'émetteur : on repart de zéro, le lien reste le même.
    pub fn reset_for_new_code(&mut self) {
        self.failures = 0;
        self.last_failure = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_limiter_allows() {
        let limiter = AttemptLimiter::default();
        assert_eq!(limiter.check(Instant::now()), Verdict::Allowed);
        assert_eq!(limiter.remaining(), 3);
    }

    #[test]
    fn delay_after_first_failure() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        assert_eq!(limiter.record_failure(t0), 2);
        assert_eq!(
            limiter.check(t0 + Duration::from_secs(1)),
            Verdict::WaitFor(Duration::from_secs(1))
        );
        assert_eq!(limiter.check(t0 + Duration::from_secs(2)), Verdict::Allowed);
    }

    #[test]
    fn longer_delay_after_second_failure() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        limiter.record_failure(t0);
        let t1 = t0 + Duration::from_secs(3);
        assert_eq!(limiter.record_failure(t1), 1);
        assert_eq!(
            limiter.check(t1 + Duration::from_secs(4)),
            Verdict::WaitFor(Duration::from_secs(6))
        );
        assert_eq!(
            limiter.check(t1 + Duration::from_secs(10)),
            Verdict::Allowed
        );
    }

    #[test]
    fn locks_after_three_failures() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        limiter.record_failure(t0);
        limiter.record_failure(t0);
        assert_eq!(limiter.record_failure(t0), 0);
        assert!(limiter.is_locked());
        assert_eq!(
            limiter.check(t0 + Duration::from_secs(3600)),
            Verdict::Locked
        );
    }

    #[test]
    fn new_code_unlocks_the_link() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        for _ in 0..3 {
            limiter.record_failure(t0);
        }
        limiter.reset_for_new_code();
        assert_eq!(limiter.failures(), 0);
        assert_eq!(limiter.check(t0), Verdict::Allowed);
    }
}
