//! Code d'accès : `NNNN-mot-C`, par exemple `4821-lanterne-7`.
//!
//! - `NNNN` : 4 chiffres tirés au hasard
//! - `mot` : un mot de la liste (ASCII, minuscules, faciles à dicter)
//! - `C` : un chiffre de contrôle calculé sur `NNNN-mot`
//!
//! Le mot et le chiffre de contrôle se vérifient en local. Une faute de frappe
//! est donc détectée avant de contacter l'émetteur, sans consommer d'essai.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use rand::rngs::OsRng;
use rand::Rng;
use thiserror::Error;

/// Liste de mots, un par ligne. Les lignes vides et celles qui commencent par `#`
/// sont ignorées.
const RAW_WORDLIST: &str = include_str!("wordlist_fr.txt");

/// Taille visée pour la liste : 2048 mots, soit environ 20 millions de codes.
pub const RECOMMENDED_WORDS: usize = 2048;

/// Nombre de valeurs possibles pour la partie numérique.
const DIGIT_SPACE: u16 = 10_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodeError {
    #[error("la liste de mots est vide")]
    EmptyWordlist,
    #[error("le code doit avoir la forme 4821-mot-7")]
    BadShape,
    #[error("la première partie du code doit être composée de 4 chiffres")]
    BadDigits,
    #[error("mot inconnu : {0}")]
    UnknownWord(String),
    #[error("le chiffre de contrôle doit être un seul chiffre")]
    BadCheckDigit,
    #[error("le code ne correspond pas à son chiffre de contrôle, il y a une faute de frappe")]
    ChecksumMismatch,
}

static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();

/// Liste de mots chargée depuis `wordlist_fr.txt`.
pub fn words() -> &'static [&'static str] {
    WORDS
        .get_or_init(|| {
            RAW_WORDLIST
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect()
        })
        .as_slice()
}

/// Nombre de codes différents possibles avec la liste actuelle.
pub fn combinations() -> u64 {
    u64::from(DIGIT_SPACE) * words().len() as u64
}

/// Code d'accès valide (mot connu, chiffre de contrôle correct).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessCode {
    digits: u16,
    word: String,
}

impl AccessCode {
    /// Tire un nouveau code avec le générateur aléatoire du système.
    pub fn generate() -> Result<Self, CodeError> {
        let words = words();
        if words.is_empty() {
            return Err(CodeError::EmptyWordlist);
        }
        let mut rng = OsRng;
        let digits = rng.gen_range(0..DIGIT_SPACE);
        let word = words[rng.gen_range(0..words.len())];
        Ok(Self {
            digits,
            word: word.to_string(),
        })
    }

    /// Partie secrète du code (sans le chiffre de contrôle), à donner à SPAKE2.
    pub fn password(&self) -> Vec<u8> {
        body(self.digits, &self.word).into_bytes()
    }

    pub fn check_digit(&self) -> u8 {
        check_digit(self.digits, &self.word)
    }
}

impl fmt::Display for AccessCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}",
            body(self.digits, &self.word),
            self.check_digit()
        )
    }
}

impl FromStr for AccessCode {
    type Err = CodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        let parts: Vec<&str> = normalized.split('-').collect();
        if parts.len() != 3 {
            return Err(CodeError::BadShape);
        }

        let digits_text = parts[0];
        if digits_text.len() != 4 || !digits_text.chars().all(|c| c.is_ascii_digit()) {
            return Err(CodeError::BadDigits);
        }
        let digits: u16 = digits_text.parse().map_err(|_| CodeError::BadDigits)?;

        let word = parts[1];
        if !words().iter().any(|w| *w == word) {
            return Err(CodeError::UnknownWord(word.to_string()));
        }

        let check_text = parts[2];
        if check_text.len() != 1 || !check_text.chars().all(|c| c.is_ascii_digit()) {
            return Err(CodeError::BadCheckDigit);
        }
        let check: u8 = check_text.parse().map_err(|_| CodeError::BadCheckDigit)?;

        if check != check_digit(digits, word) {
            return Err(CodeError::ChecksumMismatch);
        }

        Ok(Self {
            digits,
            word: word.to_string(),
        })
    }
}

fn body(digits: u16, word: &str) -> String {
    format!("{digits:04}-{word}")
}

/// Somme pondérée par la position, modulo 10.
fn check_digit(digits: u16, word: &str) -> u8 {
    let sum: u32 = body(digits, word)
        .bytes()
        .enumerate()
        .map(|(i, b)| (i as u32 + 1) * u32::from(b))
        .sum();
    (sum % 10) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_word() -> &'static str {
        words()[0]
    }

    #[test]
    fn generate_then_parse_roundtrip() {
        for _ in 0..50 {
            let code = AccessCode::generate().unwrap();
            let parsed: AccessCode = code.to_string().parse().unwrap();
            assert_eq!(parsed, code);
        }
    }

    #[test]
    fn display_has_expected_shape() {
        let code = AccessCode::generate().unwrap();
        let text = code.to_string();
        let parts: Vec<&str> = text.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 4);
        assert_eq!(parts[2].len(), 1);
    }

    #[test]
    fn parse_is_case_and_space_tolerant() {
        let word = first_word();
        let check = check_digit(1234, word);
        let text = format!("  1234-{}-{}  ", word.to_uppercase(), check);
        assert!(text.parse::<AccessCode>().is_ok());
    }

    #[test]
    fn wrong_check_digit_is_rejected() {
        let word = first_word();
        let wrong = (check_digit(1234, word) + 1) % 10;
        let text = format!("1234-{word}-{wrong}");
        assert_eq!(
            text.parse::<AccessCode>().unwrap_err(),
            CodeError::ChecksumMismatch
        );
    }

    #[test]
    fn typo_in_digits_is_rejected() {
        let word = first_word();
        let check = check_digit(4821, word);
        // 4812 au lieu de 4821 : la somme pondérée change, sauf collision modulo 10.
        let typo = format!("4812-{word}-{check}");
        let result = typo.parse::<AccessCode>();
        if check_digit(4812, word) != check {
            assert_eq!(result.unwrap_err(), CodeError::ChecksumMismatch);
        }
    }

    #[test]
    fn unknown_word_is_rejected() {
        let err = "1234-zzzzzzzz-0".parse::<AccessCode>().unwrap_err();
        assert!(matches!(err, CodeError::UnknownWord(_)));
    }

    #[test]
    fn bad_shapes_are_rejected() {
        assert_eq!(
            "1234-mot".parse::<AccessCode>().unwrap_err(),
            CodeError::BadShape
        );
        assert_eq!(
            "12-mot-1".parse::<AccessCode>().unwrap_err(),
            CodeError::BadDigits
        );
    }

    #[test]
    fn wordlist_is_clean() {
        let list = words();
        assert!(!list.is_empty());
        let mut seen = std::collections::HashSet::new();
        for word in list {
            assert!(
                word.chars().all(|c| c.is_ascii_lowercase()),
                "mot non ASCII ou non minuscule : {word}"
            );
            assert!(
                (4..=8).contains(&word.len()),
                "longueur hors de 4 à 8 lettres : {word}"
            );
            assert!(seen.insert(*word), "mot en double : {word}");
        }
    }

    #[test]
    #[ignore = "liste de mots provisoire, voir wordlist_fr.txt"]
    fn wordlist_has_recommended_size() {
        assert_eq!(words().len(), RECOMMENDED_WORDS);
    }
}
