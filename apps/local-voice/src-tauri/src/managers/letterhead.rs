//! Briefkopf-Profile `lva.briefkopf/1` (Entwurf 2026-10-13, Abschnitt 4; Etappe 2).
//!
//! Ein Briefkopf ist das Erscheinungsbild exportierter Dokumente (Word, PDF): Firmenname
//! und Zeilen, Logo, Farben, Fusszeile mit Seitenzahl. Er ist austauschbar:
//! - **mitgeliefert** (schreibgeschuetzt, im Code): `neutral`, `wai`, `ingenieurbuero`;
//! - **eigene** Profile als Ordner `<App-Daten>/briefkoepfe/<id>/` mit `briefkopf.json`
//!   und optional `logo.png` / `logo.jpg`.
//!
//! Owner-Annahmen (Entwurf Abschnitt 10), hier als Konstanten:
//! - Annahme 3: alle drei mitgeliefert, Vorgabe `neutral` ([`DEFAULT_ID`]).
//! - Annahme 4: keine Rechtsangaben (USt-IdNr., Bank) in den mitgelieferten Fusszeilen.
//! - Die Farben der Marken spiegeln `design-system_wolffappliedai` (WAI-Gelb nur als Flaeche
//!   mit Ink-Text, Ingenieurbuero Navy mit weissem Text); Aenderungen kommen von dort.
//!
//! Sicherheit: Ids sind `[a-z0-9-]{1,40}`, Pfade entstehen nur aus Wurzel + Id + festem
//! Dateinamen (keine Pfadflucht). Logos werden beim Setzen geprueft (PNG/JPEG an den
//! Kopfbytes, hoechstens 2 MB, hoechstens 4000 px Kante) und in den Profilordner kopiert.
//! Geschrieben wird ueber eine `.part`-Datei und Umbenennen.
//!
//! Kein Zugriff auf `settings::get_settings`: die Wahl kommt als Argument.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Formatkennung.
pub const SCHEMA: &str = "lva.briefkopf/1";
/// Vorgabe, solange der Nutzer nichts waehlt (Owner-Annahme 3).
pub const DEFAULT_ID: &str = "neutral";
/// Ordner unter den App-Daten.
pub const DIR_NAME: &str = "briefkoepfe";
const FILE_NAME: &str = "briefkopf.json";
/// Groesste Logodatei.
pub const MAX_LOGO_BYTES: u64 = 2 * 1024 * 1024;
/// Laengste Logokante in Pixeln.
pub const MAX_LOGO_PX: u32 = 4000;
const MAX_LINES: usize = 4;
const MAX_FOOTER_LINES: usize = 3;
const MAX_TEXT: usize = 120;

const WAI_LOGO: &[u8] = include_bytes!("../../assets/documents/logo-wai-w.png");
const IB_LOGO: &[u8] = include_bytes!("../../assets/documents/logo-ingenieurbuero-w.png");

/// Firmenangaben im Kopf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Firma {
    pub name: String,
    #[serde(default)]
    pub zeilen: Vec<String>,
}

/// Wo das Logo steht.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LogoPosition {
    Links,
    Rechts,
}

/// Logo eines Profils: Datei im Profilordner (`logo-<ulid>.png|jpg`, aeltere
/// Profile `logo.png`/`logo.jpg`) oder, bei mitgelieferten Profilen,
/// `builtin:<name>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Logo {
    pub datei: String,
    /// Breite im Dokument in Millimetern.
    pub breite_mm: f32,
    pub position: LogoPosition,
}

/// Farben als `#RRGGBB`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Farben {
    /// Akzentflaeche (Balken, Tabellenkopf).
    pub akzent: String,
    /// Text auf der Akzentflaeche.
    pub akzent_text: String,
    /// Fliesstext.
    pub text: String,
    /// Ueberschriften.
    pub titel: String,
    /// Kopf-/Fusszeilen, Hinweise.
    pub gedaempft: String,
    /// Linien und Tabellenraster.
    pub linie: String,
}

/// Fusszeile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Fusszeile {
    #[serde(default)]
    pub zeilen: Vec<String>,
    #[serde(default = "yes")]
    pub seitenzahl: bool,
}

fn yes() -> bool {
    true
}

/// Ein Briefkopf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Letterhead {
    pub schema: String,
    pub id: String,
    pub name: String,
    /// Mitgeliefert (schreibgeschuetzt). Wird nicht gespeichert, sondern beim Laden gesetzt.
    #[serde(default, skip_serializing)]
    pub builtin: bool,
    pub firma: Firma,
    #[serde(default)]
    pub logo: Option<Logo>,
    pub farben: Farben,
    pub fusszeile: Fusszeile,
}

