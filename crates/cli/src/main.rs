//! Binaire de test de Lugus Transfer : `lugus send`, `lugus receive`, `lugus code`.
//!
//! Étape 1 : l'émetteur publie ses adresses dans la DHT, le récepteur le
//! retrouve grâce au lien, et les deux échangent un message de bienvenue.
//! Le code n'est pas encore vérifié par l'émetteur (étape 2, crate `auth`) et
//! aucun fichier n'est transféré (étape 3).

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lugus_core::code::{self, AccessCode};
use lugus_core::link::LugusLink;
use lugus_core::{DEFAULT_LINK_TTL_SECS, MAX_CODE_ATTEMPTS, PROTOCOL_VERSION};
use lugus_noctiluca::{current_path, net_err, Beacon, Role};

/// Délai maximal pour trouver l'émetteur dans la DHT et s'y connecter.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);

/// Taille maximale du message de bienvenue.
const MAX_HELLO_BYTES: usize = 1024;

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

async fn cmd_send(path: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("{} n'existe pas", path.display());
    }
    warn_if_wordlist_is_small();

    let access = AccessCode::generate().context("génération du code")?;

    eprintln!("Ouverture de l'endpoint…");
    let beacon = Beacon::bind(Role::Sender).await?;
    beacon.online().await;

    println!();
    println!("Lien : {}", beacon.link());
    println!("Code : {access}");
    println!(
        "Validité : {} h, {MAX_CODE_ATTEMPTS} essais",
        DEFAULT_LINK_TTL_SECS / 3600
    );
    println!();
    println!("Adresses locales :");
    for addr in beacon.ip_addrs() {
        println!("  {addr}");
    }
    println!();
    eprintln!(
        "Publication dans la DHT en cours (quelques secondes). \
         En attente du destinataire, Ctrl+C pour arrêter."
    );

    let conn = beacon
        .accept()
        .await?
        .context("l'endpoint s'est fermé avant toute connexion")?;
    eprintln!("Destinataire connecté, chemin : {}", current_path(&conn));

    let (mut send, mut recv) = conn.accept_bi().await.map_err(net_err)?;
    let hello = recv.read_to_end(MAX_HELLO_BYTES).await.map_err(net_err)?;
    println!("Reçu : {}", String::from_utf8_lossy(&hello));

    let reply = format!(
        "lugus/{PROTOCOL_VERSION} : bonjour, ici l'émetteur de « {} »",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    );
    send.write_all(reply.as_bytes()).await.map_err(net_err)?;
    send.finish().map_err(net_err)?;

    // Laisse au récepteur le temps de lire la réponse et de fermer la connexion.
    let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    eprintln!("Chemin final : {}", current_path(&conn));
    beacon.close().await;
    Ok(())
}

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

    eprintln!("Code valide en local : {access}");
    eprintln!("Destination : {}", dest.display());
    eprintln!("Recherche de l'émetteur dans la DHT…");

    let beacon = Beacon::bind(Role::Receiver).await?;
    let started = Instant::now();
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, beacon.connect(&link))
        .await
        .context("émetteur introuvable dans le délai imparti")??;
    eprintln!(
        "Connecté en {:.1} s, chemin : {}",
        started.elapsed().as_secs_f32(),
        current_path(&conn)
    );

    let (mut send, mut recv) = conn.open_bi().await.map_err(net_err)?;
    let hello = format!("lugus/{PROTOCOL_VERSION} : bonjour, ici le destinataire");
    send.write_all(hello.as_bytes()).await.map_err(net_err)?;
    send.finish().map_err(net_err)?;

    let reply = recv.read_to_end(MAX_HELLO_BYTES).await.map_err(net_err)?;
    println!("Reçu : {}", String::from_utf8_lossy(&reply));
    eprintln!("Chemin final : {}", current_path(&conn));

    conn.close(0u32.into(), b"fin");
    beacon.close().await;
    Ok(())
}

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
