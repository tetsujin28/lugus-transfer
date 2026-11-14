//! Format du lien : `lugus://r/<clé>?v=1`.
//!
//! `<clé>` est la clé publique Ed25519 éphémère de l'émetteur (32 octets),
//! écrite en z-base-32 (52 caractères), comme le fait pkarr. C'est aussi le nom
//! sous lequel l'émetteur publie ses adresses dans la DHT.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

use crate::{LINK_SCHEME, PROTOCOL_VERSION};

/// Alphabet z-base-32.
const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

/// Taille d'une clé publique Ed25519.
pub const KEY_LEN: usize = 32;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkError {
    #[error("le lien doit commencer par {0}://r/")]
    BadPrefix(&'static str),
    #[error("caractère invalide dans la clé : {0:?}")]
    InvalidKeyChar(char),
    #[error("la clé doit faire 32 octets, elle en fait {0}")]
    BadKeyLength(usize),
    #[error("la clé n'est pas écrite sous sa forme canonique")]
    NonCanonicalKey,
    #[error("version de lien non prise en charge : {0}")]
    UnsupportedVersion(String),
    #[error("paramètre de lien inconnu : {0}")]
    UnknownParameter(String),
}

/// Lien de réception : contient uniquement la clé publique de l'émetteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LugusLink {
    key: [u8; KEY_LEN],
}

impl LugusLink {
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    /// Clé écrite en z-base-32 (52 caractères).
    pub fn encode_key(&self) -> String {
        zbase32_encode(&self.key)
    }
}

impl fmt::Display for LugusLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}://r/{}?v={}",
            LINK_SCHEME,
            self.encode_key(),
            PROTOCOL_VERSION
        )
    }
}

impl FromStr for LugusLink {
    type Err = LinkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let prefix = format!("{}://r/", LINK_SCHEME);
        let rest = s
            .trim()
            .strip_prefix(prefix.as_str())
            .ok_or(LinkError::BadPrefix(LINK_SCHEME))?;

        let (key_part, query) = match rest.split_once('?') {
            Some((key, query)) => (key, Some(query)),
            None => (rest, None),
        };

        if let Some(query) = query {
            for param in query.split('&').filter(|p| !p.is_empty()) {
                match param.split_once('=') {
                    Some(("v", value)) => {
                        if value != PROTOCOL_VERSION.to_string() {
                            return Err(LinkError::UnsupportedVersion(value.to_string()));
                        }
                    }
                    _ => return Err(LinkError::UnknownParameter(param.to_string())),
                }
            }
        }

        let bytes = zbase32_decode(key_part)?;
        let key: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|b: Vec<u8>| LinkError::BadKeyLength(b.len()))?;
        Ok(Self { key })
    }
}

fn zbase32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8 + 4) / 5);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(ALPHABET[index] as char);
        }
        buffer &= (1u32 << bits) - 1;
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

fn zbase32_decode(text: &str) -> Result<Vec<u8>, LinkError> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for c in text.chars() {
        let lower = c.to_ascii_lowercase();
        let index = ALPHABET
            .iter()
            .position(|&a| a as char == lower)
            .ok_or(LinkError::InvalidKeyChar(c))?;
        buffer = (buffer << 5) | index as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
            buffer &= (1u32 << bits) - 1;
        }
    }
    // Les bits de bourrage restants doivent être à zéro.
    if buffer != 0 {
        return Err(LinkError::NonCanonicalKey);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_key() -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        key
    }

    #[test]
    fn key_is_52_chars() {
        let link = LugusLink::new(sample_key());
        assert_eq!(link.encode_key().len(), 52);
    }

    #[test]
    fn link_roundtrip() {
        let link = LugusLink::new(sample_key());
        let text = link.to_string();
        assert!(text.starts_with("lugus://r/"));
        assert!(text.ends_with("?v=1"));
        let parsed: LugusLink = text.parse().unwrap();
        assert_eq!(parsed, link);
    }

    #[test]
    fn link_without_query_is_accepted() {
        let link = LugusLink::new(sample_key());
        let text = format!("lugus://r/{}", link.encode_key());
        assert_eq!(text.parse::<LugusLink>().unwrap(), link);
    }

    #[test]
    fn rejects_wrong_scheme() {
        let err = "http://r/abc".parse::<LugusLink>().unwrap_err();
        assert_eq!(err, LinkError::BadPrefix("lugus"));
    }

    #[test]
    fn rejects_wrong_length() {
        let err = "lugus://r/ybndrfg8".parse::<LugusLink>().unwrap_err();
        assert!(matches!(err, LinkError::BadKeyLength(_)));
    }

    #[test]
    fn rejects_invalid_char() {
        let err = "lugus://r/l".parse::<LugusLink>().unwrap_err();
        assert_eq!(err, LinkError::InvalidKeyChar('l'));
    }

    #[test]
    fn rejects_future_version() {
        let link = LugusLink::new(sample_key());
        let text = format!("lugus://r/{}?v=2", link.encode_key());
        assert!(matches!(
            text.parse::<LugusLink>().unwrap_err(),
            LinkError::UnsupportedVersion(_)
        ));
    }
}
