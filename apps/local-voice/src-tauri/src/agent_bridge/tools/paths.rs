//! Pfadpruefung fuer `transcribe_file` (A8): der Pfad kommt von einem Agenten und ist deshalb
//! unvertraut. Er darf nur auf eine lokale Audio- oder Videodatei zeigen.
//!
//! Zwei Stufen:
//! 1. `check_syntax` (rein, ohne Dateisystem): nur ein absoluter Pfad mit Laufwerksbuchstaben,
//!    keine UNC-Adresse (`\\server\freigabe`), kein Geraete- oder Namensraumpfad (`\\.\`, `\\?\`),
//!    kein `..`, kein Datenstrom (`datei.wav:strom`), keine reservierten Geraetenamen (`CON`,
//!    `NUL`, `COM1`, ...), keine Platzhalter, keine Steuerzeichen, eine erlaubte Endung.
//! 2. `resolve` (Dateisystem): der Pfad wird aufgeloest (Verknuepfungen und Junctions folgen),
//!    das Ziel muss wieder lokal sein (ein Laufwerk, das auf eine Netzfreigabe zeigt, wird
//!    abgewiesen), wieder eine erlaubte Endung haben, eine echte, nicht leere und nicht
//!    uebergrosse Datei sein.
//!
//! Grenze: zwischen der Pruefung und dem Lesen der Datei durch die Import-Warteschlange kann
//! jemand mit Schreibrechten im selben Ordner die Datei austauschen. Das ist derselbe Benutzer,
//! dem die Datei ohnehin gehoert; die Warteschlange liest nur und das Ergebnis bleibt lokal.

use std::path::{Path, PathBuf};

use crate::media::MEDIA_EXTENSIONS;

/// Laengster angenommener Pfad (Zeichen). Windows kennt 260 (ohne Namensraumpraefix).
pub const MAX_PATH_CHARS: usize = 520;
/// Groesste Datei, die ein Agent einreihen darf (16 GiB).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Warum ein Pfad abgewiesen wird.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    TooLong,
    /// Steuerzeichen, NUL oder unsichtbare Umschaltzeichen.
    BadCharacters,
    /// Kein absoluter Pfad mit Laufwerksbuchstaben (relativ, laufwerksrelativ, Wurzel ohne Laufwerk).
    NotAbsolute,
    /// `\\server\freigabe\...` oder `//server/...`.
    Network,
    /// `\\.\...` oder `\\?\...` (Geraete- und Namensraumpfade).
    DevicePath,
    /// `..` als Pfadteil.
    Traversal,
    /// Doppelpunkt hinter dem Laufwerk (Datenstrom, Geraetename mit Doppelpunkt).
    StreamSyntax,
    /// `CON`, `NUL`, `COM1`, ... (auch mit Endung).
    ReservedName(String),
    /// Platzhalter, verbotene Zeichen, Punkt oder Leerzeichen am Ende eines Namens.
    BadName,
    /// Endung nicht in der Liste der Audio-/Videoformate.
    BadExtension(String),
    NotFound,
    NotAFile,
    EmptyFile,
    TooLarge(u64),
    Unreadable(String),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Empty => write!(f, "Der Pfad fehlt."),
            PathError::TooLong => write!(f, "Der Pfad ist zu lang."),
            PathError::BadCharacters => write!(f, "Der Pfad enthält Steuerzeichen."),
            PathError::NotAbsolute => write!(
                f,
                "Der Pfad muss absolut sein und mit einem Laufwerk beginnen (z. B. C:\\Aufnahmen\\datei.wav)."
            ),
            PathError::Network => write!(f, "Netzwerkpfade (\\\\Server\\Freigabe) sind nicht erlaubt."),
            PathError::DevicePath => write!(f, "Geräte- und Namensraumpfade (\\\\.\\, \\\\?\\) sind nicht erlaubt."),
            PathError::Traversal => write!(f, "Der Pfad darf kein „..“ enthalten."),
            PathError::StreamSyntax => write!(f, "Der Pfad enthält einen Doppelpunkt hinter dem Laufwerk."),
            PathError::ReservedName(n) => write!(f, "„{n}“ ist ein reservierter Gerätename."),
            PathError::BadName => write!(f, "Der Pfad enthält Platzhalter oder unzulässige Zeichen."),
            PathError::BadExtension(e) => write!(
                f,
                "Das Dateiformat „{e}“ wird nicht transkribiert (erlaubt: {}).",
                MEDIA_EXTENSIONS.join(", ")
            ),
            PathError::NotFound => write!(f, "Die Datei gibt es nicht."),
            PathError::NotAFile => write!(f, "Der Pfad zeigt nicht auf eine Datei."),
            PathError::EmptyFile => write!(f, "Die Datei ist leer."),
            PathError::TooLarge(b) => write!(
                f,
                "Die Datei ist zu groß ({} GiB, höchstens {} GiB).",
                b / (1024 * 1024 * 1024),
                MAX_FILE_BYTES / (1024 * 1024 * 1024)
            ),
            PathError::Unreadable(m) => write!(f, "Die Datei lässt sich nicht lesen ({m})."),
        }
    }
}

