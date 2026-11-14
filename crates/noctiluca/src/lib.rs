//! Noctiluca : la lumière qui montre où aller.
//!
//! Ce crate s'occupe de la connectivité :
//!
//! - un endpoint iroh avec une clé Ed25519 éphémère, propre à chaque transfert ;
//! - la publication des adresses de l'émetteur dans la DHT Mainline (pkarr),
//!   signées par cette clé, et leur résolution côté récepteur ;
//! - la connexion QUIC directe quand c'est possible : réseau local, IPv6, port
//!   ouvert automatiquement par iroh (UPnP, PCP, NAT-PMP) ou hole punching ;
//! - le repli sur les relais publics d'iroh, chiffrés de bout en bout.
//!
//! Le hole punching est coordonné par iroh à travers ses relais. La « boîte aux
//! lettres » dans la DHT, prévue à l'origine, n'est donc pas nécessaire pour
//! l'instant. Elle reste une option si l'on veut se passer des relais.
//!
//! Aucun serveur iroh.link n'est utilisé pour la découverte : seulement la DHT.

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use iroh::address_lookup::{AddrFilter, AddressLookup};
use iroh::endpoint::presets;
pub use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, PublicKey, RelayMode, SecretKey, TransportAddr};
use iroh_mainline_address_lookup::DhtAddressLookup;
use lugus_core::link::LugusLink;
use lugus_core::protocol::Message;
use lugus_core::{wire, ALPN};
use n0_future::StreamExt;

/// Convertit une erreur iroh ou QUIC en `anyhow::Error`, en gardant son message.
///
/// Les erreurs d'iroh utilisent `n0-error`. Passer par leur affichage évite de
/// dépendre de la façon dont elles implémentent `std::error::Error`.
pub fn net_err(err: impl fmt::Display) -> anyhow::Error {
    anyhow!("{err:#}")
}

/// Rôle de l'endpoint : l'émetteur publie ses adresses, le récepteur ne fait que
/// chercher celles de l'émetteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

/// Chemin réellement emprunté par une connexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// Adresse privée : les deux machines sont sur le même réseau.
    Local,
    /// Adresse publique : connexion directe à travers Internet.
    Direct,
    /// Passage par un relais (chiffré de bout en bout).
    Relay,
    /// Aucun chemin sélectionné pour l'instant.
    Unknown,
}

impl PathKind {
    /// Identifiant stable, pour l'interface : `local`, `direct`, `relay`, `unknown`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Direct => "direct",
            Self::Relay => "relay",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Local => "réseau local",
            Self::Direct => "direct",
            Self::Relay => "relais",
            Self::Unknown => "inconnu",
        };
        f.write_str(label)
    }
}

/// Endpoint Lugus : une clé éphémère, un socket QUIC, la découverte par la DHT.
pub struct Beacon {
    endpoint: Endpoint,
}

impl Beacon {
    /// Crée un endpoint avec une clé neuve. Doit être appelé dans un runtime Tokio.
    pub async fn bind(role: Role) -> Result<Self> {
        let secret_key = SecretKey::generate();

        // Clé éphémère et lien privé : on peut publier les adresses IP, pas
        // seulement l'URL du relais, pour permettre les connexions directes.
        let dht = match role {
            Role::Sender => DhtAddressLookup::builder().addr_filter(AddrFilter::unfiltered()),
            Role::Receiver => DhtAddressLookup::builder().no_publish(),
        };

        // Preset `Minimal` : uniquement le fournisseur cryptographique, sans les
        // serveurs DNS d'iroh.link. La découverte passe seulement par la DHT.
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Default)
            .address_lookup(dht)
            .bind()
            .await
            .map_err(net_err)?;

