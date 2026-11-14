//! Binaire de test de Lugus Transfer : `lugus send`, `lugus receive`, `lugus code`.
//!
//! Étape 2 : l'émetteur publie ses adresses dans la DHT, le récepteur le
//! retrouve grâce au lien, puis les deux s'authentifient avec le code (SPAKE2,
//! 3 essais). Une fois le code accepté, l'émetteur envoie le manifeste et le
//! récepteur renvoie sa sélection. Aucun fichier n'est encore transféré (étape 3).

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use lugus_auth::{Handshake, Side, EXPORTER_LABEL};
use lugus_core::code::{self, AccessCode};
use lugus_core::limiter::{AttemptLimiter, Verdict};
use lugus_core::link::LugusLink;
use lugus_core::protocol::{FileKind, Manifest, Message, ProtocolError, Selection};
use lugus_core::{wire, DEFAULT_LINK_TTL_SECS, MAX_CODE_ATTEMPTS, PROTOCOL_VERSION};
use lugus_noctiluca::{
    channel_binding, current_path, net_err, recv_msg, remote_key, send_msg, Beacon, Role,
    HANDSHAKE_MAX_LEN,
};

/// Délai maximal pour trouver l'émetteur dans la DHT et s'y connecter.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);

/// Délai maximal pour joindre un relais au démarrage.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(20);

/// Délai maximal pour une poignée de main complète, côté émetteur.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Délai laissé au pair pour lire nos derniers messages avant de fermer.
const LINGER: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(
    name = "lugus",
    version,
    about = "Lugus Transfer : transfert pair à pair de très gros fichiers"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Envoie un fichier ou un dossier : affiche un lien et un code à partager.
    Send {
        /// Fichier ou dossier à envoyer
        path: PathBuf,
    },
    /// Reçoit à partir d'un lien et d'un code.
    Receive {
        /// Lien reçu, de la forme lugus://r/<clé>
        link: String,
        /// Code reçu, de la forme 4821-mot-7 (demandé si absent)
        #[arg(long)]
        code: Option<String>,
        /// Dossier de destination
        #[arg(long, default_value = ".")]
        dest: PathBuf,
    },
    /// Génère un code d'exemple, ou vérifie un code donné (sans réseau).
    Code {
        /// Code à vérifier, par exemple 4821-lanterne-7
        check: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Send { path } => cmd_send(path).await,
        Command::Receive { link, code, dest } => cmd_receive(link, code, dest).await,
        Command::Code { check } => cmd_code(check),
    }
}

// ---------------------------------------------------------------------------
// Émetteur
// ---------------------------------------------------------------------------