impl std::error::Error for PathError {}

fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Ist `component` ein reservierter Windows-Geraetename (`CON`, `NUL`, `COM1` ..., auch mit Endung)?
pub fn reserved_device_name(component: &str) -> Option<String> {
    // Reserviert ist der Teil vor dem ersten Punkt: `con.txt` und `NUL.wav` meinen das Geraet.
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ');
    let up = stem.to_ascii_uppercase();
    let reserved = matches!(
        up.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((up.starts_with("COM") || up.starts_with("LPT"))
        && up.len() == 4
        && matches!(up.as_bytes()[3], b'1'..=b'9'));
    reserved.then(|| stem.to_string())
}

fn extension_of(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn check_extension(path: &str) -> Result<(), PathError> {
    let ext = extension_of(path);
    if MEDIA_EXTENSIONS.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(PathError::BadExtension(ext))
    }
}

/// Prueft die Schreibweise (siehe Moduldoku, Stufe 1). Liefert den bereinigten Pfad.
pub fn check_syntax(raw: &str) -> Result<PathBuf, PathError> {
    let path = raw.trim();
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(PathError::TooLong);
    }
    if path.chars().any(is_invisible) {
        return Err(PathError::BadCharacters);
    }
    let prefix: String = path.chars().take(4).collect();
    if matches!(prefix.as_str(), r"\\?\" | r"\\.\" | "//?/" | "//./") {
        return Err(PathError::DevicePath);
    }
    if path.starts_with(r"\\") || path.starts_with("//") {
        return Err(PathError::Network);
    }
    let bytes = path.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if !drive_absolute {
        return Err(PathError::NotAbsolute);
    }
    let rest = &path[2..];
    if rest.contains(':') {
        return Err(PathError::StreamSyntax);
    }
    let mut last = "";
    for component in rest.split(['\\', '/']).filter(|c| !c.is_empty()) {
        if component == ".." {
            return Err(PathError::Traversal);
        }
        if component == "." {
            continue;
        }
        if component
            .chars()
            .any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
            || component.ends_with('.')
            || component.ends_with(' ')
        {
            return Err(PathError::BadName);
        }
        if let Some(name) = reserved_device_name(component) {
            return Err(PathError::ReservedName(name));
        }
        last = component;
    }
    if last.is_empty() {
        return Err(PathError::NotAFile);
    }
    check_extension(path)?;
    Ok(PathBuf::from(path))
}

/// Entfernt den Namensraumpraefix, den `canonicalize` unter Windows voranstellt. Netzfreigaben
/// (`\\?\UNC\`) und alles, was danach nicht mehr nach `X:\` aussieht, bleibt abgewiesen.
fn strip_verbatim(canonical: &Path) -> Result<PathBuf, PathError> {
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        if rest
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("UNC\\"))
        {
            return Err(PathError::Network);
        }
        let b = rest.as_bytes();
        let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
        if !drive {
            return Err(PathError::DevicePath);
        }
        return Ok(PathBuf::from(rest));
    }
    if text.starts_with(r"\\") {
        return Err(PathError::Network);
    }
    Ok(canonical.to_path_buf())
}

/// Dateisystem-Stufe (siehe Moduldoku, Stufe 2). `max_bytes` ist fuer Tests einstellbar.
pub fn resolve_with(path: &Path, max_bytes: u64) -> Result<PathBuf, PathError> {
    let canonical = std::fs::canonicalize(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => PathError::NotFound,
        _ => PathError::Unreadable(e.kind().to_string()),
    })?;
    let local = strip_verbatim(&canonical)?;
    // Das Ziel einer Verknuepfung muss selbst ein erlaubtes Format sein.
    check_extension(&local.to_string_lossy())?;
    let meta =
        std::fs::metadata(&local).map_err(|e| PathError::Unreadable(e.kind().to_string()))?;
    if !meta.is_file() {
        return Err(PathError::NotAFile);
    }
    if meta.len() == 0 {
        return Err(PathError::EmptyFile);
    }
    if meta.len() > max_bytes {
        return Err(PathError::TooLarge(meta.len()));
    }
    Ok(local)
}

/// Schreibweise und Dateisystem zusammen: der Weg von `transcribe_file`.
pub fn check_media_path(raw: &str) -> Result<PathBuf, PathError> {
    let syntax = check_syntax(raw)?;
    resolve_with(&syntax, MAX_FILE_BYTES)
}

#[cfg(test)]
mod tests;
