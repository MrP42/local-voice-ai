//! Speicher der eigenen Assistenten-Profile (lva#556, Paket G2, Epic #554).
//!
//! Das Speicherformat gilt gemeinsam mit dem iPhone (`AssistantProfileStore.swift`) und steht in
//! `apps/shared/assistenten/README.md` (Abschnitt „Speicherformat“). Rust hält die **Dateien**, nicht
//! den Inhalt: Prüfen, Lesen und Kodieren der Profile gehört dem gemeinsamen TypeScript-Kern
//! (`src/components/voice-call/assistenten/`). Rust prüft nur
//!
//! * die Id-Regeln (`^[a-z0-9][a-z0-9-]{0,63}$`, keine Windows-Gerätenamen, vorbehalten sind `alles`,
//!   `aktiv` und `vorlage-…`), damit aus einer Id nie ein fremder Pfad wird,
//! * die Größe (256 KiB je Datei, 200 Dateien),
//! * die Art des Eintrags (nur reguläre Dateien werden gelesen oder verändert; Symlinks, Sockets, FIFOs
//!   und Verzeichnisse nie),
//! * das Verzeichnis selbst: ist `<App-Datenpfad>/assistenten` ein Link (Symlink, unter Windows jeder
//!   Reparse-Punkt, also auch eine Junction), führt keine Operation etwas aus (`unsafe_path`): `O_NOFOLLOW`
//!   schützt nur die letzte Pfadkomponente, ein verlinktes Verzeichnis würde sonst jeden Zugriff nach
//!   draußen lenken. Die Prüfung steht unter der Sperre vor jeder Operation (Rust bietet ohne Zusatzpaket keinen
//!   Zugriff relativ zu einem geöffneten Verzeichnis); ein Prozess desselben Benutzers, der das Verzeichnis
//!   zwischen Prüfung und Zugriff austauscht, liegt wie bei den Modellen außerhalb der Grenze, und
//! * dass eine bestehende Datei, die es überschreiben oder löschen soll, **byte-genau** dem Text
//!   entspricht, den TypeScript geprüft hat (`ersetzt`/`erwartet`, Vergleich-und-Tausch), zusätzlich
//!   ein JSON-Objekt mit `schema = "lva.assistent/1"` und `id` = Dateiname ist.
//!
//! Die Zusicherung „eine abgewiesene Datei wird nie überschrieben oder gelöscht“ steht auf zwei Beinen:
//! TypeScript bietet nur gültige eigene Profile zum Überschreiben und Löschen an, und Rust fasst nur
//! eine Datei an, deren Bytes der übergebene Text sind und die als eigenes Profil kenntlich ist
//! (beschädigt, neueres Schema, falsche Id im Inhalt: abgewiesen).
//!
//! Schreiben ist atomar: in eine Hilfsdatei (`.lva-….tmp`, endet nie auf `.json`) im selben
//! Verzeichnis, `fsync`, dann Umbenennen. Ein Abbruch hinterlässt höchstens eine Hilfsdatei, die der
//! Speicher ignoriert; die Zieldatei ist danach entweder der alte oder der neue Stand, nie ein halber.
//! Alle Schreib- und Leseschritte laufen unter einer gemeinsamen Sperre (ein Prozess, ein
//! Verzeichnis); zwei Programme am selben Verzeichnis sind nicht vorgesehen (README: „Ein Schreiber
//! je Verzeichnis“). Inhalte stehen nie im Log.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

/// Höchstens so viele Profil-Dateien (die ersten 200 Namen, nach Bytes sortiert, sind die Auswahl).
pub const MAX_DATEIEN: usize = 200;
/// Höchstens so groß ist eine Datei (Profil oder `aktiv.json`).
pub const MAX_BYTES: usize = 256 * 1024;
/// Die Datei der aktiven Auswahl; `aktiv` ist deshalb als Id vorbehalten.
pub const AKTIV_DATEI: &str = "aktiv.json";
/// Unterverzeichnis im App-Datenpfad: ein Verzeichnis je Gerät.
pub const VERZEICHNIS: &str = "assistenten";
/// Schema der Profile (nur zur Kennzeichnung eigener Dateien beim Überschreiben und Löschen).
const SCHEMA: &str = "lva.assistent/1";

static SPERRE: Mutex<()> = Mutex::new(());
static ZAEHLER: AtomicU64 = AtomicU64::new(0);

/// Eine Sperre pro Prozess; eine vergiftete Sperre (ein abgestürzter Befehl) gibt den Speicher nicht dauerhaft frei.
fn sperre() -> MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Warum eine Datei nicht als Text geliefert wird. Die Namen entsprechen den Fehlercodes der Profilprüfung
/// (`json`, `laenge`, `zu_viele`, `wert`); der Pfad ist immer `""`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Abweisung {
    /// Kein Text: Symlink, FIFO, Socket, Gerät, nicht lesbar oder kein gültiges UTF-8.
    Json,
    /// Größer als 256 KiB (nicht gelesen).
    Laenge,
    /// Jenseits der ersten 200 Namen (nicht gelesen).
    ZuViele,
    /// Nur für `aktiv.json`: belegt, also kein Auswahldokument.
    Wert,
}

/// Fehler der Befehle. Ein Fehler ändert nie etwas am Verzeichnis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AssistentenFehler {
    /// Die Id verletzt das Muster oder ist ein Windows-Gerätename.
    InvalidId,
    /// Vorbehaltene Id (`alles`, `aktiv`, `vorlage-…`): nicht zu speichern, zu löschen oder als Datei zu lesen.
    ReservedId,
    /// Es gibt die Datei (für den Speicher) nicht.
    NotFound,
    /// Der Text ist größer als 256 KiB.
    TooLarge,
    /// Schon 200 Dateien im Verzeichnis.
    TooMany,
    /// Unter dieser Id liegt etwas, das nicht der erwartete eigene Stand ist (abgewiesene Datei, fremde
    /// Änderung, jenseits der Auswahl, belegte `aktiv.json`). Es wird nichts verändert.
    Occupied,
    /// Schreiben oder Löschen ist fehlgeschlagen (Datenträger voll, gesperrt, kein Zugriff). Der alte
    /// Stand bleibt.
    WriteFailed,
    /// Das Verzeichnis ließ sich nicht lesen.
    ReadFailed,
    /// Das Verzeichnis der Assistenten ist ein Link (Symlink, unter Windows Junction oder anderer Reparse-Punkt):
    /// nichts wird gelesen, geschrieben oder gelöscht, auch nicht außerhalb.
    UnsafePath,
}

/// Eine Datei des Verzeichnisses: der Text (unverändert, nicht kodiert) oder der Grund, warum es keinen gibt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AssistentenDatei {
    /// Dateiname mit Endung (`eigen-abc.json`).
    pub name: String,
    /// Der Text der Datei, wenn sie gelesen wurde.
    pub inhalt: Option<String>,
    /// Der Grund, wenn nicht.
    pub abgewiesen: Option<Abweisung>,
}

/// Alles, was im Verzeichnis liegt, in einem Zug (unter der Sperre, also ein stimmiger Stand).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AssistentenListe {
    /// Die Dateien, nach Bytes des Namens sortiert; eine belegte `aktiv.json` (kein Verzeichnis) steht am Ende.
    pub dateien: Vec<AssistentenDatei>,
    /// Die Id in `aktiv.json`, wenn sie ein Auswahldokument ist (ob es das Profil gibt, prüft TypeScript).
    pub aktiv_id: Option<String>,
    /// `aktiv.json` ist belegt (kein Auswahldokument): `voice_assistant_active_set` meldet `occupied`.
    pub aktiv_belegt: bool,
}

/// Die aktive Auswahl allein.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AssistentenAktiv {
    pub id: Option<String>,
    pub belegt: bool,
}

// ---------------------------------------------------------------- Ids

