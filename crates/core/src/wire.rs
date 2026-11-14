//! Encodage des messages sur le fil.
//!
//! Une trame = longueur du corps (u32, gros-boutiste) + corps encodé en postcard.
//! Ce module ne fait aucune entrée-sortie : il prépare et décode les octets.

use thiserror::Error;

use crate::protocol::Message;

/// Taille de l'en-tête d'une trame.
pub const HEADER_LEN: usize = 4;

/// Taille maximale d'un corps de trame (16 Mio), pour les gros manifestes.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireError {
    #[error("trame trop grande : {0} octets")]
    TooLarge(usize),
    #[error("encodage impossible : {0}")]
    Encode(String),
    #[error("message illisible : {0}")]
    Decode(String),
}

/// Encode un message en trame complète (en-tête compris).
pub fn encode(message: &Message) -> Result<Vec<u8>, WireError> {
    let body = postcard::to_allocvec(message).map_err(|e| WireError::Encode(e.to_string()))?;
    if body.len() > MAX_FRAME_LEN {
        return Err(WireError::TooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(HEADER_LEN + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Lit la longueur du corps annoncée par l'en-tête.
pub fn body_len(header: [u8; HEADER_LEN]) -> Result<usize, WireError> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(WireError::TooLarge(len));
    }
    Ok(len)
}

/// Décode le corps d'une trame (sans l'en-tête).
pub fn decode(body: &[u8]) -> Result<Message, WireError> {
    postcard::from_bytes(body).map_err(|e| WireError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ProtocolError;

    fn roundtrip(message: Message) {
        let frame = encode(&message).unwrap();
        let header: [u8; HEADER_LEN] = frame[..HEADER_LEN].try_into().unwrap();
        let len = body_len(header).unwrap();
        assert_eq!(len, frame.len() - HEADER_LEN);
        assert_eq!(decode(&frame[HEADER_LEN..]).unwrap(), message);
    }

    #[test]
    fn messages_roundtrip() {
        roundtrip(Message::Hello { version: 1 });
        roundtrip(Message::Pake {
            payload: vec![0x41; 33],
        });
        roundtrip(Message::Confirm { tag: [9u8; 32] });
        roundtrip(Message::Error(ProtocolError::WrongCode { remaining: 2 }));
        roundtrip(Message::Request {
            file: 3,
            offset: 8 * 1024 * 1024,
        });
        roundtrip(Message::Block {
            offset: 0,
            len: 4096,
            hash: [1u8; 32],
        });
        roundtrip(Message::FileDone { full_hash: None });
    }

    #[test]
    fn oversized_header_is_rejected() {
        let header = (MAX_FRAME_LEN as u32 + 1).to_be_bytes();
        assert!(matches!(body_len(header), Err(WireError::TooLarge(_))));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(matches!(
            decode(&[0xff, 0xff, 0xff]),
            Err(WireError::Decode(_))
        ));
    }
}
