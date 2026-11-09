//! Wo der Sprachdienst lauscht. Beide Seiten leiten den Ort gleich ab; die App übergibt nie einen
//! Pfad aus der Oberfläche.
//!
//! - Windows: Named Pipe `\\.\pipe\wai-sprache-<SID>` (SID des angemeldeten Benutzers). Der Dienst
//!   legt sie mit expliziter DACL, `PIPE_REJECT_REMOTE_CLIENTS` und `FILE_FLAG_FIRST_PIPE_INSTANCE`
//!   an (Leitfaden `sprachdienst-protokoll.md`).
//! - macOS/Linux: Unix-Socket `<Laufzeitordner>/wai/sprache.sock`, Ordner `0700`, Socket `0600`.
//!   Laufzeitordner ist `XDG_RUNTIME_DIR`, unter macOS ersatzweise `TMPDIR` (je Benutzer getrennt).

use std::path::{Path, PathBuf};

/// Name der Pipe für eine Benutzer-SID (`S-1-5-21-…`). `None`, wenn die SID nicht wie eine aussieht.
pub fn pipe_name(sid: &str) -> Option<String> {
    let teile: Vec<&str> = sid.split('-').collect();
    let ok = sid.len() <= 184
        && teile.len() >= 4
        && teile[0] == "S"
        && teile[1] == "1"
        && teile[2..]
            .iter()
            .all(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| format!(r"\\.\pipe\wai-sprache-{sid}"))
}

/// Socket-Pfad unter einem Laufzeitordner.
pub fn socket_pfad(laufzeit: &Path) -> PathBuf {
    laufzeit.join("wai").join("sprache.sock")
}

/// Laufzeitordner des Benutzers aus der Umgebung: `XDG_RUNTIME_DIR`, nur unter macOS auch
/// `TMPDIR` (dort je Benutzer getrennt; unter Linux ist `TMPDIR` oft das gemeinsame `/tmp`). Nur
/// absolute Pfade zählen; `None` heißt: kein sicherer Ort bekannt, Sprache bleibt aus. Beim Verbinden
/// prüft die App zusätzlich Besitzer und Modus des Ordners (`0700`, eigene UID) und die Peer-UID.
pub fn laufzeitordner() -> Option<PathBuf> {
    let namen: &[&str] = if cfg!(target_os = "macos") {
        &["XDG_RUNTIME_DIR", "TMPDIR"]
    } else {
        &["XDG_RUNTIME_DIR"]
    };
    namen
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_nur_fuer_sids() {
        assert_eq!(
            pipe_name("S-1-5-21-1-2-3-1001").as_deref(),
            Some(r"\\.\pipe\wai-sprache-S-1-5-21-1-2-3-1001")
        );
        assert_eq!(pipe_name(r"S-1-5\..\x"), None);
        assert_eq!(pipe_name("jeder"), None);
        assert_eq!(pipe_name("S-1-"), None);
        assert_eq!(pipe_name("S-1--5"), None);
        assert_eq!(pipe_name("S-1-5"), None);
    }

    #[test]
    fn socket_unter_laufzeit() {
        assert_eq!(
            socket_pfad(Path::new("/run/user/1000")),
            PathBuf::from("/run/user/1000/wai/sprache.sock")
        );
    }
}