async fn cmd_send(path: PathBuf) -> Result<()> {
    let manifest = lugus_store::scan(&path)
        .with_context(|| format!("lecture de {} impossible", path.display()))?;
    if manifest.files.is_empty() {
        bail!("aucun fichier à envoyer dans {}", path.display());
    }
    warn_if_wordlist_is_small();

    let mut access = AccessCode::generate().context("génération du code")?;
    let mut limiter = AttemptLimiter::default();

    eprintln!("Ouverture de l'endpoint…");
    let beacon = Beacon::bind(Role::Sender).await?;
    if tokio::time::timeout(ONLINE_TIMEOUT, beacon.online())
        .await
        .is_err()
    {
        eprintln!(
            "Attention : aucun relais joignable. Seules les connexions directes \
             seront possibles."
        );
    }

    println!();
    println!("Lien : {}", beacon.link());
    println!("Code : {access}");
    println!(
        "Validité : {} h, {MAX_CODE_ATTEMPTS} essais",
        DEFAULT_LINK_TTL_SECS / 3600
    );
    println!();
    print_manifest_summary(&manifest);
    println!();
    eprintln!(
        "Publication dans la DHT en cours (quelques secondes). \
         En attente du destinataire, Ctrl+C pour arrêter."
    );

    loop {
        let conn = beacon
            .accept()
            .await?
            .context("l'endpoint s'est fermé avant toute connexion")?;
        eprintln!("Connexion entrante, chemin : {}", current_path(&conn));

        let handshake = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            sender_handshake(&conn, &access, &mut limiter, beacon.key()),
        )
        .await;

        match handshake {
            Ok(Ok(Some((mut send, mut recv)))) => {
                eprintln!("Code accepté.");
                serve_manifest(&mut send, &mut recv, &manifest).await?;
                let _ = tokio::time::timeout(LINGER, conn.closed()).await;
                eprintln!("Chemin final : {}", current_path(&conn));
                break;
            }
            Ok(Ok(None)) => {
                let _ = tokio::time::timeout(LINGER, conn.closed()).await;
            }
            Ok(Err(err)) => eprintln!("Connexion abandonnée : {err:#}"),
            Err(_) => {
                eprintln!("Poignée de main trop longue, connexion fermée.");
                conn.close(1u32.into(), b"delai depasse");
            }
        }

        if limiter.is_locked() {
            eprintln!(
                "{MAX_CODE_ATTEMPTS} essais ratés : le code est bloqué, le lien reste valable."
            );
            if !ask_yes_no("Générer un nouveau code ?").await? {
                bail!("envoi annulé");
            }
            access = AccessCode::generate().context("génération du code")?;
            limiter.reset_for_new_code();
            println!("Nouveau code : {access}");
        } else if limiter.failures() > 0 {
            eprintln!("Essais restants pour ce code : {}", limiter.remaining());
        }
    }

    beacon.close().await;
    Ok(())
}

/// Poignée de main côté émetteur. Renvoie les flux si le code est accepté,
/// `None` si la demande a été refusée (le pair a reçu un message d'erreur).
async fn sender_handshake(
    conn: &lugus_noctiluca::Connection,
    access: &AccessCode,
    limiter: &mut AttemptLimiter,
    own_key: [u8; 32],
) -> Result<Option<(lugus_noctiluca::SendStream, lugus_noctiluca::RecvStream)>> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(net_err)?;

    match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Hello { version } if version == PROTOCOL_VERSION => {}
        Message::Hello { .. } => {
            let error = ProtocolError::UnsupportedVersion {
                supported: PROTOCOL_VERSION,
            };
            reject(&mut send, error).await?;
            return Ok(None);
        }
        _ => {
            reject(&mut send, ProtocolError::BadMessage).await?;
            return Ok(None);
        }
    }

    let Message::Pake { payload: peer_pake } = recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? else {
        reject(&mut send, ProtocolError::BadMessage).await?;
        return Ok(None);
    };

    // L'essai est réservé ici : il compte même si le pair abandonne ensuite.
    match limiter.begin(Instant::now()) {
        Verdict::Allowed => {}
        Verdict::WaitFor(wait) => {
            let error = ProtocolError::TooSoon {
                wait_secs: wait.as_secs().max(1),
            };
            reject(&mut send, error).await?;
            return Ok(None);
        }
        Verdict::Locked => {
            reject(&mut send, ProtocolError::CodeLocked).await?;
            return Ok(None);
        }
    }

    let context = lugus_auth::Context {
        sender_id: own_key,
        receiver_id: remote_key(conn),
        channel_binding: channel_binding(conn, EXPORTER_LABEL)?,
    };
    let (handshake, own_pake) = Handshake::start(Side::Sender, &access.password(), context);
    let session = match handshake.finish(&peer_pake) {
        Ok(session) => session,
        Err(_) => {
            reject(&mut send, ProtocolError::BadMessage).await?;
            return Ok(None);
        }
    };
    send_msg(&mut send, &Message::Pake { payload: own_pake }).await?;

    let Message::Confirm { tag } = recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? else {
        reject(&mut send, ProtocolError::BadMessage).await?;
        return Ok(None);
    };
    if !session.verify_peer(&tag) {
        eprintln!("Code incorrect reçu.");
        let error = ProtocolError::WrongCode {
            remaining: limiter.remaining(),
        };
        reject(&mut send, error).await?;
        return Ok(None);
    }

    limiter.succeed();
    send_msg(
        &mut send,
        &Message::Confirm {
            tag: session.own_tag(),
        },
    )
    .await?;
    Ok(Some((send, recv)))
}