/// Fehlercodes fuer die Oberflaeche (`letterhead_*`).
pub const CODE_NOT_FOUND: &str = "letterhead_not_found";
pub const CODE_READ_ONLY: &str = "letterhead_read_only";
pub const CODE_INVALID: &str = "letterhead_invalid";
pub const CODE_LOGO: &str = "letterhead_logo_invalid";
pub const CODE_IO: &str = "letterhead_io";

// ---------------------------------------------------------------------------
// Mitgeliefert
// ---------------------------------------------------------------------------

fn colors(akzent: &str, akzent_text: &str, titel: &str) -> Farben {
    Farben {
        akzent: akzent.into(),
        akzent_text: akzent_text.into(),
        text: "#1F2937".into(),
        titel: titel.into(),
        gedaempft: "#626976".into(),
        linie: "#E1E6EC".into(),
    }
}

/// Die mitgelieferten Profile in Anzeigereihenfolge.
pub fn builtins() -> Vec<Letterhead> {
    vec![
        Letterhead {
            schema: SCHEMA.into(),
            id: DEFAULT_ID.into(),
            name: "Neutral".into(),
            builtin: true,
            firma: Firma {
                name: String::new(),
                zeilen: vec![],
            },
            logo: None,
            farben: colors("#E5E9EF", "#111418", "#14283F"),
            fusszeile: Fusszeile {
                zeilen: vec![],
                seitenzahl: true,
            },
        },
        Letterhead {
            schema: SCHEMA.into(),
            id: "wai".into(),
            name: "Wolff Applied AI".into(),
            builtin: true,
            firma: Firma {
                name: "Wolff Applied AI".into(),
                zeilen: vec!["Applied Artificial Intelligence".into()],
            },
            logo: Some(Logo {
                datei: "builtin:wai".into(),
                breite_mm: 14.0,
                position: LogoPosition::Links,
            }),
            // Gelb nur als Flaeche mit Ink darauf; Ueberschriften Ink.
            farben: colors("#FFDD00", "#111418", "#111418"),
            fusszeile: Fusszeile {
                zeilen: vec!["Wolff Applied AI · info@wolffappliedai.de".into()],
                seitenzahl: true,
            },
        },
        Letterhead {
            schema: SCHEMA.into(),
            id: "ingenieurbuero".into(),
            name: "Ingenieurbüro Wolff".into(),
            builtin: true,
            firma: Firma {
                name: "Ingenieurbüro Wolff".into(),
                zeilen: vec!["Beratung & Schulung · IT · KI · Digitalisierung".into()],
            },
            logo: Some(Logo {
                datei: "builtin:ingenieurbuero".into(),
                breite_mm: 14.0,
                position: LogoPosition::Links,
            }),
            farben: colors("#004B85", "#FFFFFF", "#004B85"),
            fusszeile: Fusszeile {
                zeilen: vec!["Ingenieurbüro Wolff · patrick.wolff@ingenieurbuerowolff.de".into()],
                seitenzahl: true,
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Pruefung
// ---------------------------------------------------------------------------

/// `[a-z0-9-]{1,40}`, nicht mit `-` beginnend.
pub fn valid_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `#RRGGBB` -> (r, g, b).
pub fn parse_color(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn short(text: &str) -> bool {
    text.chars().count() <= MAX_TEXT && !text.chars().any(char::is_control)
}

/// Prueft ein Profil; Fehler `letterhead_invalid:<feld>`.
pub fn validate(lh: &Letterhead) -> Result<(), String> {
    let bad = |field: &str| Err(format!("{CODE_INVALID}:{field}"));
    if lh.schema != SCHEMA {
        return bad("schema");
    }
    if !valid_id(&lh.id) {
        return bad("id");
    }
    let name = lh.name.trim();
    if name.is_empty() || !short(name) {
        return bad("name");
    }
    if !short(&lh.firma.name) {
        return bad("firma");
    }
    if lh.firma.zeilen.len() > MAX_LINES || !lh.firma.zeilen.iter().all(|l| short(l)) {
        return bad("zeilen");
    }
    if lh.fusszeile.zeilen.len() > MAX_FOOTER_LINES || !lh.fusszeile.zeilen.iter().all(|l| short(l))
    {
        return bad("fusszeile");
    }
    let f = &lh.farben;
    for (field, value) in [
        ("akzent", &f.akzent),
        ("akzent_text", &f.akzent_text),
        ("text", &f.text),
        ("titel", &f.titel),
        ("gedaempft", &f.gedaempft),
        ("linie", &f.linie),
    ] {
        if parse_color(value).is_none() {
            return bad(field);
        }
    }
    if let Some(logo) = &lh.logo {
        let ok_file =
            valid_logo_file(&logo.datei) || (lh.builtin && logo.datei.starts_with("builtin:"));
        if !ok_file {
            return bad("logo");
        }
        if !(5.0..=80.0).contains(&logo.breite_mm) {
            return bad("logo_breite");
        }
    }
    Ok(())
}

/// Bildart eines Logos an den Kopfbytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
}

impl ImageKind {
    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
        }
    }
}

pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(ImageKind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageKind::Jpeg)
    } else {
        None
    }
}