/// Warum eine Id nicht als Dateiname taugt (Spiegel der Profilprüfung: `muster`, `wert`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdFehler {
    Muster,
    Wert,
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`, nur ASCII, ohne Zeilenumbruch am Ende.
fn id_passt(id: &str) -> bool {
    let b = id.as_bytes();
    if b.is_empty() || b.len() > 64 {
        return false;
    }
    let klein_oder_ziffer = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    klein_oder_ziffer(b[0]) && b[1..].iter().all(|&c| klein_oder_ziffer(c) || c == b'-')
}

/// Windows-Gerätenamen (`con`, `prn`, `aux`, `nul`, `com0`…`com9`, `lpt0`…`lpt9`): als Dateiname unbrauchbar.
fn ist_geraetename(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || ((id.starts_with("com") || id.starts_with("lpt"))
            && id.len() == 4
            && id.as_bytes()[3].is_ascii_digit())
}

/// Prüft die Id als Dateiname (Muster, Gerätename). Vorbehaltene Ids bestehen das Muster und gehören
/// nicht in diese Prüfung (siehe [`ist_vorbehalten`]).
pub fn pruefe_id(id: &str) -> Result<(), IdFehler> {
    if !id_passt(id) {
        return Err(IdFehler::Muster);
    }
    if ist_geraetename(id) {
        return Err(IdFehler::Wert);
    }
    Ok(())
}

/// Vorbehalten: `alles` (die Vorlage „Alles“), `aktiv` (die Datei der Auswahl) und alles mit `vorlage-`.
pub fn ist_vorbehalten(id: &str) -> bool {
    id == "alles" || id == "aktiv" || id.starts_with("vorlage-")
}

fn dateiname(id: &str) -> String {
    format!("{id}.json")
}

// ---------------------------------------------------------------- Dateien

/// `FILE_ATTRIBUTE_REPARSE_POINT` (Windows): Symlink, Junction und jeder andere Reparse-Punkt.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Trägt das Attributwort einer Windows-Datei das Merkmal „Reparse-Punkt“?
#[cfg_attr(not(windows), allow(dead_code))]
fn ist_reparse_punkt(attribute: u32) -> bool {
    attribute & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Ist der Eintrag ein Link auf etwas anderes: ein Symlink, unter Windows jeder Reparse-Punkt (auch eine Junction,
/// die `is_symlink` je nach Rust-Version nicht meldet)?
fn ist_verlinkt(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if ist_reparse_punkt(meta.file_attributes()) {
            return true;
        }
    }
    false
}

/// Das Verzeichnis der Assistenten darf kein Link sein: sonst folgten `read_dir`, `create_dir_all` und jeder
/// Dateizugriff (`O_NOFOLLOW` gilt nur für die letzte Komponente) dem Link aus dem Profilspeicher hinaus. Ein
/// fehlendes Verzeichnis ist in Ordnung (es ist leer und wird beim Schreiben als echtes Verzeichnis angelegt); jeder
/// andere Fehler wird den Zugriff selbst scheitern lassen.
fn wurzel_pruefen(verzeichnis: &Path) -> Result<(), AssistentenFehler> {
    match fs::symlink_metadata(verzeichnis) {
        Ok(meta) if ist_verlinkt(&meta) => Err(AssistentenFehler::UnsafePath),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Art {
    Fehlt,
    Regulaer,
    Verzeichnis,
    Anderes,
}

/// Art des Eintrags, ohne Links zu verfolgen. Ein Link (Symlink, unter Windows jeder Reparse-Punkt, auch bei einer
/// einzelnen Profildatei) und ein Fehler außer „fehlt“ gelten als „anderes“ (wird nie gelesen oder überschrieben).
fn art(pfad: &Path) -> Art {
    match fs::symlink_metadata(pfad) {
        Ok(m) if ist_verlinkt(&m) => Art::Anderes,
        Ok(m) if m.file_type().is_file() => Art::Regulaer,
        Ok(m) if m.file_type().is_dir() => Art::Verzeichnis,
        Ok(_) => Art::Anderes,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Art::Fehlt
        }
        Err(_) => Art::Anderes,
    }
}

/// Liest eine reguläre Datei bis 256 KiB als UTF-8. Alles andere ist eine Abweisung, und es wird nicht
/// gelesen: nicht reguläre Einträge nie (ein FIFO würde das Öffnen blockieren: unter Unix wird mit
/// `O_NONBLOCK | O_NOFOLLOW` geöffnet und danach die Art der geöffneten Datei geprüft), zu große vor dem Lesen.
fn lies(pfad: &Path) -> Result<String, Abweisung> {
    if art(pfad) != Art::Regulaer {
        return Err(Abweisung::Json);
    }
    let mut optionen = OpenOptions::new();
    optionen.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        optionen.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let datei = optionen.open(pfad).map_err(|_| Abweisung::Json)?;
    let meta = datei.metadata().map_err(|_| Abweisung::Json)?;
    if !meta.is_file() {
        return Err(Abweisung::Json);
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(Abweisung::Laenge);
    }
    // `take` begrenzt das Lesen auch dann, wenn die Datei zwischen Prüfung und Lesen wächst.
    let mut puffer = Vec::with_capacity(meta.len().min(MAX_BYTES as u64) as usize);
    datei
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut puffer)
        .map_err(|_| Abweisung::Json)?;
    if puffer.len() > MAX_BYTES {
        return Err(Abweisung::Laenge);
    }
    String::from_utf8(puffer).map_err(|_| Abweisung::Json)
}

/// Kenntlich als eigenes Profil dieser Id: ein JSON-Objekt mit `schema = "lva.assistent/1"` und `id` gleich
/// dem Dateinamen ohne Endung. Das ist **nicht** die Profilprüfung (die macht TypeScript), sondern die
/// Sperre davor: beschädigte, neuere und fremde Dateien bestehen sie nicht und bleiben unberührt.
/// Strenger als TypeScript ist zulässig (dann bleibt die Datei eher stehen, als dass sie überschrieben wird).
fn ist_eigenes_profil(id: &str, text: &str) -> bool {
    let Ok(serde_json::Value::Object(objekt)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return false;
    };
    objekt.get("schema").and_then(|v| v.as_str()) == Some(SCHEMA)
        && objekt.get("id").and_then(|v| v.as_str()) == Some(id)
}

enum AktivZustand {
    Fehlt,
    Auswahl(String),
    Belegt { verzeichnis: bool },
}

/// So viele Namen probiert das Anlegen einer Hilfsdatei, wenn der gewählte schon vergeben ist.
const HILFS_VERSUCHE: usize = 16;

/// Legt eine neue Hilfsdatei `verzeichnis/<vorsatz>-<Prozessnummer>-<Zähler>.tmp` an und liefert Pfad und Datei.
/// Der Name ist vergeben, wenn dort schon eine Datei liegt (der Rest eines früheren Laufs mit derselben Prozessnummer,
/// eine fremde Datei): sie gehört nicht diesem Aufruf und wird **nie** angefasst, gelöscht oder überschrieben (`create_new`);
/// es kommt der nächste Zähler, höchstens `HILFS_VERSUCHE` mal, dann `AlreadyExists`. Jeder andere Fehler kommt sofort
/// zurück. Wer die Datei bekommt, hat sie selbst angelegt und räumt sie bei einem Fehler auf; sonst gibt es nichts aufzuräumen.
fn hilfsdatei_anlegen(
    verzeichnis: &Path,
    vorsatz: &str,
    zaehler: &AtomicU64,
) -> io::Result<(PathBuf, File)> {
    for _ in 0..HILFS_VERSUCHE {
        let pfad = verzeichnis.join(format!(
            "{vorsatz}-{}-{}.tmp",
            std::process::id(),
            zaehler.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&pfad) {
            Ok(datei) => return Ok((pfad, datei)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// Schreibt `bytes` atomar nach `verzeichnis/name`: Hilfsdatei im selben Verzeichnis, `fsync`, Umbenennen.
/// Bei jedem Fehler wird die selbst angelegte Hilfsdatei entfernt und das Ziel nicht angefasst. `schreiber` ist der
/// Schreibschritt (im Test ein Schreiber, der „Datenträger voll“ oder einen Abbruch mitten im Schreiben nachstellt).
fn schreibe_atomar_mit(
    verzeichnis: &Path,
    name: &str,
    bytes: &[u8],
    schreiber: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), AssistentenFehler> {
    schreibe_atomar_mit_zaehler(verzeichnis, name, bytes, schreiber, &ZAEHLER)
}

/// Wie `schreibe_atomar_mit`, mit eigenem Zähler für die Namen der Hilfsdateien (im Test für Namensstöße).
fn schreibe_atomar_mit_zaehler(
    verzeichnis: &Path,
    name: &str,
    bytes: &[u8],
    schreiber: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    zaehler: &AtomicU64,
) -> Result<(), AssistentenFehler> {
    wurzel_pruefen(verzeichnis)?;
    fs::create_dir_all(verzeichnis).map_err(|_| AssistentenFehler::WriteFailed)?;
    // Noch einmal nach dem Anlegen: erst jetzt steht fest, dass die Hilfsdatei in einem echten Verzeichnis entsteht.
    wurzel_pruefen(verzeichnis)?;
    // Scheitert schon das Anlegen (auch an einem vergebenen Namen), gibt es nichts Eigenes zu entfernen.
    let (hilfsdatei, mut datei) = hilfsdatei_anlegen(verzeichnis, ".lva", zaehler)
        .map_err(|_| AssistentenFehler::WriteFailed)?;
    let ziel = verzeichnis.join(name);
    let ergebnis = (|| -> io::Result<()> {
        schreiber(&mut datei, bytes)?;
        datei.sync_all()?;
        drop(datei);
        fs::rename(&hilfsdatei, &ziel)
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&hilfsdatei);
        return Err(AssistentenFehler::WriteFailed);
    }
    #[cfg(unix)]
    if let Ok(verz) = File::open(verzeichnis) {
        let _ = verz.sync_all();
    }
    Ok(())
}

// ---------------------------------------------------------------- Export

/// Schreibt `inhalt` (die Bytes eines Profils, wie TypeScript sie kodiert) in die Datei, die die Person im
/// Speichern-Dialog gewählt hat (G2 Teil 2: Export). Atomar wie der Speicher: Hilfsdatei im selben Verzeichnis,
/// `fsync`, Umbenennen; bei jedem Fehler bleibt das Ziel, wie es war, und die selbst angelegte Hilfsdatei wird entfernt
/// (eine schon vorhandene gleichen Namens nie, siehe `hilfsdatei_anlegen`). Das Ziel muss
/// ein absoluter Pfad auf `.json` sein und darf kein Verzeichnis und kein Link sein; der Text höchstens 256 KiB.
/// Fehler sind `TooLarge` und `WriteFailed` (alles Übrige: kein brauchbares Ziel, Datenträger voll, gesperrt).
/// `schreiber` ist der Schreibschritt (im Test ein Schreiber, der „Datenträger voll“ nachstellt).
fn exportiere_mit(
    pfad: &Path,
    inhalt: &str,
    schreiber: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), AssistentenFehler> {
    exportiere_mit_zaehler(pfad, inhalt, schreiber, &ZAEHLER)
}

/// Wie `exportiere_mit`, mit eigenem Zähler für die Namen der Hilfsdateien (im Test für Namensstöße). Eine Hilfsdatei, die
/// es schon gibt, gehört nicht dem Export: sie bleibt unberührt, der Export nimmt den nächsten Namen (`hilfsdatei_anlegen`).
fn exportiere_mit_zaehler(
    pfad: &Path,
    inhalt: &str,
    schreiber: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    zaehler: &AtomicU64,
) -> Result<(), AssistentenFehler> {
    if inhalt.len() > MAX_BYTES {
        return Err(AssistentenFehler::TooLarge);
    }
    let endet_auf_json = pfad
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let Some(verzeichnis) = pfad.parent() else {
        return Err(AssistentenFehler::WriteFailed);
    };
    if pfad.file_name().is_none() || !pfad.is_absolute() || !endet_auf_json {
        return Err(AssistentenFehler::WriteFailed);
    }
    // Ein vorhandenes Ziel muss eine gewöhnliche Datei sein: ein Verzeichnis, einen Link oder ein Gerät überschreibt der
    // Export nie (das Umbenennen träte sonst an dessen Stelle).
    match fs::symlink_metadata(pfad) {
        Ok(m) if !m.file_type().is_file() || ist_verlinkt(&m) => {
            return Err(AssistentenFehler::WriteFailed)
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(AssistentenFehler::WriteFailed),
    }
    // Scheitert schon das Anlegen (auch an einem vergebenen Namen), gibt es nichts Eigenes zu entfernen.
    let (hilfsdatei, mut datei) = hilfsdatei_anlegen(verzeichnis, ".lva-export", zaehler)
        .map_err(|_| AssistentenFehler::WriteFailed)?;
    let ergebnis = (|| -> io::Result<()> {
        schreiber(&mut datei, inhalt.as_bytes())?;
        datei.sync_all()?;
        drop(datei);
        fs::rename(&hilfsdatei, pfad)
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&hilfsdatei);
        return Err(AssistentenFehler::WriteFailed);
    }
    Ok(())
}

// ---------------------------------------------------------------- Speicher

/// Der Speicher eines Verzeichnisses. Die Befehle benutzen `<App-Datenpfad>/assistenten`.
#[derive(Debug, Clone)]
pub struct Speicher {
    verzeichnis: PathBuf,
}

impl Speicher {
    pub fn neu(verzeichnis: PathBuf) -> Self {
        Self { verzeichnis }
    }

    fn pfad(&self, name: &str) -> PathBuf {
        self.verzeichnis.join(name)
    }

    /// Vor jeder Operation (unter der Sperre): das Verzeichnis ist kein Link, sonst `UnsafePath`.
    fn wurzel(&self) -> Result<(), AssistentenFehler> {
        wurzel_pruefen(&self.verzeichnis)
    }

    /// Dateinamen der Profile (endet auf `.json`, ohne `aktiv.json`, ohne Verzeichnisse), nach Bytes sortiert.
    /// Symlinks und andere nicht reguläre Einträge zählen mit (sie erscheinen als abgewiesen), Hilfsdateien
    /// und fremde Endungen nicht. Ein fehlendes Verzeichnis ist leer; jeder andere Lesefehler ist einer.
    fn dateinamen(&self) -> Result<Vec<String>, AssistentenFehler> {
        let eintraege = match fs::read_dir(&self.verzeichnis) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(AssistentenFehler::ReadFailed),
        };
        let mut namen = Vec::new();
        for eintrag in eintraege {
            let eintrag = eintrag.map_err(|_| AssistentenFehler::ReadFailed)?;
            let name = eintrag.file_name().to_string_lossy().into_owned();
            // Die Endung zählt ohne Rücksicht auf Groß- und Kleinschreibung: `A.JSON` ist unter Windows dieselbe Datei
            // wie `a.json` und erscheint hier als Kandidat, den der Leser (Dateiname = Id) abweist, statt still zu fehlen.
            let klein = name.to_ascii_lowercase();
            if klein.len() <= ".json".len() || !klein.ends_with(".json") || klein == AKTIV_DATEI {
                continue;
            }
            if matches!(art(&eintrag.path()), Art::Regulaer | Art::Anderes) {
                namen.push(name);
            }
        }
        namen.sort_unstable();
        Ok(namen)
    }

    /// Ob die Datei zur Auswahl gehört: zu den ersten 200 Namen. Bei höchstens 200 Namen immer.
    fn in_auswahl(&self, name: &str) -> Result<bool, AssistentenFehler> {
        let namen = self.dateinamen()?;
        Ok(namen.len() <= MAX_DATEIEN || namen[..MAX_DATEIEN].iter().any(|n| n == name))
    }

    fn aktiv_zustand(&self) -> AktivZustand {
        let pfad = self.pfad(AKTIV_DATEI);
        match art(&pfad) {
            Art::Fehlt => AktivZustand::Fehlt,
            Art::Verzeichnis => AktivZustand::Belegt { verzeichnis: true },
            Art::Anderes => AktivZustand::Belegt { verzeichnis: false },
            Art::Regulaer => {
                let belegt = AktivZustand::Belegt { verzeichnis: false };
                let Ok(text) = lies(&pfad) else {
                    return belegt;
                };
                // Ein Objekt mit genau einem Mitglied `id` (Zeichenkette); doppelte Schlüssel fasst der
                // Leser zusammen, der letzte gilt (wie im TypeScript-Leser).
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Object(o)) if o.len() == 1 => match o.get("id") {
                        Some(serde_json::Value::String(id)) => AktivZustand::Auswahl(id.clone()),
                        _ => belegt,
                    },
                    _ => belegt,
                }
            }
        }
    }

    /// Alle Dateien mit ihrem Text und den Stand der aktiven Auswahl, in einem Zug. Über 200 Dateien hinaus
    /// wird nichts gelesen. Eine belegte `aktiv.json` steht am Ende und zählt nicht zur Grenze von 200.
    pub fn liste(&self) -> Result<AssistentenListe, AssistentenFehler> {
        let _sperre = sperre();
        self.wurzel()?;
        let namen = self.dateinamen()?;
        let mut dateien = Vec::with_capacity(namen.len() + 1);
        for (index, name) in namen.into_iter().enumerate() {
            let (inhalt, abgewiesen) = if index >= MAX_DATEIEN {
                (None, Some(Abweisung::ZuViele))
            } else {
                match lies(&self.pfad(&name)) {
                    Ok(text) => (Some(text), None),
                    Err(grund) => (None, Some(grund)),
                }
            };
            dateien.push(AssistentenDatei {
                name,
                inhalt,
                abgewiesen,
            });
        }
        let (aktiv_id, aktiv_belegt) = match self.aktiv_zustand() {
            AktivZustand::Fehlt => (None, false),
            AktivZustand::Auswahl(id) => (Some(id), false),
            AktivZustand::Belegt { verzeichnis } => {
                if !verzeichnis {
                    dateien.push(AssistentenDatei {
                        name: AKTIV_DATEI.to_string(),
                        inhalt: None,
                        abgewiesen: Some(Abweisung::Wert),
                    });
                }
                (None, true)
            }
        };
        Ok(AssistentenListe {
            dateien,
            aktiv_id,
            aktiv_belegt,
        })
    }

    /// Eine Datei nach Id. `None`: es gibt sie für den Speicher nicht (vorbehaltene oder ungültige Id,
    /// fehlt, Verzeichnis, jenseits der ersten 200 Namen). Ein Symlink oder anderes Nicht-Reguläres, eine zu
    /// große Datei und ein Text, der kein UTF-8 ist, kommen als Abweisung zurück.
    pub fn lesen(&self, id: &str) -> Result<Option<AssistentenDatei>, AssistentenFehler> {
        if ist_vorbehalten(id) || pruefe_id(id).is_err() {
            return Ok(None);
        }
        let _sperre = sperre();
        self.wurzel()?;
        let name = dateiname(id);
        let pfad = self.pfad(&name);
        if matches!(art(&pfad), Art::Fehlt | Art::Verzeichnis) || !self.in_auswahl(&name)? {
            return Ok(None);
        }
        let (inhalt, abgewiesen) = match lies(&pfad) {
            Ok(text) => (Some(text), None),
            Err(grund) => (None, Some(grund)),
        };
        Ok(Some(AssistentenDatei {
            name,
            inhalt,
            abgewiesen,
        }))
    }

    /// Speichert `inhalt` (die Bytes, die TypeScript geliefert hat) als `<id>.json`, atomar.
    ///
    /// * `ersetzt = None`: eine **neue** Datei; gibt es die Datei schon (gleich welcher Art), ist es `Occupied`.
    ///   Ab 200 Dateien `TooMany`.
    /// * `ersetzt = Some(text)`: überschreibt die vorhandene Datei, aber nur, wenn sie eine reguläre Datei
    ///   innerhalb der Auswahl ist, als eigenes Profil dieser Id kenntlich ist und genau `text` enthält
    ///   (so, wie TypeScript sie geprüft hat). Alles andere ist `Occupied`; fehlt sie inzwischen: `NotFound`.
    pub fn speichern(
        &self,
        id: &str,
        inhalt: &str,
        ersetzt: Option<&str>,
    ) -> Result<(), AssistentenFehler> {
        pruefe_id(id).map_err(|_| AssistentenFehler::InvalidId)?;
        if ist_vorbehalten(id) {
            return Err(AssistentenFehler::ReservedId);
        }
        if inhalt.len() > MAX_BYTES {
            return Err(AssistentenFehler::TooLarge);
        }
        let _sperre = sperre();
        self.wurzel()?;
        let name = dateiname(id);
        let ziel = self.pfad(&name);
        match art(&ziel) {
            Art::Fehlt => {
                if ersetzt.is_some() {
                    return Err(AssistentenFehler::NotFound);
                }
                if self.dateinamen()?.len() >= MAX_DATEIEN {
                    return Err(AssistentenFehler::TooMany);
                }
            }
            Art::Regulaer => {
                let Some(erwartet) = ersetzt else {
                    return Err(AssistentenFehler::Occupied);
                };
                self.pruefe_eigener_stand(id, &name, &ziel, erwartet)?;
            }
            Art::Verzeichnis | Art::Anderes => return Err(AssistentenFehler::Occupied),
        }
        schreibe_atomar_mit(&self.verzeichnis, &name, inhalt.as_bytes(), &|d, b| {
            d.write_all(b)
        })
    }

    /// Löscht `<id>.json`, aber nur, wenn sie eine reguläre Datei innerhalb der Auswahl ist, als eigenes Profil
    /// dieser Id kenntlich ist und genau `erwartet` enthält. Vorbehaltene Id: `ReservedId`. Ungültige Id, fehlende
    /// Datei, Verzeichnis, Symlink oder anderes Nicht-Reguläres: `NotFound`. Alles andere: `Occupied`.
    /// Die aktive Auswahl bleibt, wie sie ist (der Rückfall auf `alles` greift beim Lesen).
    pub fn loeschen(&self, id: &str, erwartet: &str) -> Result<(), AssistentenFehler> {
        if ist_vorbehalten(id) {
            return Err(AssistentenFehler::ReservedId);
        }
        if pruefe_id(id).is_err() {
            return Err(AssistentenFehler::NotFound);
        }
        let _sperre = sperre();
        self.wurzel()?;
        let name = dateiname(id);
        let ziel = self.pfad(&name);
        if art(&ziel) != Art::Regulaer {
            return Err(AssistentenFehler::NotFound);
        }
        self.pruefe_eigener_stand(id, &name, &ziel, erwartet)?;
        fs::remove_file(&ziel).map_err(|_| AssistentenFehler::WriteFailed)
    }

    /// Die Datei ist ein gültiger eigener Stand: innerhalb der Auswahl, kenntlich als eigenes Profil und
    /// byte-gleich `erwartet`; sonst `Occupied`.
    fn pruefe_eigener_stand(
        &self,
        id: &str,
        name: &str,
        pfad: &Path,
        erwartet: &str,
    ) -> Result<(), AssistentenFehler> {
        if !self.in_auswahl(name)? {
            return Err(AssistentenFehler::Occupied);
        }
        match lies(pfad) {
            Ok(text) if text == erwartet && ist_eigenes_profil(id, &text) => Ok(()),
            _ => Err(AssistentenFehler::Occupied),
        }
    }

    /// Die aktive Auswahl allein.
    pub fn aktiv(&self) -> Result<AssistentenAktiv, AssistentenFehler> {
        let _sperre = sperre();
        self.wurzel()?;
        // Ein nicht lesbares Verzeichnis ist ein Fehler, kein „fehlt“: sonst sähe es aus wie „Alles“.
        self.dateinamen()?;
        Ok(match self.aktiv_zustand() {
            AktivZustand::Fehlt => AssistentenAktiv {
                id: None,
                belegt: false,
            },
            AktivZustand::Auswahl(id) => AssistentenAktiv {
                id: Some(id),
                belegt: false,
            },
            AktivZustand::Belegt { .. } => AssistentenAktiv {
                id: None,
                belegt: true,
            },
        })
    }

    /// Wählt `id` als aktiv: schreibt nur `aktiv.json` (`{"id": "<id>"}`), nie ein Profil, und nur, wenn sie fehlt
    /// oder ein Auswahldokument ist (sonst `Occupied`). `aktiv` ist `ReservedId`. Ein Eigenes muss als reguläre
    /// Datei innerhalb der Auswahl liegen und als eigenes Profil kenntlich sein (sonst `NotFound`). Die Vorlagen
    /// (`alles`, `vorlage-…`) liegen nicht im Verzeichnis; dass es die Vorlage gibt, prüft TypeScript.
    pub fn waehlen(&self, id: &str) -> Result<(), AssistentenFehler> {
        pruefe_id(id).map_err(|_| AssistentenFehler::InvalidId)?;
        if id == "aktiv" {
            return Err(AssistentenFehler::ReservedId);
        }
        let _sperre = sperre();
        self.wurzel()?;
        if !ist_vorbehalten(id) {
            let name = dateiname(id);
            let pfad = self.pfad(&name);
            let vorhanden = art(&pfad) == Art::Regulaer
                && self.in_auswahl(&name)?
                && lies(&pfad).is_ok_and(|text| ist_eigenes_profil(id, &text));
            if !vorhanden {
                return Err(AssistentenFehler::NotFound);
            }
        }
        if matches!(self.aktiv_zustand(), AktivZustand::Belegt { .. }) {
            return Err(AssistentenFehler::Occupied);
        }
        let text = format!("{{\n  \"id\": \"{id}\"\n}}\n");
        schreibe_atomar_mit(&self.verzeichnis, AKTIV_DATEI, text.as_bytes(), &|d, b| {
            d.write_all(b)
        })
    }
}

// ---------------------------------------------------------------- Befehle

fn speicher(app: &AppHandle) -> Result<Speicher, AssistentenFehler> {
    crate::portable::app_data_dir(app)
        .map(|daten| Speicher::neu(daten.join(VERZEICHNIS)))
        .map_err(|_| AssistentenFehler::ReadFailed)
}

/// Führt `f` auf einem Blockier-Thread gegen den Speicher aus.
async fn blockierend<T, F>(app: &AppHandle, f: F) -> Result<T, AssistentenFehler>
where
    T: Send + 'static,
    F: FnOnce(&Speicher) -> Result<T, AssistentenFehler> + Send + 'static,
{
    let speicher = speicher(app)?;
    tauri::async_runtime::spawn_blocking(move || f(&speicher))
        .await
        .map_err(|_| AssistentenFehler::ReadFailed)?
}

/// Alle Dateien der eigenen Assistenten mit Text und aktiver Auswahl in einem Zug.
#[tauri::command]
#[specta::specta]
pub async fn voice_assistants_list(app: AppHandle) -> Result<AssistentenListe, AssistentenFehler> {
    blockierend(&app, |s| s.liste()).await
}

/// Eine Datei nach Id (`None`, wenn es sie für den Speicher nicht gibt).
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_read(
    app: AppHandle,
    id: String,
) -> Result<Option<AssistentenDatei>, AssistentenFehler> {
    blockierend(&app, move |s| s.lesen(&id)).await
}

/// Speichert den Text eines eigenen Assistenten atomar. `ersetzt`: der Text, den die Oberfläche geprüft hat
/// und der überschrieben werden soll (`null` = neue Datei).
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_save(
    app: AppHandle,
    id: String,
    inhalt: String,
    ersetzt: Option<String>,
) -> Result<(), AssistentenFehler> {
    blockierend(&app, move |s| s.speichern(&id, &inhalt, ersetzt.as_deref())).await
}

/// Löscht einen eigenen Assistenten, wenn die Datei noch genau `erwartet` enthält.
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_delete(
    app: AppHandle,
    id: String,
    erwartet: String,
) -> Result<(), AssistentenFehler> {
    blockierend(&app, move |s| s.loeschen(&id, &erwartet)).await
}

/// Schreibt einen Assistenten als Datei an den Ort, den die Person im Speichern-Dialog gewählt hat (Export). Die
/// Bytes liefert die Oberfläche (`kodiereProfil`); es wird nichts im Verzeichnis der Assistenten verändert.
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_export(path: String, inhalt: String) -> Result<(), AssistentenFehler> {
    tauri::async_runtime::spawn_blocking(move || {
        exportiere_mit(Path::new(&path), &inhalt, &|d, b| d.write_all(b))
    })
    .await
    .map_err(|_| AssistentenFehler::WriteFailed)?
}

/// Die aktive Auswahl (`aktiv.json`).
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_active_get(
    app: AppHandle,
) -> Result<AssistentenAktiv, AssistentenFehler> {
    blockierend(&app, |s| s.aktiv()).await
}

/// Wählt einen Assistenten als aktiv (nur `aktiv.json` wird geschrieben).
#[tauri::command]
#[specta::specta]
pub async fn voice_assistant_active_set(
    app: AppHandle,
    id: String,
) -> Result<(), AssistentenFehler> {
    blockierend(&app, move |s| s.waehlen(&id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ------------------------------------------------------------ Hilfen

    fn profil(id: &str) -> String {
        format!("{{\n  \"id\": \"{id}\",\n  \"name\": \"Test\",\n  \"schema\": \"{SCHEMA}\"\n}}\n")
    }

    fn werkstatt() -> (TempDir, Speicher) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Speicher::neu(tmp.path().join(VERZEICHNIS));
        (tmp, s)
    }

    fn lege(s: &Speicher, name: &str, inhalt: &[u8]) {
        fs::create_dir_all(&s.verzeichnis).unwrap();
        fs::write(s.pfad(name), inhalt).unwrap();
    }

    fn lege_profil(s: &Speicher, id: &str) {
        lege(s, &dateiname(id), profil(id).as_bytes());
    }

    fn bytes(s: &Speicher, name: &str) -> Vec<u8> {
        fs::read(s.pfad(name)).unwrap()
    }

    fn hilfsdateien(s: &Speicher) -> Vec<String> {
        fs::read_dir(&s.verzeichnis)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".tmp"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn datei<'a>(l: &'a AssistentenListe, name: &str) -> &'a AssistentenDatei {
        l.dateien.iter().find(|d| d.name == name).unwrap()
    }

    // ------------------------------------------------------------ Prüfstände (ids)

    #[derive(Deserialize)]
    struct IdFall {
        id: String,
        erwartet: String,
        vorbehalten: bool,
    }

    #[derive(Deserialize)]
    struct Pruefstaende {
        ids: Vec<IdFall>,
    }

    /// Dieselbe Datei wie Swift und TypeScript (`pruefstaende.json`, Abschnitt `ids`): Ergebnis der Id-Prüfung
    /// (`ok`/`muster`/`wert`) und ob die Id vorbehalten ist.
    #[test]
    fn pruefstaende_ids() {
        let text = include_str!("../../../../shared/assistenten/pruefstaende.json");
        let alle: Pruefstaende = serde_json::from_str(text).unwrap();
        assert!(
            alle.ids.len() >= 70,
            "Prüfstand zu klein: {}",
            alle.ids.len()
        );
        for fall in &alle.ids {
            let ist = match pruefe_id(&fall.id) {
                Ok(()) => "ok",
                Err(IdFehler::Muster) => "muster",
                Err(IdFehler::Wert) => "wert",
            };
            assert_eq!(ist, fall.erwartet, "Id {:?}", fall.id);
            assert_eq!(
                ist_vorbehalten(&fall.id),
                fall.vorbehalten,
                "Id {:?}",
                fall.id
            );
        }
    }

    // ------------------------------------------------------------ Grundfunktionen

    #[test]
    fn leeres_oder_fehlendes_verzeichnis() {
        let (_t, s) = werkstatt();
        let l = s.liste().unwrap();
        assert!(l.dateien.is_empty());
        assert_eq!(l.aktiv_id, None);
        assert!(!l.aktiv_belegt);
        assert_eq!(s.lesen("eigen-a").unwrap(), None);
        assert_eq!(
            s.aktiv().unwrap(),
            AssistentenAktiv {
                id: None,
                belegt: false
            }
        );
    }

    #[test]
    fn speichern_legt_verzeichnis_an_und_liefert_bytes_unveraendert() {
        let (_t, s) = werkstatt();
        // Rust kodiert nichts um: CRLF, Umlaute und fehlender Zeilenumbruch am Ende bleiben, wie sie kamen.
        let text =
            "{\r\n  \"id\": \"eigen-a\", \"name\": \"Größe über\",\"schema\": \"lva.assistent/1\"}";
        s.speichern("eigen-a", text, None).unwrap();
        assert_eq!(bytes(&s, "eigen-a.json"), text.as_bytes());
        let l = s.liste().unwrap();
        assert_eq!(l.dateien.len(), 1);
        assert_eq!(l.dateien[0].inhalt.as_deref(), Some(text));
        assert_eq!(l.dateien[0].abgewiesen, None);
        let gelesen = s.lesen("eigen-a").unwrap().unwrap();
        assert_eq!(gelesen.inhalt.as_deref(), Some(text));
        assert!(hilfsdateien(&s).is_empty());
    }

    #[test]
    fn sortierung_nach_bytes() {
        let (_t, s) = werkstatt();
        for id in ["b", "a", "a-b", "a1", "0z", "a-1"] {
            lege_profil(&s, id);
        }
        let namen: Vec<String> = s
            .liste()
            .unwrap()
            .dateien
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(
            namen,
            ["0z.json", "a-1.json", "a-b.json", "a.json", "a1.json", "b.json"]
        );
    }

    #[test]
    fn fremde_dateien_und_hilfsdateien_werden_ignoriert() {
        let (_t, s) = werkstatt();
        lege_profil(&s, "eigen-a");
        for name in [
            "notiz.txt",
            "x.json.bak",
            ".json",
            ".lva-1-1.tmp",
            "eigen-b.json.tmp",
        ] {
            lege(&s, name, b"{}");
        }
        fs::create_dir(s.pfad("ordner.json")).unwrap();
        let l = s.liste().unwrap();
        assert_eq!(l.dateien.len(), 1);
        assert_eq!(l.dateien[0].name, "eigen-a.json");
    }

    #[test]
    fn verzeichnispfad_ist_eine_datei_meldet_fehler() {
        let tmp = tempfile::tempdir().unwrap();
        let pfad = tmp.path().join("assistenten");
        fs::write(&pfad, b"x").unwrap();
        let s = Speicher::neu(pfad.clone());
        assert_eq!(s.liste().unwrap_err(), AssistentenFehler::ReadFailed);
        assert_eq!(s.aktiv().unwrap_err(), AssistentenFehler::ReadFailed);
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), None)
                .unwrap_err(),
            AssistentenFehler::ReadFailed
        );
        assert_eq!(fs::read(&pfad).unwrap(), b"x");
    }

    // ------------------------------------------------------------ Ids und vorbehaltene Namen

    #[test]
    fn ungueltige_ids_werden_vor_dem_dateisystem_abgewiesen() {
        let (t, s) = werkstatt();
        let lang = "a".repeat(65);
        for id in [
            "",
            "../x",
            "a/b",
            "a\\b",
            "A",
            "a b",
            "a\n",
            "a\0b",
            "ä",
            "-a",
            "con",
            "lpt1",
            lang.as_str(),
        ] {
            assert_eq!(
                s.speichern(id, "{}", None),
                Err(AssistentenFehler::InvalidId),
                "{id:?}"
            );
            assert_eq!(
                s.loeschen(id, "{}"),
                Err(AssistentenFehler::NotFound),
                "{id:?}"
            );
            assert_eq!(s.waehlen(id), Err(AssistentenFehler::InvalidId), "{id:?}");
            assert_eq!(s.lesen(id).unwrap(), None, "{id:?}");
        }
        // Nichts ist entstanden, auch nicht neben dem Verzeichnis.
        assert!(!s.verzeichnis.exists());
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
    }

    #[test]
    fn id_mit_64_zeichen_geht() {
        let (_t, s) = werkstatt();
        let id = format!("a{}", "b".repeat(63));
        s.speichern(&id, &profil(&id), None).unwrap();
        assert!(s.pfad(&dateiname(&id)).is_file());
    }

    #[test]
    fn vorbehaltene_ids_lassen_sich_nicht_speichern_oder_loeschen() {
        let (_t, s) = werkstatt();
        for id in [
            "alles",
            "aktiv",
            "vorlage-neu",
            "vorlage-",
            "vorlage-buchgespraech",
        ] {
            assert_eq!(
                s.speichern(id, "{}", None),
                Err(AssistentenFehler::ReservedId),
                "{id}"
            );
            assert_eq!(
                s.loeschen(id, "{}"),
                Err(AssistentenFehler::ReservedId),
                "{id}"
            );
            assert_eq!(s.lesen(id).unwrap(), None, "{id}");
        }
        assert!(!s.verzeichnis.exists());
        // Eine vorbehaltene Datei, die jemand von Hand hineinlegt, bleibt unberührt, auch beim Löschen mit ihren Bytes.
        let text = profil("vorlage-x");
        lege(&s, "vorlage-x.json", text.as_bytes());
        assert_eq!(
            s.loeschen("vorlage-x", &text),
            Err(AssistentenFehler::ReservedId)
        );
        assert_eq!(
            s.speichern("vorlage-x", "{}", Some(&text)),
            Err(AssistentenFehler::ReservedId)
        );
        assert_eq!(bytes(&s, "vorlage-x.json"), text.as_bytes());
    }

    #[test]
    fn vorlagen_lassen_sich_waehlen_aktiv_nicht() {
        let (_t, s) = werkstatt();
        assert_eq!(s.waehlen("aktiv"), Err(AssistentenFehler::ReservedId));
        assert!(!s.pfad(AKTIV_DATEI).exists());
        s.waehlen("alles").unwrap();
        assert_eq!(s.aktiv().unwrap().id.as_deref(), Some("alles"));
        s.waehlen("vorlage-projektstand").unwrap();
        assert_eq!(
            s.aktiv().unwrap().id.as_deref(),
            Some("vorlage-projektstand")
        );
        // Es entstand nur aktiv.json, kein Profil.
        assert_eq!(fs::read_dir(&s.verzeichnis).unwrap().count(), 1);
        assert_eq!(
            bytes(&s, AKTIV_DATEI),
            b"{\n  \"id\": \"vorlage-projektstand\"\n}\n"
        );
    }

    // ------------------------------------------------------------ Größe

    #[test]
    fn groesse_256_kib_geht_ein_byte_mehr_nicht() {
        let (_t, s) = werkstatt();
        let kopf = profil("eigen-a");
        let fuell = |n: usize| format!("{}{}", kopf, " ".repeat(n - kopf.len()));
        let genau = fuell(MAX_BYTES);
        s.speichern("eigen-a", &genau, None).unwrap();
        let l = s.liste().unwrap();
        assert_eq!(l.dateien[0].inhalt.as_deref(), Some(genau.as_str()));
        let zu_gross = fuell(MAX_BYTES + 1);
        assert_eq!(
            s.speichern("eigen-b", &zu_gross, None),
            Err(AssistentenFehler::TooLarge)
        );
        assert!(!s.pfad("eigen-b.json").exists());
        // Überschreiben mit zu großem Text ändert die vorhandene Datei nicht.
        assert_eq!(
            s.speichern("eigen-a", &zu_gross, Some(&genau)),
            Err(AssistentenFehler::TooLarge)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), genau.as_bytes());
    }

    #[test]
    fn zu_grosse_datei_wird_nicht_gelesen_und_nie_veraendert() {
        let (_t, s) = werkstatt();
        let gross = vec![b' '; MAX_BYTES + 1];
        lege(&s, "eigen-a.json", &gross);
        let l = s.liste().unwrap();
        assert_eq!(
            datei(&l, "eigen-a.json").abgewiesen,
            Some(Abweisung::Laenge)
        );
        assert_eq!(datei(&l, "eigen-a.json").inhalt, None);
        assert_eq!(
            s.lesen("eigen-a").unwrap().unwrap().abgewiesen,
            Some(Abweisung::Laenge)
        );
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), None),
            Err(AssistentenFehler::Occupied)
        );
        let text = String::from_utf8(gross.clone()).unwrap();
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), Some(&text)),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(
            s.loeschen("eigen-a", &text),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(s.waehlen("eigen-a"), Err(AssistentenFehler::NotFound));
        assert_eq!(bytes(&s, "eigen-a.json"), gross);
    }

    // ------------------------------------------------------------ Überschreiben und Löschen (Vergleich-und-Tausch)

    #[test]
    fn neu_anlegen_fasst_vorhandenes_nie_an() {
        let (_t, s) = werkstatt();
        s.speichern("eigen-a", &profil("eigen-a"), None).unwrap();
        let alt = bytes(&s, "eigen-a.json");
        assert_eq!(
            s.speichern("eigen-a", "{\"neu\": true}", None),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), alt);
    }

    #[test]
    fn ueberschreiben_nur_mit_dem_geprueften_alten_text() {
        let (_t, s) = werkstatt();
        let alt = profil("eigen-a");
        s.speichern("eigen-a", &alt, None).unwrap();
        let neu = alt.replace("Test", "Neu");
        // Falscher alter Text: jemand hat die Datei inzwischen geändert.
        assert_eq!(
            s.speichern("eigen-a", &neu, Some(&alt.replace("Test", "Fremd"))),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), alt.as_bytes());
        s.speichern("eigen-a", &neu, Some(&alt)).unwrap();
        assert_eq!(bytes(&s, "eigen-a.json"), neu.as_bytes());
        // Ein zweiter Versuch mit dem jetzt veralteten Text scheitert.
        assert_eq!(
            s.speichern("eigen-a", &alt, Some(&alt)),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), neu.as_bytes());
        assert!(hilfsdateien(&s).is_empty());
    }

    #[test]
    fn ueberschreiben_einer_fehlenden_datei_ist_not_found() {
        let (_t, s) = werkstatt();
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), Some("x")),
            Err(AssistentenFehler::NotFound)
        );
        assert!(!s.pfad("eigen-a.json").exists());
    }

    #[test]
    fn loeschen_nur_mit_dem_geprueften_text() {
        let (_t, s) = werkstatt();
        let text = profil("eigen-a");
        s.speichern("eigen-a", &text, None).unwrap();
        assert_eq!(
            s.loeschen("eigen-a", &text.replace("Test", "X")),
            Err(AssistentenFehler::Occupied)
        );
        assert!(s.pfad("eigen-a.json").exists());
        s.loeschen("eigen-a", &text).unwrap();
        assert!(!s.pfad("eigen-a.json").exists());
        assert_eq!(
            s.loeschen("eigen-a", &text),
            Err(AssistentenFehler::NotFound)
        );
    }

    #[test]
    fn beschaedigte_neuere_und_fremde_dateien_bleiben_auch_mit_eigenen_bytes_stehen() {
        let (_t, s) = werkstatt();
        let faelle: Vec<(&str, String)> = vec![
            ("kaputt-a", "{ kein json".to_string()),
            (
                "neuer-b",
                profil("neuer-b").replace("lva.assistent/1", "lva.assistent/2"),
            ),
            (
                "fremd-c",
                "{\"id\": \"fremd-c\", \"name\": \"x\"}".to_string(),
            ),
            ("falsch-d", profil("anderer")),
            ("liste-e", "[1, 2]".to_string()),
            ("leer-f", String::new()),
        ];
        for (id, text) in &faelle {
            lege(&s, &dateiname(id), text.as_bytes());
        }
        for (id, text) in &faelle {
            // Auch wenn die Oberfläche genau diese Bytes übergibt: Rust fasst sie nicht an.
            assert_eq!(
                s.loeschen(id, text),
                Err(AssistentenFehler::Occupied),
                "{id}"
            );
            assert_eq!(
                s.speichern(id, &profil(id), Some(text)),
                Err(AssistentenFehler::Occupied),
                "{id}"
            );
            assert_eq!(
                s.speichern(id, &profil(id), None),
                Err(AssistentenFehler::Occupied),
                "{id}"
            );
            assert_eq!(s.waehlen(id), Err(AssistentenFehler::NotFound), "{id}");
            assert_eq!(bytes(&s, &dateiname(id)), text.as_bytes(), "{id}");
        }
    }

    #[test]
    fn nicht_utf8_ist_abgewiesen_und_unberuehrt() {
        let (_t, s) = werkstatt();
        let roh = [0x7b, 0xff, 0xfe, 0x7d];
        lege(&s, "eigen-a.json", &roh);
        let l = s.liste().unwrap();
        assert_eq!(datei(&l, "eigen-a.json").abgewiesen, Some(Abweisung::Json));
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), None),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), roh);
    }

    #[test]
    fn bom_bleibt_im_text_der_leser_weist_es_ab() {
        let (_t, s) = werkstatt();
        let mit_bom = format!("\u{feff}{}", profil("eigen-a"));
        lege(&s, "eigen-a.json", mit_bom.as_bytes());
        let l = s.liste().unwrap();
        // Rust liefert den Text unverändert; die Abweisung ist Sache des strengen Lesers in TypeScript.
        assert_eq!(
            datei(&l, "eigen-a.json").inhalt.as_deref(),
            Some(mit_bom.as_str())
        );
        // Als eigenes Profil kenntlich ist die Datei für Rust nicht (kein gültiges JSON): unberührt.
        assert_eq!(
            s.loeschen("eigen-a", &mit_bom),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(bytes(&s, "eigen-a.json"), mit_bom.as_bytes());
    }

    // ------------------------------------------------------------ Art des Eintrags

    #[cfg(unix)]
    #[test]
    fn symlink_socket_und_verzeichnis_werden_nie_gelesen_oder_veraendert() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        let (t, s) = werkstatt();
        fs::create_dir_all(&s.verzeichnis).unwrap();
        // Ziel außerhalb: ein gültiges Profil, auf das ein Symlink zeigt.
        let draussen = t.path().join("draussen.json");
        let text = profil("link-a");
        fs::write(&draussen, &text).unwrap();
        symlink(&draussen, s.pfad("link-a.json")).unwrap();
        symlink(t.path().join("gibt-es-nicht"), s.pfad("haengt-b.json")).unwrap();
        let _socket = UnixListener::bind(s.pfad("sock-c.json")).unwrap();
        fs::create_dir(s.pfad("ordner-d.json")).unwrap();

        let l = s.liste().unwrap();
        for name in ["link-a.json", "haengt-b.json", "sock-c.json"] {
            assert_eq!(datei(&l, name).abgewiesen, Some(Abweisung::Json), "{name}");
            assert_eq!(datei(&l, name).inhalt, None, "{name}");
        }
        assert!(
            l.dateien.iter().all(|d| d.name != "ordner-d.json"),
            "Verzeichnisse werden ignoriert"
        );

        for id in ["link-a", "haengt-b", "sock-c"] {
            assert_eq!(
                s.lesen(id).unwrap().unwrap().abgewiesen,
                Some(Abweisung::Json),
                "{id}"
            );
            assert_eq!(
                s.speichern(id, &profil(id), None),
                Err(AssistentenFehler::Occupied),
                "{id}"
            );
            assert_eq!(
                s.speichern(id, &profil(id), Some(&text)),
                Err(AssistentenFehler::Occupied),
                "{id}"
            );
            assert_eq!(
                s.loeschen(id, &text),
                Err(AssistentenFehler::NotFound),
                "{id}"
            );
            assert_eq!(s.waehlen(id), Err(AssistentenFehler::NotFound), "{id}");
        }
        // Verzeichnis: nicht vorhanden für den Speicher, nichts wird überschrieben oder entfernt.
        assert_eq!(s.lesen("ordner-d").unwrap(), None);
        assert_eq!(
            s.speichern("ordner-d", &profil("ordner-d"), None),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(
            s.loeschen("ordner-d", "{}"),
            Err(AssistentenFehler::NotFound)
        );
        assert_eq!(s.waehlen("ordner-d"), Err(AssistentenFehler::NotFound));
        // Nichts wurde verändert: der Symlink zeigt weiter auf das Ziel, das Ziel ist unverändert.
        assert!(fs::symlink_metadata(s.pfad("link-a.json"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&draussen).unwrap(), text.as_bytes());
        assert!(s.pfad("ordner-d.json").is_dir());
    }

    /// R2: das Merkmal „Reparse-Punkt“ wird aus dem Attributwort gelesen (auf jedem System prüfbar; unter Windows
    /// entscheidet es zusätzlich zu `is_symlink` über Links, etwa bei Reparse-Punkten ohne Symlink-Kennung).
    #[test]
    fn reparse_merkmal_kommt_aus_dem_attributwort() {
        assert!(ist_reparse_punkt(0x400));
        assert!(ist_reparse_punkt(0x400 | 0x20));
        assert!(ist_reparse_punkt(0x400 | 0x10));
        assert!(!ist_reparse_punkt(0));
        assert!(!ist_reparse_punkt(0x20)); // ARCHIVE
        assert!(!ist_reparse_punkt(0x10)); // DIRECTORY
        assert!(!ist_reparse_punkt(0x2000_0000 | 0x800)); // andere Merkmale (z. B. komprimiert)
    }

    /// R2: Eine einzelne Profildatei, die ein Reparse-Punkt ist (Datei-Symlink), wird nie gelesen, überschrieben,
    /// gelöscht oder gewählt, und das Ziel bleibt unverändert. Das Anlegen eines Datei-Symlinks braucht unter Windows
    /// Rechte (Entwicklermodus); fehlen sie, meldet der Test das und endet, statt etwas Falsches zu belegen.
    #[cfg(windows)]
    #[test]
    fn datei_symlink_als_profil_wird_unter_windows_abgewiesen() {
        let (t, s) = werkstatt();
        fs::create_dir_all(&s.verzeichnis).unwrap();
        let draussen = t.path().join("draussen.json");
        let text = profil("link-a");
        fs::write(&draussen, &text).unwrap();
        if let Err(e) = std::os::windows::fs::symlink_file(&draussen, s.pfad("link-a.json")) {
            eprintln!("Datei-Symlink nicht angelegt (Rechte fehlen?): {e}");
            return;
        }
        assert!(ist_verlinkt(
            &fs::symlink_metadata(s.pfad("link-a.json")).unwrap()
        ));
        let l = s.liste().unwrap();
        assert_eq!(datei(&l, "link-a.json").abgewiesen, Some(Abweisung::Json));
        assert_eq!(datei(&l, "link-a.json").inhalt, None);
        assert_eq!(
            s.lesen("link-a").unwrap().unwrap().abgewiesen,
            Some(Abweisung::Json)
        );
        assert_eq!(
            s.speichern("link-a", &profil("link-a"), Some(&text)),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(
            s.loeschen("link-a", &text),
            Err(AssistentenFehler::NotFound)
        );
        assert_eq!(s.waehlen("link-a"), Err(AssistentenFehler::NotFound));
        assert_eq!(fs::read(&draussen).unwrap(), text.as_bytes());
        assert!(ist_verlinkt(
            &fs::symlink_metadata(s.pfad("link-a.json")).unwrap()
        ));
    }

    // ------------------------------------------------------------ Verlinktes Verzeichnis

    /// Alle Einträge eines Verzeichnisses mit ihren Bytes (Verzeichnisse als `None`), sortiert: ein Vergleichsstand.
    fn schnappschuss(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut alle: Vec<(String, Option<Vec<u8>>)> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let inhalt = if e.path().is_file() {
                    Some(fs::read(e.path()).unwrap())
                } else {
                    None
                };
                (e.file_name().to_string_lossy().into_owned(), inhalt)
            })
            .collect();
        alle.sort();
        alle
    }

    /// Verlinkt `link` auf das Verzeichnis `ziel`: unter Unix ein Symlink, unter Windows eine Junction (`mklink /J`
    /// braucht keine besonderen Rechte). Das Anlegen muss gelingen, sonst scheitert der Test laut.
    fn verlinke_verzeichnis(link: &Path, ziel: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(ziel, link).unwrap();
        #[cfg(windows)]
        {
            let ausgabe = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(ziel)
                .output()
                .unwrap();
            assert!(
                ausgabe.status.success(),
                "Junction nicht angelegt: {}{}",
                String::from_utf8_lossy(&ausgabe.stdout),
                String::from_utf8_lossy(&ausgabe.stderr)
            );
        }
    }

    /// B1 (Codex R1): Ist `<App-Datenpfad>/assistenten` ein Link auf ein fremdes Verzeichnis (Unix: Symlink,
    /// Windows: Junction), wird alles abgewiesen (Lesen und jede Veränderung), und das fremde Verzeichnis bleibt
    /// Byte für Byte, wie es war (auch keine Hilfsdatei).
    #[cfg(any(unix, windows))]
    #[test]
    fn verlinktes_verzeichnis_wird_nie_gelesen_oder_veraendert() {
        let tmp = tempfile::tempdir().unwrap();
        let fremd = tmp.path().join("fremd");
        fs::create_dir_all(&fremd).unwrap();
        let text = profil("eigen-a");
        fs::write(fremd.join("eigen-a.json"), &text).unwrap();
        fs::write(fremd.join(AKTIV_DATEI), b"{\"id\": \"eigen-a\"}").unwrap();
        fs::write(fremd.join("notiz.txt"), b"fremd").unwrap();
        let wurzel = tmp.path().join(VERZEICHNIS);
        verlinke_verzeichnis(&wurzel, &fremd);
        assert!(
            ist_verlinkt(&fs::symlink_metadata(&wurzel).unwrap()),
            "der Link wird als Link erkannt"
        );
        let s = Speicher::neu(wurzel.clone());
        let vorher = schnappschuss(&fremd);

        let verweigert = AssistentenFehler::UnsafePath;
        let nein = Err(verweigert);
        assert_eq!(s.liste().unwrap_err(), verweigert);
        assert_eq!(s.lesen("eigen-a").unwrap_err(), verweigert);
        assert_eq!(s.aktiv().unwrap_err(), verweigert);
        assert_eq!(s.waehlen("eigen-a"), nein);
        assert_eq!(s.waehlen("alles"), nein);
        assert_eq!(s.speichern("eigen-b", &profil("eigen-b"), None), nein);
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), Some(&text)),
            nein
        );
        assert_eq!(s.loeschen("eigen-a", &text), nein);
        assert_eq!(
            schreibe_atomar_mit(&wurzel, "eigen-c.json", b"{}", &|d, b| d.write_all(b)),
            nein
        );
        assert_eq!(
            schreibe_atomar_mit(&wurzel, AKTIV_DATEI, b"{}", &|d, b| d.write_all(b)),
            nein
        );
        assert_eq!(
            schnappschuss(&fremd),
            vorher,
            "fremdes Verzeichnis unberührt"
        );
        assert!(
            ist_verlinkt(&fs::symlink_metadata(&wurzel).unwrap()),
            "der Link bleibt, wie er war"
        );
        // Ungültige und vorbehaltene Ids erreichen das Dateisystem ohnehin nie.
        assert_eq!(s.lesen("alles"), Ok(None));
        assert_eq!(s.lesen("Ungültig"), Ok(None));
    }

    /// Ein Link ins Leere (Ziel fehlt): auch er wird nicht benutzt, und das Ziel wird nicht angelegt.
    #[cfg(unix)]
    #[test]
    fn verlinktes_verzeichnis_ins_leere_legt_das_ziel_nicht_an() {
        let tmp = tempfile::tempdir().unwrap();
        let ziel = tmp.path().join("gibt-es-nicht");
        let wurzel = tmp.path().join(VERZEICHNIS);
        std::os::unix::fs::symlink(&ziel, &wurzel).unwrap();
        let s = Speicher::neu(wurzel);
        assert_eq!(s.liste().unwrap_err(), AssistentenFehler::UnsafePath);
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), None),
            Err(AssistentenFehler::UnsafePath)
        );
        assert_eq!(s.waehlen("alles"), Err(AssistentenFehler::UnsafePath));
        assert!(!ziel.exists(), "das Ziel des Links wurde nicht angelegt");
    }

    /// Ein Link, der erst nach der ersten Prüfung entsteht (das Verzeichnis fehlte, jemand legt einen Link an), wird
    /// nach dem Anlegen noch einmal erkannt: die Hilfsdatei entsteht nie im fremden Verzeichnis.
    #[cfg(unix)]
    #[test]
    fn link_statt_fehlendem_verzeichnis_beim_schreiben_wird_erkannt() {
        let tmp = tempfile::tempdir().unwrap();
        let fremd = tmp.path().join("fremd");
        fs::create_dir_all(&fremd).unwrap();
        let wurzel = tmp.path().join(VERZEICHNIS);
        let s = Speicher::neu(wurzel.clone());
        assert_eq!(
            s.liste().unwrap().dateien.len(),
            0,
            "fehlt: leer, kein Fehler"
        );
        std::os::unix::fs::symlink(&fremd, &wurzel).unwrap();
        assert_eq!(
            s.speichern("eigen-a", &profil("eigen-a"), None),
            Err(AssistentenFehler::UnsafePath)
        );
        assert!(schnappschuss(&fremd).is_empty());
    }

    /// Ein echtes Verzeichnis, dessen Elternpfad über einen Link läuft, ist kein Link (der App-Datenpfad liegt unter
    /// der Hoheit des Benutzers, etwa auf einem anderen Laufwerk): es wird benutzt.
    #[cfg(any(unix, windows))]
    #[test]
    fn verlinkter_elternpfad_ist_erlaubt_nur_das_verzeichnis_selbst_zaehlt() {
        let tmp = tempfile::tempdir().unwrap();
        let echt = tmp.path().join("echt");
        fs::create_dir_all(&echt).unwrap();
        let eltern = tmp.path().join("daten");
        verlinke_verzeichnis(&eltern, &echt);
        let s = Speicher::neu(eltern.join(VERZEICHNIS));
        s.speichern("eigen-a", &profil("eigen-a"), None).unwrap();
        assert_eq!(s.liste().unwrap().dateien.len(), 1);
        assert!(echt.join(VERZEICHNIS).join("eigen-a.json").is_file());
    }

    #[test]
    fn endung_gross_und_klein_wird_als_kandidat_gelistet_aktiv_json_nie() {
        let (_t, s) = werkstatt();
        lege(&s, "eigen.JSON", profil("eigen").as_bytes());
        lege(&s, "Mix.json", profil("mix").as_bytes());
        lege(&s, "notiz.txt", b"x");
        let l = s.liste().unwrap();
        let namen: Vec<&str> = l.dateien.iter().map(|d| d.name.as_str()).collect();
        // Auf einem Dateisystem, das Groß- und Kleinschreibung unterscheidet, sind es zwei Kandidaten; wo nicht
        // (Windows, macOS), steht jede Datei einmal unter dem Namen, den das Dateisystem meldet.
        assert!(namen.contains(&"eigen.JSON"), "{namen:?}");
        assert!(namen.contains(&"Mix.json"), "{namen:?}");
        assert!(!namen.contains(&"notiz.txt"));
        assert_eq!(bytes(&s, "Mix.json"), profil("mix").as_bytes());
    }

    // ------------------------------------------------------------ 200 Dateien

    fn fuelle(s: &Speicher, anzahl: usize) -> Vec<String> {
        let ids: Vec<String> = (0..anzahl).map(|i| format!("e{i:03}")).collect();
        for id in &ids {
            lege_profil(s, id);
        }
        ids
    }

    #[test]
    fn grenze_200_neuanlage_ueberschreiben_und_nachruecken() {
        let (_t, s) = werkstatt();
        let ids = fuelle(&s, 200);
        // Hilfsdatei, Fremdes, Verzeichnis und aktiv.json zählen nicht.
        lege(&s, ".lva-1-1.tmp", b"x");
        lege(&s, "readme.txt", b"x");
        fs::create_dir(s.pfad("ordner.json")).unwrap();
        s.waehlen("e000").unwrap();
        assert_eq!(s.liste().unwrap().dateien.len(), 200);
        // Die 201. Neuanlage ist TooMany, gleich welcher Name (auch einer, der vorn einzusortieren wäre).
        assert_eq!(
            s.speichern("e999", &profil("e999"), None),
            Err(AssistentenFehler::TooMany)
        );
        assert_eq!(
            s.speichern("a-neu", &profil("a-neu"), None),
            Err(AssistentenFehler::TooMany)
        );
        assert!(!s.pfad("e999.json").exists() && !s.pfad("a-neu.json").exists());
        // Ein vorhandenes Profil innerhalb der Auswahl lässt sich überschreiben.
        let alt = profil("e100");
        let neu = alt.replace("Test", "Neu");
        s.speichern("e100", &neu, Some(&alt)).unwrap();
        // Löschen gibt Platz frei.
        s.loeschen("e150", &profil("e150")).unwrap();
        s.speichern("e999", &profil("e999"), None).unwrap();
        assert_eq!(s.liste().unwrap().dateien.len(), 200);
        assert_eq!(ids.len(), 200);
    }

    #[test]
    fn jenseits_der_200_ist_nichts_abrufbar_und_nichts_veraenderbar() {
        let (_t, s) = werkstatt();
        fuelle(&s, 201);
        s.waehlen("e000").unwrap();
        let l = s.liste().unwrap();
        let (innen, aussen): (Vec<_>, Vec<_>) = l
            .dateien
            .iter()
            .partition(|d| d.abgewiesen != Some(Abweisung::ZuViele));
        assert_eq!(innen.len(), 200);
        assert_eq!(aussen.len(), 1);
        assert_eq!(aussen[0].name, "e200.json");
        assert_eq!(
            aussen[0].inhalt, None,
            "jenseits der Auswahl wird nicht gelesen"
        );
        // Abruf, Auswählen, Speichern, Löschen: für den Speicher nicht vorhanden bzw. belegt, nichts ändert sich.
        let text = profil("e200");
        assert_eq!(s.lesen("e200").unwrap(), None);
        assert_eq!(s.waehlen("e200"), Err(AssistentenFehler::NotFound));
        assert_eq!(
            s.speichern("e200", &text.replace("Test", "X"), Some(&text)),
            Err(AssistentenFehler::Occupied)
        );
        assert_eq!(s.loeschen("e200", &text), Err(AssistentenFehler::Occupied));
        assert_eq!(bytes(&s, "e200.json"), text.as_bytes());
        // Wird eine Datei der Auswahl gelöscht, rückt e200 nach und ist wieder lesbar und überschreibbar.
        s.loeschen("e010", &profil("e010")).unwrap();
        let l = s.liste().unwrap();
        assert!(l.dateien.iter().all(|d| d.abgewiesen.is_none()));
        assert_eq!(
            datei(&l, "e200.json").inhalt.as_deref(),
            Some(text.as_str())
        );
        s.speichern("e200", &text.replace("Test", "X"), Some(&text))
            .unwrap();
    }

    #[test]
    fn aktiv_auf_ein_profil_jenseits_der_200_zeigt_auf_nichts_lesbares() {
        let (_t, s) = werkstatt();
        fuelle(&s, 200);
        s.waehlen("e199").unwrap();
        // Ein Name, der vorn einsortiert, schiebt e199 aus der Auswahl (nach dem Auswählen von außen angelegt).
        lege_profil(&s, "a000");
        let l = s.liste().unwrap();
        assert_eq!(l.aktiv_id.as_deref(), Some("e199"));
        assert_eq!(datei(&l, "e199.json").abgewiesen, Some(Abweisung::ZuViele));
        assert_eq!(s.lesen("e199").unwrap(), None);
    }

    // ------------------------------------------------------------ aktiv.json

    fn aktiv_belegt_pruefen(s: &Speicher, erwartet_in_liste: bool) {
        let a = s.aktiv().unwrap();
        assert!(a.belegt && a.id.is_none());
        let l = s.liste().unwrap();
        assert!(l.aktiv_belegt);
        let eintrag = l.dateien.iter().find(|d| d.name == AKTIV_DATEI);
        assert_eq!(eintrag.is_some(), erwartet_in_liste);
        if let Some(e) = eintrag {
            assert_eq!(e.abgewiesen, Some(Abweisung::Wert));
            assert_eq!(e.inhalt, None);
        }
        lege_profil(s, "eigen-x");
        assert_eq!(s.waehlen("eigen-x"), Err(AssistentenFehler::Occupied));
        assert_eq!(s.waehlen("alles"), Err(AssistentenFehler::Occupied));
        // Auch bei wiederholtem Aufruf: nichts wird geschrieben.
        assert_eq!(s.waehlen("alles"), Err(AssistentenFehler::Occupied));
    }

    #[test]
    fn aktiv_auswahldokumente() {
        let (_t, s) = werkstatt();
        for (roh, id) in [
            ("{\"id\":\"eigen-x\"}", "eigen-x"),
            ("  {\n \"id\" : \"eigen-x\"\n}\r\n\t", "eigen-x"),
            ("{\"id\":\"a\",\"id\":\"b\"}", "b"),
            ("{\"id\": \"\\u0061lles\"}", "alles"),
            ("{\"id\":\"\"}", ""),
        ] {
            lege(&s, AKTIV_DATEI, roh.as_bytes());
            let a = s.aktiv().unwrap();
            assert_eq!(a.id.as_deref(), Some(id), "{roh}");
            assert!(!a.belegt, "{roh}");
            assert!(s.liste().unwrap().dateien.is_empty(), "{roh}");
        }
    }

    #[test]
    fn aktiv_belegt_nichts_wird_geschrieben() {
        let voll_profil = profil("alt");
        let gross = " ".repeat(MAX_BYTES + 1);
        let faelle: Vec<(&str, Vec<u8>)> = vec![
            ("vollstaendiges Profil", voll_profil.into_bytes()),
            ("weiteres Mitglied", br#"{"id":"alles","x":1}"#.to_vec()),
            ("falscher Typ", br#"{"id":1}"#.to_vec()),
            ("ohne id", br#"{"name":"alles"}"#.to_vec()),
            ("Liste", br#"["alles"]"#.to_vec()),
            ("Zeichenkette", br#""alles""#.to_vec()),
            ("kaputt", b"{\"id\":".to_vec()),
            ("leer", Vec::new()),
            ("Müll nach dem Wert", br#"{"id":"alles"} x"#.to_vec()),
            ("BOM", b"\xef\xbb\xbf{\"id\":\"alles\"}".to_vec()),
            ("kein UTF-8", vec![0x7b, 0xff, 0x7d]),
            ("zu groß", gross.into_bytes()),
        ];
        for (name, roh) in faelle {
            let (_t, s) = werkstatt();
            lege(&s, AKTIV_DATEI, &roh);
            aktiv_belegt_pruefen(&s, true);
            assert_eq!(bytes(&s, AKTIV_DATEI), roh, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn aktiv_als_symlink_socket_oder_verzeichnis_ist_belegt() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        // Symlink (auch auf ein gültiges Auswahldokument): belegt, nichts wird geschrieben.
        let (t, s) = werkstatt();
        fs::create_dir_all(&s.verzeichnis).unwrap();
        let ziel = t.path().join("ziel.json");
        fs::write(&ziel, "{\"id\":\"alles\"}").unwrap();
        symlink(&ziel, s.pfad(AKTIV_DATEI)).unwrap();
        aktiv_belegt_pruefen(&s, true);
        assert_eq!(fs::read(&ziel).unwrap(), b"{\"id\":\"alles\"}");
        assert!(fs::symlink_metadata(s.pfad(AKTIV_DATEI))
            .unwrap()
            .file_type()
            .is_symlink());
        // Hängender Symlink.
        let (t2, s2) = werkstatt();
        fs::create_dir_all(&s2.verzeichnis).unwrap();
        symlink(t2.path().join("nirgends"), s2.pfad(AKTIV_DATEI)).unwrap();
        aktiv_belegt_pruefen(&s2, true);
        // Socket.
        let (_t3, s3) = werkstatt();
        fs::create_dir_all(&s3.verzeichnis).unwrap();
        let _socket = UnixListener::bind(s3.pfad(AKTIV_DATEI)).unwrap();
        aktiv_belegt_pruefen(&s3, true);
        // Verzeichnis: wird wie jedes Verzeichnis nicht aufgelistet, ist aber belegt.
        let (_t4, s4) = werkstatt();
        fs::create_dir_all(s4.pfad(AKTIV_DATEI)).unwrap();
        aktiv_belegt_pruefen(&s4, false);
        assert!(s4.pfad(AKTIV_DATEI).is_dir());
    }

    #[test]
    fn aktiv_auswaehlen_schreibt_nur_aktiv_json_und_ersetzt_ein_auswahldokument() {
        let (_t, s) = werkstatt();
        lege_profil(&s, "eigen-a");
        lege_profil(&s, "eigen-b");
        let vorher = (bytes(&s, "eigen-a.json"), bytes(&s, "eigen-b.json"));
        s.waehlen("eigen-a").unwrap();
        assert_eq!(bytes(&s, AKTIV_DATEI), b"{\n  \"id\": \"eigen-a\"\n}\n");
        s.waehlen("eigen-b").unwrap();
        assert_eq!(s.aktiv().unwrap().id.as_deref(), Some("eigen-b"));
        s.waehlen("alles").unwrap();
        assert_eq!(s.aktiv().unwrap().id.as_deref(), Some("alles"));
        assert_eq!(
            (bytes(&s, "eigen-a.json"), bytes(&s, "eigen-b.json")),
            vorher
        );
        assert!(hilfsdateien(&s).is_empty());
        // Ein Eigenes, das es nicht gibt, ist NotFound und ändert aktiv.json nicht.
        assert_eq!(s.waehlen("eigen-z"), Err(AssistentenFehler::NotFound));
        assert_eq!(s.aktiv().unwrap().id.as_deref(), Some("alles"));
    }

    #[test]
    fn loeschen_des_aktiven_profils_laesst_aktiv_json_stehen() {
        let (_t, s) = werkstatt();
        let text = profil("eigen-a");
        s.speichern("eigen-a", &text, None).unwrap();
        s.waehlen("eigen-a").unwrap();
        s.loeschen("eigen-a", &text).unwrap();
        // Der Rückfall auf „Alles“ greift beim Lesen (TypeScript); aktiv.json bleibt, wie sie ist.
        assert_eq!(s.aktiv().unwrap().id.as_deref(), Some("eigen-a"));
        assert_eq!(s.lesen("eigen-a").unwrap(), None);
    }

    // ------------------------------------------------------------ Atomar schreiben, Fehler beim Schreiben

    #[test]
    fn datentraeger_voll_laesst_den_alten_stand_und_keine_hilfsdatei() {
        let (_t, s) = werkstatt();
        let alt = profil("eigen-a");
        s.speichern("eigen-a", &alt, None).unwrap();
        // Ein Schreiber, der nach der Hälfte „Datenträger voll“ (ENOSPC) meldet.
        let halb = |d: &mut File, b: &[u8]| -> io::Result<()> {
            d.write_all(&b[..b.len() / 2])?;
            Err(io::Error::from_raw_os_error(28))
        };
        let neu = alt.replace("Test", "Neu");
        assert_eq!(
            schreibe_atomar_mit(&s.verzeichnis, "eigen-a.json", neu.as_bytes(), &halb),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(
            bytes(&s, "eigen-a.json"),
            alt.as_bytes(),
            "kein halbes Profil"
        );
        assert!(hilfsdateien(&s).is_empty(), "Hilfsdatei aufgeräumt");
        // Neue Datei: es entsteht keine.
        assert_eq!(
            schreibe_atomar_mit(&s.verzeichnis, "eigen-b.json", neu.as_bytes(), &halb),
            Err(AssistentenFehler::WriteFailed)
        );
        assert!(!s.pfad("eigen-b.json").exists());
        assert!(hilfsdateien(&s).is_empty());
        // aktiv.json ebenso.
        s.waehlen("alles").unwrap();
        let aktiv_alt = bytes(&s, AKTIV_DATEI);
        assert_eq!(
            schreibe_atomar_mit(&s.verzeichnis, AKTIV_DATEI, b"{\"id\":\"x\"}", &halb),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(bytes(&s, AKTIV_DATEI), aktiv_alt);
    }

    #[test]
    fn umbenennen_scheitert_ziel_bleibt_hilfsdatei_weg() {
        let (_t, s) = werkstatt();
        fs::create_dir_all(s.pfad("ziel.json")).unwrap();
        lege(&s, "ziel.json/inhalt", b"x");
        assert_eq!(
            schreibe_atomar_mit(&s.verzeichnis, "ziel.json", b"{}", &|d, b| d.write_all(b)),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(fs::read(s.pfad("ziel.json/inhalt")).unwrap(), b"x");
        assert!(hilfsdateien(&s).is_empty());
    }

    #[test]
    fn liegengebliebene_hilfsdatei_stoert_nicht() {
        let (_t, s) = werkstatt();
        // Ein Absturz zwischen Schreiben und Umbenennen hinterlässt höchstens so etwas.
        lege(&s, ".lva-99-0.tmp", b"{\"halb");
        s.speichern("eigen-a", &profil("eigen-a"), None).unwrap();
        let l = s.liste().unwrap();
        assert_eq!(l.dateien.len(), 1);
        assert_eq!(
            s.aktiv().unwrap(),
            AssistentenAktiv {
                id: None,
                belegt: false
            }
        );
    }

    // ------------------------------------------------------------ Nebenläufigkeit

    #[test]
    fn parallele_neuanlage_derselben_id_gewinnt_genau_einer() {
        let (_t, s) = werkstatt();
        let s = Arc::new(s);
        let ergebnisse: Vec<_> = (0..16)
            .map(|i| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    s.speichern(
                        "eigen-a",
                        &profil("eigen-a").replace("Test", &format!("T{i}")),
                        None,
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(ergebnisse.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(ergebnisse
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| *e == AssistentenFehler::Occupied));
        assert!(hilfsdateien(&s).is_empty());
        assert!(ist_eigenes_profil(
            "eigen-a",
            &String::from_utf8(bytes(&s, "eigen-a.json")).unwrap()
        ));
    }

    #[test]
    fn parallele_ueberschreibversuche_vom_selben_stand_gewinnt_genau_einer() {
        let (_t, s) = werkstatt();
        let alt = profil("eigen-a");
        s.speichern("eigen-a", &alt, None).unwrap();
        let s = Arc::new(s);
        let ergebnisse: Vec<_> = (0..16)
            .map(|i| {
                let (s, alt) = (Arc::clone(&s), alt.clone());
                std::thread::spawn(move || {
                    s.speichern(
                        "eigen-a",
                        &alt.replace("Test", &format!("T{i}")),
                        Some(&alt),
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(
            ergebnisse.iter().filter(|r| r.is_ok()).count(),
            1,
            "kein verlorenes Update"
        );
    }

    #[test]
    fn parallele_neuanlagen_ueber_die_grenze_ergeben_genau_200_dateien() {
        let (_t, s) = werkstatt();
        let s = Arc::new(s);
        let ergebnisse: Vec<_> = (0..8)
            .map(|k| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    (0..40)
                        .map(|i| {
                            let id = format!("p{k}-{i:02}");
                            s.speichern(&id, &profil(&id), None)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        assert_eq!(ergebnisse.iter().filter(|r| r.is_ok()).count(), 200);
        assert!(ergebnisse
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| *e == AssistentenFehler::TooMany));
        assert_eq!(s.liste().unwrap().dateien.len(), 200);
        assert!(hilfsdateien(&s).is_empty());
    }

    #[test]
    fn parallel_auswaehlen_und_speichern_hinterlaesst_ein_auswahldokument() {
        let (_t, s) = werkstatt();
        lege_profil(&s, "eigen-a");
        let s = Arc::new(s);
        let handles: Vec<_> = (0..8)
            .map(|k| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let _ = s.waehlen(if (k + i) % 2 == 0 { "alles" } else { "eigen-a" });
                        let id = format!("n{k}-{i:02}");
                        let _ = s.speichern(&id, &profil(&id), None);
                        let a = s.aktiv().unwrap();
                        assert!(!a.belegt, "aktiv.json war nie halb geschrieben");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let a = s.aktiv().unwrap();
        assert!(matches!(a.id.as_deref(), Some("alles") | Some("eigen-a")));
        assert!(hilfsdateien(&s).is_empty());
    }

    // ------------------------------------------------------------ Export

    fn exportziel(t: &TempDir, name: &str) -> PathBuf {
        t.path().join(name)
    }

    fn tmp_reste(t: &TempDir) -> Vec<String> {
        fs::read_dir(t.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    fn exportiere(pfad: &Path, inhalt: &str) -> Result<(), AssistentenFehler> {
        exportiere_mit(pfad, inhalt, &|d, b| d.write_all(b))
    }

    #[test]
    fn export_schreibt_die_bytes_unveraendert_und_ersetzt_eine_vorhandene_datei() {
        let t = tempfile::tempdir().unwrap();
        let ziel = exportziel(&t, "mein-coach.json");
        let text = profil("mein-coach");
        exportiere(&ziel, &text).unwrap();
        assert_eq!(fs::read(&ziel).unwrap(), text.as_bytes());
        // Die Person hat im Dialog „Ersetzen“ gewählt: die Datei bekommt den neuen Inhalt, ganz oder gar nicht.
        let neu = profil("anders");
        exportiere(&ziel, &neu).unwrap();
        assert_eq!(fs::read(&ziel).unwrap(), neu.as_bytes());
        assert!(tmp_reste(&t).is_empty());
        // Endung in anderer Schreibweise, Umlaute im Namen.
        exportiere(&exportziel(&t, "Übung.JSON"), &text).unwrap();
        assert!(exportziel(&t, "Übung.JSON").is_file());
    }

    #[test]
    fn export_lehnt_unbrauchbare_ziele_ab_und_laesst_alles_wie_es_war() {
        let t = tempfile::tempdir().unwrap();
        let text = profil("a");
        // Falsche Endung, relativer Pfad, kein Dateiname, fehlendes Verzeichnis.
        assert_eq!(
            exportiere(&exportziel(&t, "a.txt"), &text),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(
            exportiere(Path::new("a.json"), &text),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(
            exportiere(Path::new("/"), &text),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(
            exportiere(&t.path().join("fehlt").join("a.json"), &text),
            Err(AssistentenFehler::WriteFailed)
        );
        // Ein Verzeichnis namens `a.json` wird nicht überschrieben.
        let verz = exportziel(&t, "dir.json");
        fs::create_dir(&verz).unwrap();
        assert_eq!(
            exportiere(&verz, &text),
            Err(AssistentenFehler::WriteFailed)
        );
        assert!(verz.is_dir());
        assert!(tmp_reste(&t).is_empty());
        assert!(!exportziel(&t, "a.txt").exists());
    }

    #[test]
    fn export_ueber_256_kib_wird_abgelehnt() {
        let t = tempfile::tempdir().unwrap();
        let ziel = exportziel(&t, "gross.json");
        let zu_gross = "x".repeat(MAX_BYTES + 1);
        assert_eq!(
            exportiere(&ziel, &zu_gross),
            Err(AssistentenFehler::TooLarge)
        );
        assert!(!ziel.exists());
        let genau = "x".repeat(MAX_BYTES);
        exportiere(&ziel, &genau).unwrap();
        assert_eq!(fs::metadata(&ziel).unwrap().len() as usize, MAX_BYTES);
    }

    #[test]
    fn export_datentraeger_voll_laesst_die_alte_datei_und_keine_hilfsdatei() {
        let t = tempfile::tempdir().unwrap();
        let ziel = exportziel(&t, "a.json");
        let alt = profil("alt");
        exportiere(&ziel, &alt).unwrap();
        // Ein Schreiber, der nach der Hälfte „Datenträger voll“ (ENOSPC) meldet.
        let voll = |d: &mut File, b: &[u8]| -> io::Result<()> {
            d.write_all(&b[..b.len() / 2])?;
            Err(io::Error::from_raw_os_error(28))
        };
        assert_eq!(
            exportiere_mit(&ziel, &profil("neu"), &voll),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(fs::read(&ziel).unwrap(), alt.as_bytes());
        assert!(tmp_reste(&t).is_empty());
        // Auch ohne vorhandenes Ziel entsteht keine halbe Datei.
        let neu_ziel = exportziel(&t, "neu.json");
        assert_eq!(
            exportiere_mit(&neu_ziel, &profil("neu"), &voll),
            Err(AssistentenFehler::WriteFailed)
        );
        assert!(!neu_ziel.exists());
        assert!(tmp_reste(&t).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn export_ueberschreibt_keinen_link() {
        let t = tempfile::tempdir().unwrap();
        let echt = exportziel(&t, "echt.json");
        fs::write(&echt, b"fremd").unwrap();
        let link = exportziel(&t, "link.json");
        std::os::unix::fs::symlink(&echt, &link).unwrap();
        assert_eq!(
            exportiere(&link, &profil("a")),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(fs::read(&echt).unwrap(), b"fremd");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn export_parallel_zum_selben_ziel_hinterlaesst_eine_ganze_datei() {
        let t = tempfile::tempdir().unwrap();
        let ziel = exportziel(&t, "a.json");
        let texte: Vec<String> = (0..8).map(|k| profil(&format!("p{k}"))).collect();
        let handles: Vec<_> = texte
            .iter()
            .cloned()
            .map(|text| {
                let ziel = ziel.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let _ = exportiere(&ziel, &text);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let gelesen = fs::read_to_string(&ziel).unwrap();
        assert!(
            texte.contains(&gelesen),
            "die Datei ist immer ein ganzer Text"
        );
        assert!(tmp_reste(&t).is_empty());
    }

    // ------------------------------------------------------------ Hilfsdateien: fremde bleiben unberührt

    /// Legt eine fremde Hilfsdatei unter genau dem Namen an, den der Zähler als Nächstes vergeben würde.
    fn fremde_hilfsdatei(verzeichnis: &Path, vorsatz: &str, nr: u64, inhalt: &[u8]) -> PathBuf {
        let pfad = verzeichnis.join(format!("{vorsatz}-{}-{nr}.tmp", std::process::id()));
        fs::write(&pfad, inhalt).unwrap();
        pfad
    }

    #[test]
    fn hilfsdatei_anlegen_nimmt_den_naechsten_zaehler_und_laesst_die_fremde_unberuehrt() {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(5);
        let fremd = fremde_hilfsdatei(t.path(), ".lva", 5, b"fremd");
        let (pfad, _datei) = hilfsdatei_anlegen(t.path(), ".lva", &zaehler).unwrap();
        assert_ne!(pfad, fremd);
        assert!(pfad.to_string_lossy().ends_with("-6.tmp"));
        assert_eq!(fs::read(&fremd).unwrap(), b"fremd");
    }

    #[test]
    fn hilfsdatei_anlegen_gibt_nach_den_versuchen_auf_und_loescht_nichts() {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(0);
        let fremde: Vec<PathBuf> = (0..HILFS_VERSUCHE as u64 + 3)
            .map(|n| fremde_hilfsdatei(t.path(), ".lva", n, format!("fremd{n}").as_bytes()))
            .collect();
        let fehler = hilfsdatei_anlegen(t.path(), ".lva", &zaehler).unwrap_err();
        assert_eq!(fehler.kind(), io::ErrorKind::AlreadyExists);
        // Es wurde begrenzt probiert, nicht endlos.
        assert_eq!(zaehler.load(Ordering::Relaxed), HILFS_VERSUCHE as u64);
        for (n, f) in fremde.iter().enumerate() {
            assert_eq!(fs::read(f).unwrap(), format!("fremd{n}").as_bytes());
        }
    }

    #[test]
    fn export_mit_vergebener_hilfsdatei_nimmt_einen_anderen_namen_und_laesst_die_fremde_stehen() {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(0);
        let fremd = fremde_hilfsdatei(t.path(), ".lva-export", 0, b"fremd");
        let ziel = exportziel(&t, "a.json");
        let text = profil("a");
        exportiere_mit_zaehler(&ziel, &text, &|d, b| d.write_all(b), &zaehler).unwrap();
        assert_eq!(fs::read(&ziel).unwrap(), text.as_bytes());
        assert_eq!(fs::read(&fremd).unwrap(), b"fremd");
        assert_eq!(
            tmp_reste(&t),
            vec![fremd.file_name().unwrap().to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn export_wenn_alle_namen_vergeben_sind_scheitert_ohne_etwas_zu_loeschen_oder_zu_aendern() {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(0);
        let fremde: Vec<PathBuf> = (0..HILFS_VERSUCHE as u64)
            .map(|n| fremde_hilfsdatei(t.path(), ".lva-export", n, format!("fremd{n}").as_bytes()))
            .collect();
        let ziel = exportziel(&t, "a.json");
        let alt = profil("alt");
        fs::write(&ziel, &alt).unwrap();
        assert_eq!(
            exportiere_mit_zaehler(&ziel, &profil("neu"), &|d, b| d.write_all(b), &zaehler),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(fs::read(&ziel).unwrap(), alt.as_bytes());
        for (n, f) in fremde.iter().enumerate() {
            assert_eq!(fs::read(f).unwrap(), format!("fremd{n}").as_bytes());
        }
        assert_eq!(tmp_reste(&t).len(), fremde.len());
    }

    #[test]
    fn export_schreibfehler_raeumt_nur_die_eigene_hilfsdatei_auf() {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(0);
        let fremd = fremde_hilfsdatei(t.path(), ".lva-export", 0, b"fremd");
        let voll = |d: &mut File, b: &[u8]| -> io::Result<()> {
            d.write_all(&b[..b.len() / 2])?;
            Err(io::Error::from_raw_os_error(28))
        };
        let ziel = exportziel(&t, "a.json");
        assert_eq!(
            exportiere_mit_zaehler(&ziel, &profil("a"), &voll, &zaehler),
            Err(AssistentenFehler::WriteFailed)
        );
        assert!(!ziel.exists());
        assert_eq!(fs::read(&fremd).unwrap(), b"fremd");
        assert_eq!(tmp_reste(&t).len(), 1, "nur die fremde Datei ist übrig");
    }

    #[test]
    fn speichern_mit_vergebener_hilfsdatei_nimmt_einen_anderen_namen_und_laesst_die_fremde_stehen()
    {
        let t = tempfile::tempdir().unwrap();
        let zaehler = AtomicU64::new(0);
        let fremd = fremde_hilfsdatei(t.path(), ".lva", 0, b"fremd");
        schreibe_atomar_mit_zaehler(
            t.path(),
            "eigen-a.json",
            b"{}",
            &|d, b| d.write_all(b),
            &zaehler,
        )
        .unwrap();
        assert_eq!(fs::read(t.path().join("eigen-a.json")).unwrap(), b"{}");
        assert_eq!(fs::read(&fremd).unwrap(), b"fremd");
        // Wenn gar kein Name frei ist, bleibt das Ziel, wie es war, und nichts Fremdes verschwindet.
        let alle = AtomicU64::new(100);
        let fremde: Vec<PathBuf> = (100..100 + HILFS_VERSUCHE as u64)
            .map(|n| fremde_hilfsdatei(t.path(), ".lva", n, b"fremd"))
            .collect();
        assert_eq!(
            schreibe_atomar_mit_zaehler(
                t.path(),
                "eigen-a.json",
                b"neu",
                &|d, b| d.write_all(b),
                &alle
            ),
            Err(AssistentenFehler::WriteFailed)
        );
        assert_eq!(fs::read(t.path().join("eigen-a.json")).unwrap(), b"{}");
        for f in fremde {
            assert_eq!(fs::read(f).unwrap(), b"fremd");
        }
    }
}