/// Envoie le manifeste et attend la sélection du destinataire.
async fn serve_manifest(
    send: &mut lugus_noctiluca::SendStream,
    recv: &mut lugus_noctiluca::RecvStream,
    manifest: &Manifest,
) -> Result<()> {
    send_msg(send, &Message::Manifest(manifest.clone())).await?;
    match recv_msg(recv, wire::MAX_FRAME_LEN).await? {
        Message::Select(selection) => {
            let valid = selection
                .files
                .iter()
                .all(|&i| (i as usize) < manifest.files.len());
            if !valid {
                bail!("sélection invalide reçue");
            }
            let bytes: u64 = selection
                .files
                .iter()
                .map(|&i| manifest.files[i as usize].size)
                .sum();
            eprintln!(
                "Le destinataire a choisi {} fichier(s), {} (proxys d'abord : {}).",
                selection.files.len(),
                human_bytes(bytes),
                if selection.proxies_first {
                    "oui"
                } else {
                    "non"
                }
            );
            eprintln!("Le transfert des fichiers arrive à l'étape 3.");
        }
        other => bail!("réponse inattendue : {other:?}"),
    }
    send.finish().map_err(net_err)?;
    Ok(())
}

async fn reject(send: &mut lugus_noctiluca::SendStream, error: ProtocolError) -> Result<()> {
    send_msg(send, &Message::Error(error)).await?;
    send.finish().map_err(net_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Récepteur
// ---------------------------------------------------------------------------

async fn cmd_receive(link: String, code: Option<String>, dest: PathBuf) -> Result<()> {
    let link: LugusLink = link.parse().context("lien invalide")?;

    let raw = match code {
        Some(code) => code,
        None => {
            eprint!("Code : ");
            io::stderr().flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            line
        }
    };
    let access: AccessCode = raw
        .parse()
        .context("code invalide (vérifié en local, aucun essai consommé)")?;

    eprintln!("Destination : {}", dest.display());
    eprintln!("Recherche de l'émetteur dans la DHT…");

    let beacon = Beacon::bind(Role::Receiver).await?;
    let started = Instant::now();
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, beacon.connect(&link))
        .await
        .context("émetteur introuvable dans le délai imparti")??;
    if remote_key(&conn) != *link.key() {
        bail!("la machine jointe n'est pas celle du lien");
    }
    eprintln!(
        "Émetteur trouvé en {:.1} s, chemin : {}",
        started.elapsed().as_secs_f32(),
        current_path(&conn)
    );

    let (mut send, mut recv) = conn.open_bi().await.map_err(net_err)?;
    send_msg(
        &mut send,
        &Message::Hello {
            version: PROTOCOL_VERSION,
        },
    )
    .await?;

    let context = lugus_auth::Context {
        sender_id: *link.key(),
        receiver_id: beacon.key(),
        channel_binding: channel_binding(&conn, EXPORTER_LABEL)?,
    };
    let (handshake, own_pake) = Handshake::start(Side::Receiver, &access.password(), context);
    send_msg(&mut send, &Message::Pake { payload: own_pake }).await?;

    let session = match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Pake { payload } => handshake.finish(&payload)?,
        Message::Error(error) => bail!(describe(&error)),
        other => bail!("réponse inattendue : {other:?}"),
    };
    send_msg(
        &mut send,
        &Message::Confirm {
            tag: session.own_tag(),
        },
    )
    .await?;

    match recv_msg(&mut recv, HANDSHAKE_MAX_LEN).await? {
        Message::Confirm { tag } if session.verify_peer(&tag) => {}
        Message::Confirm { .. } => bail!("l'émetteur n'a pas prouvé qu'il connaît le code"),
        Message::Error(error) => bail!(describe(&error)),
        other => bail!("réponse inattendue : {other:?}"),
    }
    eprintln!("Code accepté.");

    let manifest = match recv_msg(&mut recv, wire::MAX_FRAME_LEN).await? {
        Message::Manifest(manifest) => manifest,
        other => bail!("manifeste attendu, reçu : {other:?}"),
    };
    if let Some(bad) = manifest
        .files
        .iter()
        .find(|f| !lugus_store::is_safe_relative_path(&f.path))
    {
        bail!("chemin dangereux dans le manifeste : {}", bad.path);
    }
    println!();
    print_manifest_summary(&manifest);

    // Pour l'instant, on choisit tout, proxys d'abord.
    let selection = Selection {
        files: (0..manifest.files.len() as u32).collect(),
        proxies_first: true,
    };
    send_msg(&mut send, &Message::Select(selection)).await?;
    send.finish().map_err(net_err)?;

    // Attend que l'émetteur ait lu la sélection et fermé son côté.
    let _ = tokio::time::timeout(LINGER, recv.read_to_end(0)).await;
    eprintln!("Chemin final : {}", current_path(&conn));
    eprintln!("Le transfert des fichiers arrive à l'étape 3.");

    conn.close(0u32.into(), b"fin");
    beacon.close().await;
    Ok(())
}

