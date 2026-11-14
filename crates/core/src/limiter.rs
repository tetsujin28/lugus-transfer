//! Règle des essais de code : 3 essais par lien, tous destinataires confondus,
//! avec un délai croissant entre deux essais.
//!
//! Un essai est **réservé dès son début** (`begin`) et compté comme un échec.
//! Il n'est rendu qu'en cas de succès (`succeed`). Ainsi, un attaquant qui
//! abandonne la connexion au milieu de l'échange perd quand même son essai, et
//! plusieurs connexions simultanées ne peuvent pas dépasser la limite.
//!
//! Le compteur vit côté émetteur. Le temps est passé en paramètre pour que la
//! logique reste testable sans horloge réelle.

use std::time::{Duration, Instant};

use crate::MAX_CODE_ATTEMPTS;

/// Délai imposé après le 1er essai raté, puis après le 2e. Au-delà, on garde le dernier.
const DELAYS_SECS: [u64; 2] = [2, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// L'essai est accordé (et réservé, si obtenu par `begin`).
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
    last_attempt: Option<Instant>,
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
            last_attempt: None,
        }
    }

    /// Un essai serait-il accordé à l'instant `now` ? Ne réserve rien.
    pub fn check(&self, now: Instant) -> Verdict {
        if self.is_locked() {
            return Verdict::Locked;
        }
        if let (Some(last), true) = (self.last_attempt, self.failures > 0) {
            let index = (self.failures as usize - 1).min(DELAYS_SECS.len() - 1);
            let delay = Duration::from_secs(DELAYS_SECS[index]);
            let elapsed = now.saturating_duration_since(last);
            if elapsed < delay {
                return Verdict::WaitFor(delay - elapsed);
            }
        }
        Verdict::Allowed
    }

    /// Demande un essai. S'il est accordé, il est aussitôt compté comme un échec.
    pub fn begin(&mut self, now: Instant) -> Verdict {
        let verdict = self.check(now);
        if verdict == Verdict::Allowed {
            self.failures += 1;
            self.last_attempt = Some(now);
        }
        verdict
    }

    /// L'essai en cours a réussi : il est rendu.
    pub fn succeed(&mut self) {
        self.failures = self.failures.saturating_sub(1);
        if self.failures == 0 {
            self.last_attempt = None;
        }
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Essais encore disponibles.
    pub fn remaining(&self) -> u32 {
        self.max_failures.saturating_sub(self.failures)
    }

    pub fn is_locked(&self) -> bool {
        self.failures >= self.max_failures
    }

    /// Nouveau code généré par l'émetteur : on repart de zéro, le lien reste le même.
    pub fn reset_for_new_code(&mut self) {
        self.failures = 0;
        self.last_attempt = None;
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
    fn begin_reserves_an_attempt() {
        let mut limiter = AttemptLimiter::default();
        assert_eq!(limiter.begin(Instant::now()), Verdict::Allowed);
        assert_eq!(limiter.remaining(), 2);
    }

    #[test]
    fn success_gives_the_attempt_back() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        limiter.begin(t0);
        limiter.succeed();
        assert_eq!(limiter.remaining(), 3);
        assert_eq!(limiter.check(t0), Verdict::Allowed);
    }

    #[test]
    fn delay_after_first_failure() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        limiter.begin(t0);
        assert_eq!(
            limiter.begin(t0 + Duration::from_secs(1)),
            Verdict::WaitFor(Duration::from_secs(1))
        );
        // Un refus pour délai ne consomme rien.
        assert_eq!(limiter.remaining(), 2);
        assert_eq!(limiter.begin(t0 + Duration::from_secs(2)), Verdict::Allowed);
    }

    #[test]
    fn longer_delay_after_second_failure() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        limiter.begin(t0);
        let t1 = t0 + Duration::from_secs(3);
        limiter.begin(t1);
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
    fn concurrent_attempts_cannot_exceed_the_limit() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        let mut granted = 0;
        for i in 0..10 {
            if limiter.begin(t0 + Duration::from_secs(60 * i)) == Verdict::Allowed {
                granted += 1;
            }
        }
        assert_eq!(granted, 3);
        assert!(limiter.is_locked());
    }

    #[test]
    fn locks_after_three_failures() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        for i in 0..3 {
            assert_eq!(
                limiter.begin(t0 + Duration::from_secs(60 * i)),
                Verdict::Allowed
            );
        }
        assert_eq!(
            limiter.check(t0 + Duration::from_secs(3600)),
            Verdict::Locked
        );
    }

    #[test]
    fn new_code_unlocks_the_link() {
        let mut limiter = AttemptLimiter::default();
        let t0 = Instant::now();
        for i in 0..3 {
            limiter.begin(t0 + Duration::from_secs(60 * i));
        }
        limiter.reset_for_new_code();
        assert_eq!(limiter.failures(), 0);
        assert_eq!(limiter.check(t0), Verdict::Allowed);
    }
}