/// Pixelmasse aus dem Kopf: PNG `IHDR`, JPEG der erste `SOFn`-Abschnitt. Ohne
/// Bildbibliothek, nur Lesen innerhalb der Grenzen (`get`), nie ein Index ausserhalb.
pub fn image_size(bytes: &[u8], kind: ImageKind) -> Option<(u32, u32)> {
    let be16 = |i: usize| -> Option<u32> {
        Some(u32::from(u16::from_be_bytes([
            *bytes.get(i)?,
            *bytes.get(i + 1)?,
        ])))
    };
    let be32 = |i: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?))
    };
    match kind {
        ImageKind::Png => {
            if bytes.get(12..16)? != b"IHDR" {
                return None;
            }
            Some((be32(16)?, be32(20)?))
        }
        ImageKind::Jpeg => {
            let mut i = 2;
            while i + 9 < bytes.len() {
                if bytes[i] != 0xFF {
                    return None;
                }
                let marker = bytes[i + 1];
                let len = be16(i + 2)? as usize;
                let sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
                if sof {
                    return Some((be16(i + 7)?, be16(i + 5)?));
                }
                i += 2 + len;
            }
            None
        }
    }
}

/// Prueft Logodaten (Art, Groesse, Kanten).
pub fn check_logo(bytes: &[u8]) -> Result<(ImageKind, u32, u32), String> {
    let kind = sniff(bytes).ok_or_else(|| format!("{CODE_LOGO}:format"))?;
    if bytes.len() as u64 > MAX_LOGO_BYTES {
        return Err(format!("{CODE_LOGO}:size"));
    }
    let (w, h) = image_size(bytes, kind).ok_or_else(|| format!("{CODE_LOGO}:format"))?;
    if w == 0 || h == 0 || w > MAX_LOGO_PX || h > MAX_LOGO_PX {
        return Err(format!("{CODE_LOGO}:dimensions"));
    }
    Ok((kind, w, h))
}

// ---------------------------------------------------------------------------
// Ablage
// ---------------------------------------------------------------------------

/// Die Profile unter einer Wurzel (`<App-Daten>/briefkoepfe`).
pub struct Store {
    root: PathBuf,
}