fn describe(error: &ProtocolError) -> String {
    match error {
        ProtocolError::WrongCode { remaining: 0 } => {
            "code incorrect, plus aucun essai : demandez un nouveau code à l'expéditeur".to_string()
        }
        ProtocolError::WrongCode { remaining } => {
            format!("code incorrect, {remaining} essai(s) restant(s)")
        }
        ProtocolError::TooSoon { wait_secs } => {
            format!("trop tôt après un essai raté, réessayez dans {wait_secs} s")
        }
        ProtocolError::CodeLocked => {
            "ce code est bloqué : demandez un nouveau code à l'expéditeur".to_string()
        }
        ProtocolError::UnsupportedVersion { supported } => {
            format!("version incompatible, l'expéditeur utilise lugus/{supported}")
        }
        ProtocolError::LinkExpired => "ce lien a expiré".to_string(),
        ProtocolError::Busy => "l'expéditeur est occupé, réessayez plus tard".to_string(),
        ProtocolError::BadMessage => "l'expéditeur a refusé un message mal formé".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Divers
// ---------------------------------------------------------------------------

fn cmd_code(check: Option<String>) -> Result<()> {
    match check {
        Some(text) => {
            let access: AccessCode = text.parse().context("code invalide")?;
            println!("Code valide : {access}");
        }
        None => {
            warn_if_wordlist_is_small();
            let access = AccessCode::generate().context("génération du code")?;
            println!("{access}");
            println!(
                "{} codes possibles avec la liste actuelle",
                code::combinations()
            );
        }
    }
    Ok(())
}

fn warn_if_wordlist_is_small() {
    let count = code::words().len();
    if count < code::RECOMMENDED_WORDS {
        eprintln!(
            "Attention : liste de mots provisoire ({count} mots sur {} prévus), \
             le code est moins robuste que prévu.",
            code::RECOMMENDED_WORDS
        );
    }
}

fn print_manifest_summary(manifest: &Manifest) {
    let count = |kind: FileKind| manifest.files.iter().filter(|f| f.kind == kind).count();
    println!(
        "Contenu « {} » : {} fichier(s), {}",
        manifest.name,
        manifest.files.len(),
        human_bytes(manifest.total_bytes())
    );
    println!(
        "  {} original(aux), {} proxy(s), {} fichier(s) annexe(s)",
        count(FileKind::Original),
        count(FileKind::Proxy),
        count(FileKind::Sidecar)
    );
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} o")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

async fn ask_yes_no(question: &str) -> Result<bool> {
    let question = question.to_string();
    let answer = tokio::task::spawn_blocking(move || -> io::Result<bool> {
        eprint!("{question} [o/N] ");
        io::stderr().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let line = line.trim().to_lowercase();
        Ok(line == "o" || line == "oui")
    })
    .await
    .map_err(|e| anyhow!("{e}"))??;
    Ok(answer)
}