        Ok(Self { endpoint })
    }

    /// Attend que l'endpoint soit joignable (connexion au relais établie).
    ///
    /// La publication dans la DHT démarre ensuite en arrière-plan et peut
    /// prendre quelques secondes : iroh ne permet pas d'attendre sa fin.
    pub async fn online(&self) {
        self.endpoint.online().await;
    }

    /// Lien de réception, construit à partir de la clé publique éphémère.
    pub fn link(&self) -> LugusLink {
        LugusLink::new(*self.endpoint.id().as_bytes())
    }

    /// Adresses IP locales connues de l'endpoint (pour l'affichage).
    pub fn ip_addrs(&self) -> Vec<String> {
        self.endpoint
            .addr()
            .ip_addrs()
            .map(|addr| addr.to_string())
            .collect()
    }

    /// Attend la prochaine connexion entrante avec l'ALPN `lugus/1`.
    ///
    /// Renvoie `Ok(None)` quand l'endpoint est fermé. Les connexions qui échouent
    /// pendant la négociation sont ignorées.
    pub async fn accept(&self) -> Result<Option<Connection>> {
        while let Some(incoming) = self.endpoint.accept().await {
            let mut accepting = match incoming.accept() {
                Ok(accepting) => accepting,
                Err(_) => continue,
            };
            let alpn = match accepting.alpn().await {
                Ok(alpn) => alpn,
                Err(_) => continue,
            };
            if alpn != ALPN {
                continue;
            }
            match accepting.await {
                Ok(conn) => return Ok(Some(conn)),
                Err(_) => continue,
            }
        }
        Ok(None)
    }

    /// Se connecte à l'émetteur désigné par le lien.
    ///
    /// iroh cherche les adresses dans la DHT, tente tous les chemins et garde le
    /// meilleur. L'identité de l'émetteur est vérifiée par TLS : la clé du lien
    /// est sa clé publique.
    pub async fn connect(&self, link: &LugusLink) -> Result<Connection> {
        let remote = PublicKey::from_bytes(link.key()).map_err(net_err)?;
        self.endpoint.connect(remote, ALPN).await.map_err(net_err)
    }

    /// Ferme proprement l'endpoint et ses connexions.
    pub async fn close(self) {
        self.endpoint.close().await;
    }
}

impl Beacon {
    /// Clé publique de cet endpoint.
    pub fn key(&self) -> [u8; 32] {
        *self.endpoint.id().as_bytes()
    }
}

/// Clé publique du pair, authentifiée par TLS.
pub fn remote_key(conn: &Connection) -> [u8; 32] {
    *conn.remote_id().as_bytes()
}

/// Secret de 32 octets propre à cette session TLS (RFC 5705), identique des
/// deux côtés de la connexion. Sert à lier l'authentification à la connexion.
pub fn channel_binding(conn: &Connection, label: &[u8]) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, label, b"")
        .map_err(|e| anyhow!("export de clé TLS impossible : {e:?}"))?;
    Ok(out)
}

/// Envoie un message sous forme de trame (longueur + postcard).
pub async fn send_msg(stream: &mut SendStream, message: &Message) -> Result<()> {
    let frame = wire::encode(message)?;
    stream.write_all(&frame).await.map_err(net_err)
}

/// Taille maximale d'un message pendant la poignée de main, avant authentification.
pub const HANDSHAKE_MAX_LEN: usize = 4096;

/// Lit le prochain message tramé, en refusant les corps de plus de `max_len` octets.
pub async fn recv_msg(stream: &mut RecvStream, max_len: usize) -> Result<Message> {
    let mut header = [0u8; wire::HEADER_LEN];
    stream.read_exact(&mut header).await.map_err(net_err)?;
    let len = wire::body_len(header)?;
    if len > max_len {
        return Err(anyhow!(
            "message trop long : {len} octets (maximum {max_len})"
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(net_err)?;
    Ok(wire::decode(&body)?)
}

/// Attend que le lien de `key` soit réellement trouvable dans la DHT, en le
/// cherchant avec un client distinct, comme le fera le destinataire.
/// Renvoie `false` si rien n'est trouvé avant `timeout`.
pub async fn wait_until_published(key: [u8; 32], timeout: Duration) -> Result<bool> {
    let id = PublicKey::from_bytes(&key).map_err(net_err)?;
    let lookup = DhtAddressLookup::builder()
        .no_publish()
        .build()
        .map_err(net_err)?;
    let search = async {
        loop {
            if let Some(mut found) = lookup.resolve(id) {
                while let Some(item) = found.next().await {
                    if item.is_ok() {
                        return true;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    };
    Ok(tokio::time::timeout(timeout, search).await.unwrap_or(false))
}

/// Chemin actuellement sélectionné par la connexion.
pub fn current_path(conn: &Connection) -> PathKind {
    conn.paths()
        .iter()
        .find(|path| path.is_selected())
        .map(|path| classify(path.remote_addr()))
        .unwrap_or(PathKind::Unknown)
}

fn classify(addr: &TransportAddr) -> PathKind {
    match addr {
        TransportAddr::Relay(_) => PathKind::Relay,
        TransportAddr::Ip(socket) if is_private(socket.ip()) => PathKind::Local,
        TransportAddr::Ip(_) => PathKind::Direct,
        _ => PathKind::Unknown,
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                // fc00::/7 : adresses locales uniques
                || (first & 0xfe00) == 0xfc00
                // fe80::/10 : adresses de lien local
                || (first & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_are_local() {
        assert!(is_private("192.168.1.20".parse().unwrap()));
        assert!(is_private("10.0.0.5".parse().unwrap()));
        assert!(is_private("fe80::1".parse().unwrap()));
        assert!(is_private("fd12:3456::1".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
        assert!(!is_private("2a01:e0a::1".parse().unwrap()));
    }
}
