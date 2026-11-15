//! Code d'accès : `NNNN-mot-C`, par exemple `4821-lanterne-7`.
//!
//! - `NNNN` : 4 chiffres tirés au hasard
//! - `mot` : un mot de la liste (ASCII, minuscules, faciles à dicter)
//! - `C` : un chiffre de contrôle (algorithme de Damm)
//!
//! Le chiffre de contrôle est calculé sur les 4 chiffres suivis de la position
//! du mot dans la liste (sur 4 chiffres). L'algorithme de Damm détecte toutes les
//! erreurs sur un seul chiffre et toutes les inversions de deux chiffres voisins.
//! Un mot inconnu est refusé directement. Les fautes de frappe sont donc
//! repérées en local, sans contacter l'émetteur et sans consommer d'essai.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use rand::rngs::OsRng;
use rand::Rng;
use thiserror::Error;

/// Liste de mots, un par ligne. Les lignes vides et celles qui commencent par `#`
/// sont ignorées. L'ordre compte : la position du mot entre dans le chiffre de
/// contrôle, émetteur et récepteur doivent donc avoir la même liste.
const RAW_WORDLIST: &str = include_str!("wordlist_fr.txt");

/// Taille visée pour la liste : 2048 mots, soit environ 20 millions de codes.
pub const RECOMMENDED_WORDS: usize = 2048;

/// Nombre maximal de mots : leur position doit tenir sur 4 chiffres.
const MAX_WORDS: usize = 10_000;

/// Nombre de valeurs possibles pour la partie numérique.
const DIGIT_SPACE: u16 = 10_000;

/// Table de l'algorithme de Damm (quasi-groupe d'ordre 10, totalement antisymétrique).
const DAMM_TABLE: [[u8; 10]; 10] = [
    [0, 3, 1, 7, 5, 9, 8, 6, 4, 2],
    [7, 0, 9, 2, 1, 5, 4, 8, 6, 3],
    [4, 2, 0, 6, 8, 7, 1, 3, 5, 9],
    [1, 7, 5, 0, 9, 8, 3, 4, 2, 6],
    [6, 1, 2, 3, 0, 4, 5, 9, 7, 8],
    [3, 6, 7, 4, 2, 0, 9, 5, 8, 1],
    [5, 8, 6, 9, 7, 2, 0, 1, 3, 4],
    [8, 9, 4, 5, 3, 6, 2, 0, 1, 7],
    [9, 4, 3, 8, 6, 1, 7, 2, 0, 5],
    [2, 5, 8, 1, 4, 3, 6, 7, 9, 0],
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodeError {
    #[error("la liste de mots est vide ou trop longue")]
    BadWordlist,
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
    word_index: u16,
}

impl AccessCode {
    /// Tire un nouveau code avec le générateur aléatoire du système.
    pub fn generate() -> Result<Self, CodeError> {
        let count = words().len();
        if count == 0 || count > MAX_WORDS {
            return Err(CodeError::BadWordlist);
        }
        let mut rng = OsRng;
        Ok(Self {
            digits: rng.gen_range(0..DIGIT_SPACE),
            word_index: rng.gen_range(0..count) as u16,
        })
    }

    pub fn word(&self) -> &'static str {
        words()[usize::from(self.word_index)]
    }

    /// Partie secrète du code (sans le chiffre de contrôle), à donner à SPAKE2.
    pub fn password(&self) -> Vec<u8> {
        format!("{:04}-{}", self.digits, self.word()).into_bytes()
    }

    pub fn check_digit(&self) -> u8 {
        check_digit(self.digits, self.word_index)
    }
}

impl fmt::Display for AccessCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{}-{}",
            self.digits,
            self.word(),
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
        if digits_text.len() != 4 || !digits_text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CodeError::BadDigits);
        }
        let digits: u16 = digits_text.parse().map_err(|_| CodeError::BadDigits)?;

        let word = parts[1];
        let word_index = words()
            .iter()
            .position(|w| *w == word)
            .ok_or_else(|| CodeError::UnknownWord(word.to_string()))?;
        let word_index = u16::try_from(word_index).map_err(|_| CodeError::BadWordlist)?;

        let check_text = parts[2];
        if check_text.len() != 1 || !check_text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CodeError::BadCheckDigit);
        }
        let check = check_text.as_bytes()[0] - b'0';

        if check != check_digit(digits, word_index) {
            return Err(CodeError::ChecksumMismatch);
        }

        Ok(Self { digits, word_index })
    }
}

/// Chiffre de contrôle de Damm sur `NNNN` suivi de la position du mot sur 4 chiffres.
fn check_digit(digits: u16, word_index: u16) -> u8 {
    damm(
        format!("{digits:04}{word_index:04}")
            .bytes()
            .map(|b| b - b'0'),
    )
}

fn damm(digits: impl Iterator<Item = u8>) -> u8 {
    digits.fold(0u8, |interim, d| {
        DAMM_TABLE[usize::from(interim)][usize::from(d)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_text(digits: u16, word_index: u16) -> String {
        format!(
            "{digits:04}-{}-{}",
            words()[usize::from(word_index)],
            check_digit(digits, word_index)
        )
    }

    #[test]
    fn damm_reference_value() {
        // Exemple de référence de l'algorithme : 572 → 4.
        assert_eq!(damm([5u8, 7, 2].into_iter()), 4);
        assert_eq!(damm([5u8, 7, 2, 4].into_iter()), 0);
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
    fn parse_is_case_and_space_tolerant() {
        let text = format!("  {}  ", code_text(1234, 0).to_uppercase());
        assert!(text.parse::<AccessCode>().is_ok());
    }

    #[test]
    fn every_single_digit_error_is_caught() {
        for digits in [0u16, 4821, 9999] {
            let good = format!("{digits:04}");
            let check = check_digit(digits, 0);
            for pos in 0..4 {
                for replacement in b'0'..=b'9' {
                    let mut bytes = good.clone().into_bytes();
                    if bytes[pos] == replacement {
                        continue;
                    }
                    bytes[pos] = replacement;
                    let typo = String::from_utf8(bytes).unwrap();
                    let text = format!("{typo}-{}-{check}", words()[0]);
                    assert_eq!(
                        text.parse::<AccessCode>().unwrap_err(),
                        CodeError::ChecksumMismatch,
                        "faute non détectée : {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn adjacent_transpositions_are_caught() {
        let check = check_digit(4821, 0);
        for typo in ["8421", "4281", "4812"] {
            let text = format!("{typo}-{}-{check}", words()[0]);
            assert_eq!(
                text.parse::<AccessCode>().unwrap_err(),
                CodeError::ChecksumMismatch,
                "inversion non détectée : {text}"
            );
        }
    }

    #[test]
    fn password_excludes_check_digit() {
        let code: AccessCode = code_text(4821, 0).parse().unwrap();
        assert_eq!(code.password(), format!("4821-{}", words()[0]).into_bytes());
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
        let text = format!("1234-{}-x", words()[0]);
        assert_eq!(
            text.parse::<AccessCode>().unwrap_err(),
            CodeError::BadCheckDigit
        );
    }

    #[test]
    fn wordlist_is_clean() {
        let list = words();
        assert!(!list.is_empty() && list.len() <= MAX_WORDS);
        let mut seen = std::collections::HashSet::new();
        for word in list {
            assert!(
                word.bytes().all(|b| b.is_ascii_lowercase()),
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
    fn wordlist_has_recommended_size() {
        assert_eq!(words().len(), RECOMMENDED_WORDS);
    }
}
