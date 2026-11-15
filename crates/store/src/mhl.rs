//! MHL 1.1 (format « hashlist »), en xxHash64, comme Bandua.
//!
//! Lu par Silverstack, ShotPut, Hedge, YoYotta, mhl-tool et Bandua MHL Companion.
//! Le hash est celui du fichier **relu depuis le disque de destination** après
//! vérification BLAKE3 bloc par bloc contre la source.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};

/// Une ligne du MHL.
pub struct MhlEntry {
    /// Chemin relatif au dossier du MHL, séparateur `/`.
    pub path: String,
    pub size: u64,
    pub modified_unix: Option<i64>,
    pub xxh64: u64,
}

/// Écrit `dir/file_name` et renvoie son chemin.
pub fn write_mhl(
    dir: &Path,
    file_name: &str,
    entries: &[MhlEntry],
    started: DateTime<Local>,
    finished: DateTime<Local>,
    tool: &str,
) -> io::Result<PathBuf> {
    let path = dir.join(file_name);
    fs::write(&path, render(entries, started, finished, tool))?;
    Ok(path)
}

fn render(
    entries: &[MhlEntry],
    started: DateTime<Local>,
    finished: DateTime<Local>,
    tool: &str,
) -> String {
    let (user, host) = user_host();
    let mut sorted: Vec<&MhlEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));

    let mut x = String::new();
    let _ = writeln!(x, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(x, "<hashlist version=\"1.1\">");
    let _ = writeln!(x, "  <creatorinfo>");
    let _ = writeln!(x, "    <name>{}</name>", xml_escape(&user));
    let _ = writeln!(x, "    <username>{}</username>", xml_escape(&user));
    let _ = writeln!(x, "    <hostname>{}</hostname>", xml_escape(&host));
    let _ = writeln!(x, "    <tool>{}</tool>", xml_escape(tool));
    let _ = writeln!(x, "    <startdate>{}</startdate>", iso(&started));
    let _ = writeln!(x, "    <finishdate>{}</finishdate>", iso(&finished));
    let _ = writeln!(x, "  </creatorinfo>");
    for entry in sorted {
        let modified = entry
            .modified_unix
            .and_then(|secs| Local.timestamp_opt(secs, 0).single())
            .map(|d| iso(&d))
            .unwrap_or_default();
        let _ = writeln!(x, "  <hash>");
        let _ = writeln!(x, "    <file>{}</file>", xml_escape(&entry.path));
        let _ = writeln!(x, "    <size>{}</size>", entry.size);
        let _ = writeln!(
            x,
            "    <lastmodificationdate>{modified}</lastmodificationdate>"
        );
        let _ = writeln!(x, "    <xxhash64be>{:016x}</xxhash64be>", entry.xxh64);
        let _ = writeln!(x, "    <hashdate>{}</hashdate>", iso(&finished));
        let _ = writeln!(x, "  </hash>");
    }
    let _ = writeln!(x, "</hashlist>");
    x
}

fn user_host() -> (String, String) {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    (user, host)
}

fn iso<Tz: TimeZone>(d: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    d.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_hashlist() {
        let now = Local::now();
        let entries = [MhlEntry {
            path: "A001/clip & co.mov".to_string(),
            size: 42,
            modified_unix: Some(1_700_000_000),
            xxh64: 0xef46db3751d8e999,
        }];
        let xml = render(
            &entries,
            now,
            now,
            "Lugus Transfer 0.1.0 (Domingo Olmo Martin)",
        );
        assert!(xml.contains("<hashlist version=\"1.1\">"));
        assert!(xml.contains("<file>A001/clip &amp; co.mov</file>"));
        assert!(xml.contains("<size>42</size>"));
        assert!(xml.contains("<xxhash64be>ef46db3751d8e999</xxhash64be>"));
        assert!(xml.contains("<tool>Lugus Transfer 0.1.0 (Domingo Olmo Martin)</tool>"));
    }
}