/// Ein Logo samt Art und Pixelmassen, fertig fuer Word und PDF.
#[derive(Clone, Debug)]
pub struct LogoData {
    pub bytes: Vec<u8>,
    pub kind: ImageKind,
    pub width_px: u32,
    pub height_px: u32,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err(format!("{CODE_INVALID}:id"));
        }
        Ok(self.root.join(id))
    }

    /// Mitgelieferte zuerst, dann eigene nach Name. Ein unlesbares Profil wird
    /// uebersprungen (und geloggt), nie geraten.
    pub fn list(&self) -> Vec<Letterhead> {
        let mut own: Vec<Letterhead> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let id = entry.file_name().to_string_lossy().to_string();
                if !valid_id(&id) || is_builtin(&id) {
                    return None;
                }
                match self.read_own(&id) {
                    Ok(lh) => Some(lh),
                    Err(e) => {
                        log::warn!("Briefkopf {id} nicht lesbar: {e}");
                        None
                    }
                }
            })
            .collect();
        own.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let mut all = builtins();
        all.extend(own);
        all
    }

    fn read_own(&self, id: &str) -> Result<Letterhead, String> {
        let path = self.dir(id)?.join(FILE_NAME);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{CODE_IO}: {e}"))?;
        let mut lh: Letterhead =
            serde_json::from_str(&text).map_err(|_| format!("{CODE_INVALID}:json"))?;
        lh.builtin = false;
        if lh.id != id {
            return Err(format!("{CODE_INVALID}:id"));
        }
        validate(&lh)?;
        Ok(lh)
    }

    /// Ein Profil; `letterhead_not_found`, wenn es fehlt (nie still "Neutral").
    pub fn get(&self, id: &str) -> Result<Letterhead, String> {
        if let Some(lh) = builtins().into_iter().find(|b| b.id == id) {
            return Ok(lh);
        }
        if !valid_id(id) || !self.dir(id)?.join(FILE_NAME).is_file() {
            return Err(CODE_NOT_FOUND.to_string());
        }
        self.read_own(id)
    }

    /// Speichert ein eigenes Profil (neu oder geaendert).
    pub fn save(&self, lh: &Letterhead) -> Result<(), String> {
        let _lock = profile_lock();
        self.save_locked(lh)
    }

    /// [`Self::save`] fuer Aufrufer, die [`PROFILE_LOCK`] schon halten.
    fn save_locked(&self, lh: &Letterhead) -> Result<(), String> {
        if is_builtin(&lh.id) {
            return Err(CODE_READ_ONLY.to_string());
        }
        let mut lh = lh.clone();
        lh.builtin = false;
        validate(&lh)?;
        let dir = self.dir(&lh.id)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{CODE_IO}: {e}"))?;
        let json = serde_json::to_string_pretty(&lh).map_err(|e| format!("{CODE_IO}: {e}"))?;
        write_atomic(&dir.join(FILE_NAME), json.as_bytes())
    }

    /// Entfernt ein eigenes Profil samt Logo.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        if is_builtin(id) {
            return Err(CODE_READ_ONLY.to_string());
        }
        let dir = self.dir(id)?;
        if !dir.is_dir() {
            return Err(CODE_NOT_FOUND.to_string());
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{CODE_IO}: {e}"))
    }

    /// Setzt das Logo eines eigenen Profils aus einer Datei (geprueft, kopiert) oder
    /// entfernt es (`None`). Liefert das geaenderte Profil.
    pub fn set_logo(&self, id: &str, source: Option<&Path>) -> Result<Letterhead, String> {
        if is_builtin(id) {
            return Err(CODE_READ_ONLY.to_string());
        }
        let _lock = profile_lock();
        let mut lh = self.get(id)?;
        let dir = self.dir(id)?;
        let previous_file = lh
            .logo
            .as_ref()
            .map(|l| l.datei.clone())
            .filter(|d| valid_logo_file(d));
        // Der Ersatz bekommt einen neuen, eindeutigen Namen: das bisherige Logo
        // wird nie ueberschrieben. Erst pruefen und schreiben, dann das Profil
        // speichern, dann aufraeumen.
        let keep = match source {
            None => {
                lh.logo = None;
                None
            }
            Some(path) => {
                let size = std::fs::metadata(path)
                    .map_err(|e| format!("{CODE_IO}: {e}"))?
                    .len();
                if size > MAX_LOGO_BYTES {
                    return Err(format!("{CODE_LOGO}:size"));
                }
                let bytes = std::fs::read(path).map_err(|e| format!("{CODE_IO}: {e}"))?;
                let (kind, _, _) = check_logo(&bytes)?;
                let name = format!("logo-{}.{}", ulid::Ulid::new(), kind.extension());
                write_atomic(&dir.join(&name), &bytes)?;
                let previous = lh.logo.take();
                lh.logo = Some(Logo {
                    datei: name.clone(),
                    breite_mm: previous.as_ref().map(|l| l.breite_mm).unwrap_or(30.0),
                    position: previous.map(|l| l.position).unwrap_or(LogoPosition::Rechts),
                });
                Some(name)
            }
        };
        if let Err(e) = self.save_locked(&lh) {
            if let Some(new) = &keep {
                let _ = std::fs::remove_file(dir.join(new));
            }
            return Err(e);
        }
        // Nur das vorher referenzierte Logo loeschen, und nur wenn das Profil
        // auf der Platte jetzt auf das neue zeigt -- ein paralleler Wechsel
        // hat sonst gewonnen, und dessen Logo bleibt stehen.
        if let Some(old) = previous_file.filter(|o| Some(o) != keep.as_ref()) {
            let now = self.get(id).ok().and_then(|l| l.logo).map(|l| l.datei);
            if now == keep {
                let _ = std::fs::remove_file(dir.join(old));
            }
        }
        Ok(lh)
    }

    /// Die Logodaten eines Profils (`None` ohne Logo). Ein defektes Logo ist ein Fehler.
    pub fn logo(&self, lh: &Letterhead) -> Result<Option<LogoData>, String> {
        let Some(logo) = &lh.logo else {
            return Ok(None);
        };
        let bytes = match logo.datei.as_str() {
            "builtin:wai" => WAI_LOGO.to_vec(),
            "builtin:ingenieurbuero" => IB_LOGO.to_vec(),
            d if valid_logo_file(d) => std::fs::read(self.dir(&lh.id)?.join(d))
                .map_err(|e| format!("{CODE_LOGO}:missing ({e})"))?,
            _ => return Err(format!("{CODE_INVALID}:logo")),
        };
        let (kind, width_px, height_px) = check_logo(&bytes)?;
        Ok(Some(LogoData {
            bytes,
            kind,
            width_px,
            height_px,
        }))
    }
}

