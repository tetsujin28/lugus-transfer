//! Lugus Transfer en ligne de commande : `lugus send`, `lugus receive`, `lugus code`.
//!
//! Toute la logique est dans `lugus_transfer::session` ; ce binaire ne fait
//! qu'afficher les événements, comme l'application le fait dans sa fenêtre.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use lugus_core::code::{self, AccessCode};
use lugus_transfer::power::EndAction;
use lugus_transfer::session::{
    run_receiver, run_sender, Emit, Event, ReceiveOptions, SendOptions, SenderCommand,
};
use lugus_transfer::DEFAULT_PARALLEL_FILES;
use tokio::sync::mpsc;

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
        /// Après la livraison : rien, veille ou extinction
        #[arg(long, default_value = "rien")]
        apres: EndAction,
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
        /// Nombre de fichiers reçus en parallèle
        #[arg(long, default_value_t = DEFAULT_PARALLEL_FILES)]
        parallel: usize,
        /// Mode Noctiluca : heure de départ, au format HH:MM
        #[arg(long)]
        depart: Option<String>,
        /// Après la réception : rien, veille ou extinction
        #[arg(long, default_value = "rien")]
        apres: EndAction,
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
        Command::Send { path, apres } => {
            warn_if_wordlist_is_small();
            let (commands, receiver) = mpsc::unbounded_channel();
            let emit = printer(Some(commands));
            run_sender(
                SendOptions {
                    path,
                    end_action: apres,
                },
                emit,
                receiver,
            )
            .await
        }
        Command::Receive {
            link,
            code,
            dest,
            parallel,
            depart,
            apres,
        } => {
            let code = match code {
                Some(code) => code,
                None => prompt("Code : ")?,
            };
            run_receiver(
                ReceiveOptions {
                    link,
                    code,
                    dest,
                    parallel,
                    start_at: depart,
                    end_action: apres,
                },
                printer(None),
            )
            .await
        }
        Command::Code { check } => cmd_code(check),
    }
}

/// Affiche les événements d'une session. `commands` permet de répondre à un
/// blocage du code côté émetteur.
fn printer(commands: Option<mpsc::UnboundedSender<SenderCommand>>) -> Emit {
    let first_progress: Mutex<Option<(Instant, u64)>> = Mutex::new(None);
    Arc::new(move |event: Event| match event {
        Event::Ready {
            link,
            code,
            name,
            files,
            bytes,
            relay,
        } => {
            println!();
            println!("Lien : {link}");
            println!("Code : {code}");
            println!("Validité : 24 h, 3 essais");
            println!("Contenu « {name} » : {files} fichier(s), {}", human(bytes));
            if !relay {
                eprintln!("Attention : aucun relais joignable, connexions directes seulement.");
            }
            eprintln!("En attente du destinataire, Ctrl+C pour arrêter.");
        }
        Event::NewCode { code } => println!("Nouveau code : {code}"),
        Event::Incoming { path } => eprintln!("Connexion entrante ({})", path_label(&path)),
        Event::WrongCode { remaining } => {
            eprintln!("Code incorrect reçu, {remaining} essai(s) restant(s).")
        }
        Event::Locked => {
            eprintln!("3 essais ratés : le code est bloqué, le lien reste valable.");
            if let Some(commands) = commands.clone() {
                tokio::spawn(async move {
                    match ask_yes_no("Générer un nouveau code ?").await {
                        Ok(true) => {
                            let _ = commands.send(SenderCommand::NewCode);
                        }
                        _ => {
                            eprintln!("Envoi annulé.");
                            std::process::exit(1);
                        }
                    }
                });
            }
        }
        Event::Authenticated { path } => eprintln!("Code accepté ({}).", path_label(&path)),
        Event::Selected { files, bytes } => {
            eprintln!(
                "Le destinataire a choisi {files} fichier(s), {}.",
                human(bytes)
            )
        }
        Event::Interrupted { message } => {
            eprintln!();
            eprintln!("Transfert interrompu : {message}");
            eprintln!("En attente d'une reprise par le destinataire…");
        }
        Event::Delivered {
            files,
            bytes,
            receipt,
            path,
        } => {
            eprintln!();
            eprintln!(
                "Livré et vérifié : {files} fichier(s), {} ({}). Accusé de réception authentique.",
                human(bytes),
                path_label(&path)
            );
            eprintln!("Empreinte de l'accusé : {receipt}");
        }
        Event::Expired => eprintln!("Le lien a expiré."),
        Event::Waiting { until, seconds } => eprintln!(
            "Mode Noctiluca : départ à {until} (dans {} min).",
            seconds.div_ceil(60)
        ),
        Event::Searching => eprintln!("Recherche de l'émetteur dans la DHT…"),
        Event::Found { seconds, path } => {
            eprintln!("Émetteur trouvé en {seconds:.1} s ({}).", path_label(&path))
        }
        Event::Manifest {
            name,
            files,
            bytes,
            originals,
            proxies,
            sidecars,
        } => {
            println!("Code accepté.");
            println!("Contenu « {name} » : {files} fichier(s), {}", human(bytes));
            println!(
                "  {originals} original(aux), {proxies} proxy(s), {sidecars} fichier(s) annexe(s)"
            );
        }
        Event::FileVerified { .. } => {}
        Event::Received {
            files,
            bytes,
            seconds,
            mhl,
            path,
            ..
        } => {
            eprintln!();
            eprintln!(
                "Reçu et vérifié : {files} fichier(s), {} en {seconds:.0} s ({}).",
                human(bytes),
                path_label(&path)
            );
            eprintln!("MHL : {mhl}");
        }
        Event::Progress { done, total, path } => {
            let mut first = first_progress.lock().expect("verrou empoisonné");
            let (since, base) = *first.get_or_insert((Instant::now(), done));
            let elapsed = since.elapsed().as_secs_f64().max(0.001);
            let rate = done.saturating_sub(base) as f64 / elapsed;
            let percent = if total == 0 {
                100.0
            } else {
                done as f64 * 100.0 / total as f64
            };
            eprint!(
                "\r{} / {} ({percent:.0} %), {}/s, {}     ",
                human(done),
                human(total),
                human(rate as u64),
                path_label(&path)
            );
            let _ = io::stderr().flush();
        }
        Event::EndAction { action } => eprintln!("Action de fin : {action:?}"),
        Event::Error { message } => eprintln!("Erreur : {message}"),
    })
}

fn path_label(path: &str) -> &'static str {
    match path {
        "local" => "réseau local",
        "direct" => "direct",
        "relay" => "relais",
        _ => "chemin inconnu",
    }
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

fn human(bytes: u64) -> String {
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

fn prompt(label: &str) -> Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
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
