//! Énergie : garder la machine éveillée, programmer un départ, agir à la fin.

use std::io;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{Local, NaiveTime, TimeDelta, TimeZone};
use serde::{Deserialize, Serialize};

/// Empêche la mise en veille du système (pas de l'écran) tant qu'il existe.
///
/// Sous Windows, la demande est liée au thread qui l'a faite : elle est donc
/// faite et levée dans un thread dédié, et non dans une tâche asynchrone qui
/// peut changer de thread.
pub struct Awake {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Awake {
    pub fn hold(reason: &str) -> Self {
        let (stop, wait) = mpsc::channel::<()>();
        let reason = reason.to_string();
        let thread = std::thread::Builder::new()
            .name("lugus-anti-veille".into())
            .spawn(move || {
                let guard = keepawake::Builder::default()
                    .idle(true)
                    .reason(reason)
                    .app_name("Lugus Transfer")
                    .app_reverse_domain("com.domingoolmomartin.lugus")
                    .create();
                // Bloque jusqu'à la fermeture du canal (fin de l'objet `Awake`).
                let _ = wait.recv();
                drop(guard);
            })
            .ok();
        Self {
            stop: Some(stop),
            thread,
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Action à la fin d'un transfert (mode Noctiluca).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndAction {
    #[default]
    Nothing,
    Sleep,
    Shutdown,
}

impl std::str::FromStr for EndAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "rien" | "nothing" | "none" => Ok(Self::Nothing),
            "veille" | "sleep" => Ok(Self::Sleep),
            "extinction" | "eteindre" | "shutdown" => Ok(Self::Shutdown),
            other => Err(format!(
                "action inconnue : {other} (rien, veille, extinction)"
            )),
        }
    }
}

/// Exécute l'action de fin. L'extinction sous Windows laisse 60 s pour annuler
/// (`shutdown /a`).
pub fn perform(action: EndAction) -> io::Result<()> {
    let (program, args): (&str, &[&str]) = match action {
        EndAction::Nothing => return Ok(()),
        EndAction::Sleep => sleep_command(),
        EndAction::Shutdown => shutdown_command(),
    };
    let status = std::process::Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} a échoué ({status})")))
    }
}

#[cfg(windows)]
fn sleep_command() -> (&'static str, &'static [&'static str]) {
    ("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"])
}

#[cfg(windows)]
fn shutdown_command() -> (&'static str, &'static [&'static str]) {
    (
        "shutdown",
        &["/s", "/t", "60", "/c", "Lugus Transfer : transfert terminé"],
    )
}

#[cfg(target_os = "macos")]
fn sleep_command() -> (&'static str, &'static [&'static str]) {
    ("pmset", &["sleepnow"])
}

#[cfg(target_os = "macos")]
fn shutdown_command() -> (&'static str, &'static [&'static str]) {
    (
        "osascript",
        &["-e", "tell application \"System Events\" to shut down"],
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn sleep_command() -> (&'static str, &'static [&'static str]) {
    ("systemctl", &["suspend"])
}

#[cfg(not(any(windows, target_os = "macos")))]
fn shutdown_command() -> (&'static str, &'static [&'static str]) {
    ("systemctl", &["poweroff"])
}

/// Délai jusqu'à la prochaine occurrence de l'heure `HH:MM` (heure locale).
pub fn delay_until(hhmm: &str) -> Result<Duration, String> {
    let time = NaiveTime::parse_from_str(hhmm.trim(), "%H:%M")
        .map_err(|_| format!("heure invalide : {hhmm} (format HH:MM)"))?;
    let now = Local::now();
    let mut target = now.date_naive().and_time(time);
    if target <= now.naive_local() {
        target += TimeDelta::days(1);
    }
    let target = Local
        .from_local_datetime(&target)
        .earliest()
        .ok_or_else(|| format!("heure inexistante ce jour-là : {hhmm}"))?;
    (target - now)
        .to_std()
        .map_err(|_| "heure déjà passée".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_end_action() {
        assert_eq!("veille".parse::<EndAction>().unwrap(), EndAction::Sleep);
        assert_eq!(
            "Shutdown".parse::<EndAction>().unwrap(),
            EndAction::Shutdown
        );
        assert_eq!("rien".parse::<EndAction>().unwrap(), EndAction::Nothing);
        assert!("plouf".parse::<EndAction>().is_err());
    }

    #[test]
    fn delay_is_within_a_day() {
        let delay = delay_until("03:30").unwrap();
        assert!(delay <= Duration::from_secs(24 * 3600));
        assert!(delay_until("25:00").is_err());
        assert!(delay_until("midi").is_err());
    }

    #[test]
    fn awake_can_be_dropped() {
        let awake = Awake::hold("test");
        drop(awake);
    }
}
