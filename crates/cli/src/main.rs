//! Binaire de test de Lugus Transfer : `lugus send`, `lugus receive`, `lugus code`.
//!
//! Squelette : le réseau n'est pas encore branché. Les commandes utilisent déjà
//! le vrai code d'accès et le vrai format de lien du crate `lugus-core`.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lugus_core::code::{self, AccessCode};
use lugus_core::link::LugusLink;
use lugus_core::{DEFAULT_LINK_TTL_SECS, MAX_CODE_ATTEMPTS};

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Send { path } => cmd_send(path),
        Command::Receive { link, code, dest } => cmd_receive(link, code, dest),
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

fn cmd_send(path: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("{} n'existe pas", path.display());
    }
    warn_if_wordlist_is_small();

    let access = AccessCode::generate().context("génération du code")?;
    println!("Code : {access}");
    println!("Validité : {} h", DEFAULT_LINK_TTL_SECS / 3600);
    println!("Essais : {MAX_CODE_ATTEMPTS} par lien");
    println!();
    println!(
        "Squelette : la publication dans la DHT arrive avec le crate noctiluca, \
         il n'y a pas encore de lien."
    );
    Ok(())
}

fn cmd_receive(link: String, code: Option<String>, dest: PathBuf) -> Result<()> {
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

    println!("Lien valide, clé de l'émetteur : {}", link.encode_key());
    println!("Code valide : {access}");
    println!("Destination : {}", dest.display());
    println!();
    println!("Squelette : la connexion arrive avec les crates noctiluca et auth.");
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