/// Dateiname eines eigenen Logos: `logo.png`/`logo.jpg` (aeltere Profile) oder
/// `logo-<ulid>.png|jpg`. Nur ASCII-Buchstaben und Ziffern dazwischen -- kein
/// Pfad, kein `..`.
fn valid_logo_file(name: &str) -> bool {
    if matches!(name, "logo.png" | "logo.jpg") {
        return true;
    }
    let Some(stem) = name
        .strip_suffix(".png")
        .or_else(|| name.strip_suffix(".jpg"))
    else {
        return false;
    };
    stem.strip_prefix("logo-").is_some_and(|u| {
        (1..=40).contains(&u.len()) && u.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Prozessweite Sperre fuer jedes Schreiben eines Profils: Logo ersetzen,
/// Profil speichern und altes Logo loeschen laufen als ein Schritt, ein
/// paralleles `save` schreibt keinen alten Verweis dazwischen zurueck.
static PROFILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn profile_lock() -> std::sync::MutexGuard<'static, ()> {
    PROFILE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Ist das eine mitgelieferte Id?
pub fn is_builtin(id: &str) -> bool {
    builtins().iter().any(|b| b.id == id)
}

/// Schreibt ueber eine eindeutige Zwischendatei im selben Ordner
/// (`.<name>.<zufall>.part`) und Umbenennen: ein Abbruch laesst das Ziel
/// unberuehrt, parallele Schreiber kommen sich nicht in die Quere.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let io = |e: std::io::Error| format!("{CODE_IO}: {e}");
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut part = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".part")
        .tempfile_in(dir)
        .map_err(io)?;
    part.write_all(bytes).map_err(io)?;
    // Windows lehnt das Umbenennen ab, solange ein anderer Schreiber dasselbe
    // Ziel gerade ersetzt: kurz warten und erneut versuchen. Scheitert es
    // endgueltig, loescht `part` beim Verlassen die Zwischendatei.
    for _ in 0..10 {
        match part.persist(target) {
            Ok(_) => return Ok(()),
            Err(e) if e.error.kind() == std::io::ErrorKind::PermissionDenied => {
                part = e.file;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(io(e.error)),
        }
    }
    part.persist(target).map_err(|e| io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(DIR_NAME));
        (dir, store)
    }

    fn own(id: &str) -> Letterhead {
        Letterhead {
            id: id.into(),
            name: "Meine Firma".into(),
            builtin: false,
            logo: None,
            ..builtins().remove(0)
        }
    }

    #[test]
    fn parallel_atomic_writes_do_not_mix() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("brief.pdf");
        let writers: Vec<_> = (0..8u8)
            .map(|i| {
                let t = target.clone();
                std::thread::spawn(move || write_atomic(&t, &vec![i; 200_000]))
            })
            .collect();
        // Windows kann ein gleichzeitiges Umbenennen auf dasselbe Ziel ablehnen
        // ("Zugriff verweigert") -- dann sauber als Fehler, nie vermischt.
        let ok = writers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert!(ok >= 1);
        let got = std::fs::read(&target).unwrap();
        assert_eq!(got.len(), 200_000);
        assert!(got.iter().all(|b| *b == got[0]), "Inhalte vermischt");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "Zwischendateien liegen geblieben");
    }

    #[test]
    fn builtins_are_valid_and_neutral_is_the_default() {
        let all = builtins();
        assert_eq!(all[0].id, DEFAULT_ID);
        for lh in &all {
            validate(lh).unwrap_or_else(|e| panic!("{}: {e}", lh.id));
        }
        let (_d, s) = store();
        for lh in &all {
            let logo = s.logo(lh).unwrap();
            assert_eq!(logo.is_some(), lh.logo.is_some(), "{}", lh.id);
        }
        // Annahme 4: keine Rechtsangaben in mitgelieferten Fusszeilen.
        for lh in &all {
            for line in &lh.fusszeile.zeilen {
                assert!(!line.contains("IBAN") && !line.contains("USt"), "{line}");
            }
        }
    }

    #[test]
    fn validation_rejects_bad_ids_colors_and_logo_paths() {
        assert!(valid_id("meine-firma-2"));
        for id in ["", "A", "../x", "a/b", "-x", &"x".repeat(41)] {
            assert!(!valid_id(id), "{id}");
        }
        let mut lh = own("x");
        lh.farben.akzent = "gelb".into();
        assert_eq!(validate(&lh).unwrap_err(), "letterhead_invalid:akzent");
        let mut lh = own("x");
        lh.logo = Some(Logo {
            datei: "../../geheim.png".into(),
            breite_mm: 20.0,
            position: LogoPosition::Rechts,
        });
        assert_eq!(validate(&lh).unwrap_err(), "letterhead_invalid:logo");
        let mut lh = own("x");
        lh.logo = Some(Logo {
            datei: "builtin:wai".into(),
            breite_mm: 20.0,
            position: LogoPosition::Rechts,
        });
        assert!(
            validate(&lh).is_err(),
            "eigene Profile duerfen kein eingebautes Logo nennen"
        );
        assert_eq!(parse_color("#FFDD00"), Some((255, 221, 0)));
    }

    #[test]
    fn own_profiles_round_trip_and_builtins_are_read_only() {
        let (_d, s) = store();
        s.save(&own("meine-firma")).unwrap();
        let list = s.list();
        assert_eq!(list.len(), 4);
        assert_eq!(list[3].id, "meine-firma");
        assert!(!list[3].builtin);
        assert_eq!(s.get("meine-firma").unwrap().name, "Meine Firma");
        assert_eq!(s.save(&builtins()[1]).unwrap_err(), CODE_READ_ONLY);
        assert_eq!(s.delete("wai").unwrap_err(), CODE_READ_ONLY);
        assert_eq!(s.get("fehlt").unwrap_err(), CODE_NOT_FOUND);
        s.delete("meine-firma").unwrap();
        assert_eq!(s.list().len(), 3);
    }

    #[test]
    fn a_logo_is_checked_copied_and_replaced() {
        let (dir, s) = store();
        s.save(&own("f")).unwrap();
        let png = dir.path().join("in.png");
        std::fs::write(&png, WAI_LOGO).unwrap();
        let lh = s.set_logo("f", Some(&png)).unwrap();
        let first = lh.logo.as_ref().unwrap().datei.clone();
        assert!(
            first.starts_with("logo-") && first.ends_with(".png"),
            "{first}"
        );
        let data = s.logo(&lh).unwrap().unwrap();
        assert_eq!((data.width_px, data.height_px), (223, 169));

        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, b"<svg/>").unwrap();
        assert_eq!(
            s.set_logo("f", Some(&fake)).unwrap_err(),
            "letterhead_logo_invalid:format"
        );
        // Ein ungueltiger Ersatz laesst das bisherige Logo stehen.
        let kept = s.get("f").unwrap();
        assert_eq!(kept.logo.as_ref().unwrap().datei, first);
        assert!(s.logo(&kept).unwrap().is_some());
        // Ein gueltiger Ersatz bekommt einen neuen Namen; das alte Logo geht
        // erst, wenn das Profil auf das neue zeigt.
        let lh = s.set_logo("f", Some(&png)).unwrap();
        let second = lh.logo.as_ref().unwrap().datei.clone();
        assert_ne!(second, first);
        let pdir = dir.path().join(DIR_NAME).join("f");
        assert!(pdir.join(&second).is_file());
        assert!(!pdir.join(&first).exists());
        let lh = s.set_logo("f", None).unwrap();
        assert!(lh.logo.is_none());
        assert!(!pdir.join(&second).exists());
        assert!(valid_logo_file("logo.jpg"));
        assert!(!valid_logo_file("logo-../x.png"));
        assert!(!valid_logo_file("logo-.png"));
    }

    #[test]
    fn a_broken_profile_file_is_skipped_not_guessed() {
        let (_d, s) = store();
        let path = s.root.join("kaputt");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(FILE_NAME), "{nicht json").unwrap();
        assert_eq!(s.list().len(), 3);
        assert_eq!(s.get("kaputt").unwrap_err(), "letterhead_invalid:json");
    }
}
