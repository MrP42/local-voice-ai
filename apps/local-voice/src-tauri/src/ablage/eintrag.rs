//! D7: Einträge im verknüpften Baum lesen, ohne sie zu öffnen, und über sie urteilen (fail-closed).
//!
//! Unter Windows liegen OneDrive-Dateien, die nur online sind, als Platzhalter (Reparse-Point mit Cloud-Tag) auf
//! dem Datenträger. Wer sie öffnet oder liest, löst einen Download aus. `fs::symlink_metadata` liefert dafür
//! nicht die echten Attribute: Rust öffnet einen Reparse-Point, der kein Symlink ist, noch einmal ohne
//! `FILE_FLAG_OPEN_REPARSE_POINT` und meldet dann `0x500020` statt `0x501620` (ohne `REPARSE_POINT` und
//! `OFFLINE`). Deshalb liest dieses Modul unter Windows Attribute, Größe und Zeit nur über `FindFirstFileExW`
//! (`FindExInfoBasic`): Die Funktion öffnet die Datei nicht und liefert `dwFileAttributes` samt `dwReserved0`
//! (Reparse-Tag bei gesetztem `REPARSE_POINT`). Die Metadaten von `DirEntry` werden nicht benutzt.
//!
//! Urteil ([`reparse_urteil`], rein und plattformneutral): zuerst „nur online“ (`RECALL_ON_DATA_ACCESS`,
//! `RECALL_ON_OPEN`, `OFFLINE`), unabhängig vom Reparse-Bit und bei Dateien wie bei Ordnern; ein solcher
//! Eintrag wird nicht gelesen, geöffnet oder betreten (auch das Auflisten eines Ordners kann eine
//! Synchronisierung anstoßen). Danach der Tag: Symlink, Mount-Point und unbekannte Tags werden abgewiesen,
//! Cloud-Tags nur bei hydrierten Einträgen zugelassen.
//!
//! Fehlerfälle und Absicherung:
//! * Datei wird zwischen Abtastung und Lesen (oder zwischen Prüfung und Öffnen) wieder online: zweistufiges
//!   Öffnen ([`oeffnen_ohne_abruf`]). Stufe 1 öffnet nur mit `FILE_READ_ATTRIBUTES` (das löst keinen Abruf
//!   aus) und urteilt am Handle, noch einmal unmittelbar vor dem Hochstufen am selben Handle; nur bei
//!   „lesbar“ wird DIESES Handle auf Lesezugriff hochgestuft (`ReOpenFile`, dasselbe Objekt, kein zweiter
//!   Pfadzugriff; lva#507), dort erneut am Handle geurteilt und die Datei-Kennung verglichen. Der Schutz
//!   kommt aus dem Urteil VOR dem Lesezugriff; `FILE_FLAG_OPEN_NO_RECALL` allein verhindert den Abruf nicht
//!   (echte OneDrive-Probe: Öffnen mit Lesezugriff lud die Datei herunter). Restrisiko, das nur eine
//!   Windows-/Provider-Sperre schließen könnte: der Anbieter lagert die Datei zwischen dem letzten Urteil und
//!   dem Hochstufen aus (ein Systemaufruf lang); das Hochstufen löst dann den Abruf aus, gelesen wird aber
//!   erst nach dem Urteil am Ergebnis. Zugesagt wird deshalb nur: Local Voice überspringt erkannte, nur
//!   online gespeicherte Dateien; durch gleichzeitige Änderungen des Synchronisationsdienstes kann beim Lesen
//!   dennoch ein Download ausgelöst werden (Restfenster, `docs/ablage-abrufschutz.md`).
//! * Zwischenverzeichnisse ([`Ordner`], lva#507): Wurzel und jede Ebene werden einmal geöffnet, am Handle
//!   beurteilt und gehalten; Auflisten und Dateiöffnen laufen relativ dazu. Ein Zwischenglied, das nach der
//!   Prüfung durch eine Junction ersetzt wird, erreicht die Lesezugriffe nie. Jede Ebene besitzt ihren
//!   Elternordner (Ahnenbindung): solange ein Kind lebt, bleibt die ganze Kette offen.
//! * Attribute, Tag oder Handle nicht bestimmbar (Fehler der Windows-Aufrufe): als Umweg abgewiesen.
//! * Auflisten bricht mittendrin ab (Laufwerk weg, Zugriff verweigert): der ganze Ordner gilt als nicht
//!   lesbar, nie eine Teilliste (sonst fielen bekannte Dateien aus dem Bestand).
//! * Unbekannte oder künftige Tags und Attribute: Tags nur über die Cloud-Maske zugelassen, jedes andere
//!   Reparse-Tag abgewiesen; jedes der drei Online-Bits genügt.
//!
//! macOS (lva#472): Cloud-Platzhalter von OneDrive und iCloud („Dateien bei Bedarf“, File Provider) sind
//! „dataless“: `st_flags` trägt `SF_DATALESS`, die Größe bleibt sichtbar, Blöcke und Inhalt fehlen. `lstat`
//! und `fstat` lösen keinen Abruf aus (gemessen 10.11.2026, macOS 15.8.1, OneDrive), ebenso ein `open` mit
//! `O_EVTONLY`; auch ein einfaches `open` zum Lesen tat es dort nicht, doch der erste `read` lädt die Datei
//! herunter. Deshalb dieselbe Folge wie unter Windows: Rohdaten aus `lstat` ([`attribute_aus_stat_flags`]
//! bildet `SF_DATALESS` auf [`ATTR_OFFLINE`] ab, derselbe Zustand und Code „nur online“ wie unter Windows),
//! Urteil VOR jedem Öffnen; Stufe 1 öffnet mit `O_EVTONLY` (kein Lesezugriff, [`oeffnen_ohne_abruf`]) und urteilt
//! am Handle (`fstat`); nur bei „lesbar“ folgt Stufe 2 mit Lesezugriff, erneutes Urteil, Kennung (Gerät,
//! Inode) verglichen. Zusätzlich schaltet [`abruf_sperren`] für den lesenden Faden die Materialisierung aus
//! (`setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, …_OFF)`): ein Lesezugriff
//! auf einen Platzhalter, der im Fenster nach dem Urteil ausgelagert wird, scheitert dann mit `EDEADLK`
//! ([`ist_abruf_gesperrt`]) statt eines Downloads. Ordner, die „dataless“ sind, werden weder betreten noch
//! aufgelistet (`readdir` kann sie materialisieren). Ohne `O_NOFOLLOW_ANY` könnte ein Zwischenglied des Pfades
//! zwischen Prüfung und Öffnen durch einen Symlink ersetzt werden; deshalb öffnet macOS ohne Folgen mit
//! `O_NOFOLLOW_ANY` (kein Symlink in keinem Glied). Grenzen: ein echter „dataless“-Ordner lässt sich ohne File
//! Provider nicht herstellen (nur über die Naht geprüft); die Thread-Richtlinie wirkt nur im eigenen Faden
//! (ein Kindprozess liest nie die Quelle, nur die Momentkopie); iCloud vor macOS 14 legte `.Name.icloud`-Stubs an
//! (Punktdateien, werden ohnehin übersprungen).
//!
//! Plattformneutral getestet wird über die Test-Naht [`naht`], die Attribute, Tags und `st_flags` einspeist.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

mod ordner;
pub use ordner::Ordner;

/// Verzeichnis (nur Windows liefert Attribute).
#[cfg_attr(not(windows), allow(dead_code))]
pub const ATTR_VERZEICHNIS: u32 = 0x10;
/// Versteckt.
pub const ATTR_VERSTECKT: u32 = 0x2;
/// System.
pub const ATTR_SYSTEM: u32 = 0x4;
/// Reparse-Point (Symlink, Junction, Cloud-Platzhalter, …).
pub const ATTR_REPARSE_POINT: u32 = 0x400;
/// Daten nicht sofort verfügbar (Offline).
pub const ATTR_OFFLINE: u32 = 0x1000;
/// Das Öffnen löst einen Abruf aus.
pub const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
/// Der Datenzugriff löst einen Abruf aus (OneDrive: nur online).
pub const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
/// Jedes dieser Bits heißt: nur online, nie lesen, öffnen oder betreten.
pub const ATTR_NUR_ONLINE: u32 = ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS;

/// macOS: `SF_DATALESS` in `st_flags` (`sys/stat.h`, nur lesbar, setzt der Kernel für File-Provider-Platzhalter):
/// die Datei oder der Ordner ist ein Platzhalter ohne lokalen Inhalt.
pub const SF_DATALESS: u32 = 0x4000_0000;

/// Bildet die `st_flags` eines Eintrags (macOS) auf das Attributfeld ab: `SF_DATALESS` wird [`ATTR_OFFLINE`]
/// (derselbe Zustand „nur online“ wie unter Windows, alle Verbraucher urteilen über [`online_urteil`]); alle
/// anderen Flags (versteckt, komprimiert, verfolgt, …) haben mit „nur online“ nichts zu tun und fallen weg.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn attribute_aus_stat_flags(flags: u32) -> u32 {
    if flags & SF_DATALESS != 0 {
        ATTR_OFFLINE
    } else {
        0
    }
}

/// Reparse-Tag: Symlink.
pub const REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Reparse-Tag: Mount-Point (Junction).
pub const REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
/// Basis der Cloud-Tags (`IO_REPARSE_TAG_CLOUD`); `IO_REPARSE_TAG_CLOUD_1` bis `_F` setzen die Bits 12 bis 15.
const REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
const REPARSE_TAG_CLOUD_MASKE: u32 = 0xFFFF_0FFF;

/// Ist `tag` ein Cloud-Tag (OneDrive und andere Cloud-Dateien-Anbieter)?
pub fn reparse_tag_cloud(tag: u32) -> bool {
    tag & REPARSE_TAG_CLOUD_MASKE == REPARSE_TAG_CLOUD
}

/// Urteil über einen Eintrag im verknüpften Baum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urteil {
    /// Gewöhnlich oder hydrierter Cloud-Platzhalter: darf gelesen und betreten werden.
    Lesbar,
    /// Nur online: weder lesen noch öffnen noch betreten (sonst Download oder Synchronisierung).
    NurOnline,
    /// Verknüpfung (Symlink, Junction) oder unbekannter Reparse-Point: nie folgen, nie lesen.
    Umweg,
}

/// Verlangen die Attribute einen Abruf (nur online)? Gilt für Dateien und Ordner, unabhängig vom
/// Reparse-Bit.
pub fn online_urteil(attribute: u32) -> bool {
    attribute & ATTR_NUR_ONLINE != 0
}

/// Wertet Attribute und Reparse-Tag in dieser Reihenfolge: (1) nur online, unabhängig vom Reparse-Bit und
/// vom Tag; (2) ohne Reparse-Bit ist der Tag belanglos: lesbar; (3) Symlink, Mount-Point und jedes nicht
/// bekannte Tag: Umweg; Cloud-Tags sind hier nur noch hydriert, also lesbar.
pub fn reparse_urteil(tag: u32, attribute: u32) -> Urteil {
    if online_urteil(attribute) {
        return Urteil::NurOnline;
    }
    if attribute & ATTR_REPARSE_POINT == 0 {
        return Urteil::Lesbar;
    }
    if tag == REPARSE_TAG_SYMLINK || tag == REPARSE_TAG_MOUNT_POINT || !reparse_tag_cloud(tag) {
        return Urteil::Umweg;
    }
    Urteil::Lesbar
}

/// Art eines Eintrags (ohne Verknüpfungen zu folgen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gattung {
    Datei,
    Ordner,
    /// Symlink (nur außerhalb von Windows; dort entscheidet der Reparse-Tag).
    #[cfg_attr(windows, allow(dead_code))]
    Link,
    /// Gerät, Pipe, Socket, …
    #[cfg_attr(windows, allow(dead_code))]
    Anderes,
}

/// Rohdaten eines Eintrags, ohne ihn zu öffnen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roh {
    pub gattung: Gattung,
    /// Windows-Dateiattribute (`dwFileAttributes`); macOS: [`ATTR_OFFLINE`] für einen Platzhalter
    /// (`SF_DATALESS`, siehe [`attribute_aus_stat_flags`]); sonst 0.
    pub attribute: u32,
    /// Reparse-Tag bei gesetztem `REPARSE_POINT`, sonst 0.
    pub tag: u32,
    pub groesse: u64,
    /// Änderungszeit (ms seit 1970).
    pub zeit: i64,
    /// Anzahl der Verzeichniseinträge (nur Unix); sonst 1.
    pub nlink: u64,
}

impl Roh {
    /// Das Urteil über diesen Eintrag.
    pub fn urteil(&self) -> Urteil {
        if self.gattung == Gattung::Link {
            return Urteil::Umweg;
        }
        reparse_urteil(self.tag, self.attribute)
    }

    pub fn ist_datei(&self) -> bool {
        self.gattung == Gattung::Datei
    }

    pub fn ist_ordner(&self) -> bool {
        self.gattung == Gattung::Ordner
    }

    /// Windows: versteckt oder System (wird übersprungen).
    pub fn versteckt(&self) -> bool {
        self.attribute & (ATTR_VERSTECKT | ATTR_SYSTEM) != 0
    }
}

/// Ein Eintrag eines Verzeichnisses: Name und, wenn lesbar, die Rohdaten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eintrag {
    pub name: OsString,
    /// `None`: Eintrag nicht lesbar (zählt als ausgelassen).
    pub roh: Option<Roh>,
}

/// Rohdaten eines Pfades, ohne ihn zu öffnen und ohne einer Verknüpfung zu folgen.
pub fn roh(p: &Path) -> io::Result<Roh> {
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut r = system::eins(p)?;
    #[cfg(test)]
    naht::anwenden(p, &mut r);
    Ok(r)
}

/// Attribute und Reparse-Tag einer schon geöffneten Datei, vom Handle (`GetFileInformationByHandleEx` mit
/// `FileAttributeTagInfo`), nicht vom Pfad. `None`: nicht bestimmbar. `p` ist nur für die Test-Naht.
pub fn offen_daten(f: &fs::File, p: &Path) -> Option<(u32, u32)> {
    let _ = p;
    #[cfg(test)]
    if naht::handle_ausfall(p) {
        return None;
    }
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut d = system::offen(f).ok();
    #[cfg(test)]
    if let Some(x) = naht::handle(p) {
        d = Some(x);
    }
    d
}

/// Urteil über eine schon geöffnete Datei, vom Handle (siehe [`offen_daten`]). Nicht bestimmbar: Umweg (im
/// Zweifel ablehnen).
pub fn offen_urteil(f: &fs::File, p: &Path) -> Urteil {
    match offen_daten(f, p) {
        Some((attribute, tag)) => reparse_urteil(tag, attribute),
        None => Urteil::Umweg,
    }
}

/// Kennung der geöffneten Datei: Windows (Volume-Seriennummer, Datei-Index), Unix (Gerät, Inode). `p` ist nur
/// für die Test-Naht.
pub fn datei_id(f: &fs::File, p: &Path) -> io::Result<(u64, u64)> {
    let _ = p;
    #[cfg(test)]
    if let Some(x) = naht::id(p) {
        return Ok(x);
    }
    system::id(f)
}

/// Wie [`oeffnen_ohne_abruf`] öffnet.
#[derive(Debug, Clone, Copy)]
pub struct OeffnenArt {
    /// `false`: keiner Verknüpfung und keinem Reparse-Point folgen (Windows: `FILE_FLAG_OPEN_REPARSE_POINT`,
    /// Unix: `O_NOFOLLOW`, macOS: `O_NOFOLLOW_ANY`, auch kein Zwischenglied). `true`: normal öffnen (WOF und
    /// Dedup liefern dann den Inhalt).
    pub folgen: bool,
    /// Windows: Freigabe (`FILE_SHARE_*`) der Lesestufe; Stufe 1 gibt alles frei.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub teilen: u32,
}

/// Warum [`oeffnen_ohne_abruf`] nichts liefert.
#[derive(Debug)]
pub enum OeffnenFehler {
    /// Das Urteil am Handle (Stufe 1 oder 2) war nicht „lesbar“; gelesen wurde nichts.
    Urteil(Urteil),
    /// Zwischen Stufe 1 und Stufe 2 wurde die Datei getauscht (andere Kennung); gelesen wurde nichts.
    Getauscht,
    /// Öffnen oder Abfrage scheiterte (z. B. Sharing-Verletzung, Datei fehlt).
    Io(io::Error),
}

/// Urteil über Attribute und Tag, wie [`reparse_urteil`] (die Ablage) oder strenger/weicher (Aufrufer).
pub type UrteilFn = fn(tag: u32, attribute: u32) -> Urteil;

/// Zweistufiges Öffnen einer Datei zum Lesen, ohne dass das Öffnen selbst einen Abruf (OneDrive-Download)
/// auslöst:
///
/// 1. Öffnen nur mit `FILE_READ_ATTRIBUTES` (Windows, ohne Lesezugriff; mit `FILE_FLAG_OPEN_REPARSE_POINT`
///    und `FILE_FLAG_OPEN_NO_RECALL`; macOS: `O_EVTONLY`, ohne Lesezugriff und ohne Abruf), Urteil und
///    Datei-Kennung am Handle (macOS: `fstat`). Nicht „lesbar“: Ende, Stufe 2 wird nie aufgerufen.
/// 2. Nur bei „lesbar“ (und nach einem erneuten Urteil am selben Handle): das Stufe-1-Handle wird auf
///    Lesezugriff nach `art` hochgestuft, OHNE den Pfad noch einmal zu öffnen (Windows `ReOpenFile` auf
///    dasselbe Objekt; Unix: Duplikat des Handles; macOS, dessen Stufe 1 nur `O_EVTONLY` ist: `openat` im
///    geprüften Ordner, sonst Pfad). Erneut am Handle urteilen, Kennung mit Stufe 1 vergleichen (Tausch:
///    [`OeffnenFehler::Getauscht`]).
///
/// Das Öffnen mit Lesezugriff ist nicht abrufsicher (`FILE_FLAG_OPEN_NO_RECALL` allein genügt nicht); der
/// Schutz ist das Urteil davor. Restrisiko: Auslagern im Fenster zwischen dem letzten Urteil und dem
/// Hochstufen (ein Systemaufruf, siehe [`zweistufig`]); unter macOS fängt es die Abrufsperre ([`abruf_sperren`], hier für beide Stufen gesetzt) ab: Fehler statt
/// Download. Auch ein Vorab-Urteil am Pfad ([`roh`]) ersetzt das nicht, es spart nur Öffnungen.
///
/// **Aufrufer MÜSSEN [`abruf_sperren`] über das Lesen halten** und [`ist_abruf_gesperrt`] als „nur online“
/// melden: die Sperre in dieser Funktion endet mit der Rückgabe und schützt das spätere Lesen aus der
/// zurückgegebenen Datei NICHT (unter macOS lädt erst der `read` herunter). Kann die Sperre unter macOS nicht
/// gesetzt werden, öffnet diese Funktion nichts (fail-closed, [`OeffnenFehler::Io`]).
pub fn oeffnen_ohne_abruf(
    p: &Path,
    art: OeffnenArt,
    urteil: UrteilFn,
) -> Result<fs::File, OeffnenFehler> {
    // macOS: auch ein Zugriff im Fenster nach dem Urteil lädt nichts herunter (Fehler statt Abruf). Ohne die
    // Sperre wird nicht geöffnet.
    let sperre = abruf_sperren();
    if sperre.fehlt() {
        return Err(OeffnenFehler::Io(io::Error::other(
            "Abrufsperre nicht setzbar",
        )));
    }
    let eins = system::stufe1(p, art).map_err(oeffnen_io)?;
    zweistufig(eins, p, urteil, &|e| system::hochstufen(e, p, art))
}

/// Urteil und Kennung am Stufe-1-Handle, erneutes Urteil unmittelbar vor dem Hochstufen (am selben Handle),
/// Hochstufen auf Lesezugriff über `hoch` (darf den Pfad NICHT noch einmal öffnen: Windows `ReOpenFile`,
/// Unix Duplikat bzw. `openat` im geprüften Ordner), Urteil und Kennung am Lese-Handle.
///
/// Restrisiko, das KEIN Code schließt (nur eine Provider-/Windows-Sperre): Der Cloud-Anbieter kann die Datei
/// zwischen dem letzten Urteil am Stufe-1-Handle und dem Öffnen mit Lesezugriff auslagern; das Öffnen löst
/// dann den Abruf aus. Das Fenster ist ein Systemaufruf lang (`ReOpenFile` ohne Pfadauflösung); das Urteil
/// VOR dem Hochstufen und am Ergebnis halten es klein, die Lesezugriffe folgen erst nach dem Urteil.
pub(super) fn zweistufig(
    eins: fs::File,
    p: &Path,
    urteil: UrteilFn,
    hoch: &dyn Fn(&fs::File) -> io::Result<fs::File>,
) -> Result<fs::File, OeffnenFehler> {
    let beurteilen = |f: &fs::File| -> Result<(), OeffnenFehler> {
        let Some((attribute, tag)) = offen_daten(f, p) else {
            return Err(OeffnenFehler::Urteil(Urteil::Umweg));
        };
        match urteil(tag, attribute) {
            Urteil::Lesbar => Ok(()),
            u => Err(OeffnenFehler::Urteil(u)),
        }
    };
    beurteilen(&eins)?;
    let id1 = datei_id(&eins, p).map_err(OeffnenFehler::Io)?;
    // Noch einmal, unmittelbar vor dem Hochstufen: der Zustand kann sich seit dem ersten Urteil geändert haben.
    beurteilen(&eins)?;
    #[cfg(test)]
    naht::stufe2_gezaehlt(p);
    #[cfg(test)]
    naht::vor_hochstufen(p);
    let zwei = hoch(&eins).map_err(oeffnen_io)?;
    beurteilen(&zwei)?;
    if datei_id(&zwei, p).map_err(OeffnenFehler::Io)? != id1 {
        return Err(OeffnenFehler::Getauscht);
    }
    drop(eins);
    Ok(zwei)
}

/// Fehler eines Öffnens: „Abruf verweigert“ (macOS `EDEADLK`) heißt nur online, ein Symlink im Pfad (macOS
/// `ELOOP` bei `O_NOFOLLOW_ANY`) ein Umweg, alles andere ist ein E/A-Fehler.
fn oeffnen_io(e: io::Error) -> OeffnenFehler {
    if ist_abruf_gesperrt(&e) {
        OeffnenFehler::Urteil(Urteil::NurOnline)
    } else if ist_symlink_im_pfad(&e) {
        OeffnenFehler::Urteil(Urteil::Umweg)
    } else {
        OeffnenFehler::Io(e)
    }
}

/// macOS: das Öffnen ohne Folgen (`O_NOFOLLOW_ANY`) traf einen Symlink in einem Glied des Pfades (`ELOOP`).
/// Anderswo ändert sich der Fehlercode nicht.
fn ist_symlink_im_pfad(e: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        e.raw_os_error() == Some(libc::ELOOP)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = e;
        false
    }
}

/// Hat das System den Zugriff verweigert, weil er einen Platzhalter herunterladen müsste? macOS: `EDEADLK`
/// (so antwortet ein Lesezugriff auf einen „dataless“-Platzhalter, solange [`abruf_sperren`] gilt, gemessen);
/// sonst nie. Aufrufer, die nach [`oeffnen_ohne_abruf`] lesen, melden das als „nur online“.
pub fn ist_abruf_gesperrt(e: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        e.raw_os_error() == Some(libc::EDEADLK)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = e;
        false
    }
}

/// Schaltet für den aufrufenden Faden die Materialisierung von Platzhaltern aus, solange der Wert lebt (macOS:
/// `setiopolicy_np` mit `IOPOL_SCOPE_THREAD`; der alte Wert kommt beim Drop zurück, auch bei einer Panik,
/// verschachtelt gilt die äußere Sperre weiter). Ein Lesezugriff auf einen Platzhalter scheitert dann mit
/// `EDEADLK` ([`ist_abruf_gesperrt`]) statt einen Download auszulösen. Das ist die Sicherung für das Fenster
/// nach dem Urteil; das Urteil VOR dem Öffnen bleibt der eigentliche Schutz. Wer eine Datei aus
/// [`oeffnen_ohne_abruf`] liest, hält die Sperre über das Lesen. Ohne Allokation, ohne Sperre, ohne E/A;
/// nicht `Send` (gilt nur für den Faden, der sie setzt). Außerhalb von macOS ohne Wirkung.
#[must_use = "die Sperre gilt nur, solange der Wert lebt"]
pub fn abruf_sperren() -> AbrufSperre {
    #[cfg(target_os = "macos")]
    {
        mac_abruf::sperren()
    }
    #[cfg(not(target_os = "macos"))]
    {
        AbrufSperre {
            wirksam: false,
            _faden: std::marker::PhantomData,
        }
    }
}

/// Siehe [`abruf_sperren`].
#[derive(Debug)]
#[must_use = "die Sperre gilt nur, solange der Wert lebt"]
pub struct AbrufSperre {
    /// Der Wert vor dem Setzen, falls beim Drop wiederherzustellen (macOS).
    #[cfg(target_os = "macos")]
    vorher: Option<i32>,
    wirksam: bool,
    _faden: std::marker::PhantomData<*const ()>,
}

impl AbrufSperre {
    /// Hat das System die Richtlinie angenommen? (Nur macOS; sonst `false`: dort gibt es nichts abzurufen.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wirksam(&self) -> bool {
        self.wirksam
    }

    /// Wäre die Sperre nötig (macOS), das System hat sie aber nicht angenommen? Dann liest oder listet der
    /// Aufrufer nicht (fail-closed): das Urteil vor dem Öffnen bliebe sonst der einzige Schutz. Praktisch
    /// ausgeschlossen (die Richtlinie gibt es seit macOS 10.15, der Mindestversion der App), aber nie still.
    pub fn fehlt(&self) -> bool {
        cfg!(target_os = "macos") && !self.wirksam
    }
}

#[cfg(target_os = "macos")]
impl Drop for AbrufSperre {
    fn drop(&mut self) {
        if let Some(v) = self.vorher {
            mac_abruf::wiederherstellen(v);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac_abruf {
    use super::AbrufSperre;
    use std::marker::PhantomData;
    use std::os::raw::c_int;
    use std::sync::atomic::{AtomicBool, Ordering};

    extern "C" {
        fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
        fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
    }

    /// `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES`, `IOPOL_SCOPE_THREAD` (`sys/resource.h`).
    const TYP: c_int = 3;
    const FADEN: c_int = 1;
    /// `IOPOL_MATERIALIZE_DATALESS_FILES_OFF`; `_DEFAULT` ist 0, die Grundmaske (ohne `_ORIG`) 3.
    const AUS: c_int = 1;
    const STANDARD: c_int = 0;
    const GRUNDMASKE: c_int = 3;

    /// Meldet höchstens einmal je Prozess (kein Pfad, kein Name), dass die Richtlinie nicht gesetzt werden kann.
    static GEMELDET: AtomicBool = AtomicBool::new(false);

    fn melden(was: &str, fehler: c_int) {
        if !GEMELDET.swap(true, Ordering::Relaxed) {
            log::warn!("ablage: Abrufsperre nicht setzbar ({was}, errno {fehler}); verknüpfte Ordner werden nicht gelesen");
        }
    }

    pub fn sperren() -> AbrufSperre {
        #[cfg(test)]
        if super::naht::sperre_verweigert() {
            return AbrufSperre {
                vorher: None,
                wirksam: false,
                _faden: PhantomData,
            };
        }
        // SAFETY: `getiopolicy_np` liest nur die E/A-Richtlinie des aufrufenden Fadens.
        let vorher = unsafe { getiopolicy_np(TYP, FADEN) };
        let grund = vorher & GRUNDMASKE;
        let (wirksam, wiederherstellen) = if vorher < 0 {
            melden(
                "getiopolicy_np",
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            );
            (false, None)
        } else if grund == AUS {
            // Schon aus (eine äußere Sperre): nichts zu setzen, nichts zurückzugeben.
            (true, None)
        } else if setzen(AUS) {
            (true, Some(grund))
        } else {
            melden(
                "setiopolicy_np",
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            );
            (false, None)
        };
        AbrufSperre {
            vorher: wiederherstellen,
            wirksam,
            _faden: PhantomData,
        }
    }

    fn setzen(wert: c_int) -> bool {
        // SAFETY: `setiopolicy_np` setzt nur die E/A-Richtlinie des aufrufenden Fadens.
        unsafe { setiopolicy_np(TYP, FADEN, wert) == 0 }
    }

    pub fn wiederherstellen(vorher: c_int) {
        if !setzen(vorher) {
            let _ = setzen(STANDARD);
        }
    }
}

#[cfg(not(windows))]
mod system {
    use super::{Gattung, OeffnenArt, Roh};
    use std::fs;
    use std::io;
    use std::path::Path;

    /// macOS: `st_flags` des Eintrags (aus `lstat`/`fstat`, öffnet nichts).
    #[cfg(target_os = "macos")]
    fn flags_von(m: &fs::Metadata) -> u32 {
        use std::os::macos::fs::MetadataExt;
        m.st_flags()
    }

    fn aus_meta(m: &fs::Metadata) -> Roh {
        let t = m.file_type();
        let gattung = if t.is_symlink() {
            Gattung::Link
        } else if t.is_dir() {
            Gattung::Ordner
        } else if t.is_file() {
            Gattung::Datei
        } else {
            Gattung::Anderes
        };
        #[cfg(unix)]
        let nlink = {
            use std::os::unix::fs::MetadataExt;
            m.nlink()
        };
        #[cfg(not(unix))]
        let nlink = 1;
        #[cfg(target_os = "macos")]
        let attribute = super::attribute_aus_stat_flags(flags_von(m));
        #[cfg(not(target_os = "macos"))]
        let attribute = 0;
        Roh {
            gattung,
            attribute,
            tag: 0,
            groesse: m.len(),
            zeit: m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64),
            nlink,
        }
    }

    pub fn eins(p: &Path) -> io::Result<Roh> {
        fs::symlink_metadata(p).map(|m| aus_meta(&m))
    }

    /// Attribute am offenen Handle: macOS aus `fstat` (`SF_DATALESS`), sonst keine.
    pub fn offen(f: &fs::File) -> io::Result<(u32, u32)> {
        #[cfg(target_os = "macos")]
        {
            Ok((
                super::attribute_aus_stat_flags(flags_von(&f.metadata()?)),
                0,
            ))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = f;
            Ok((0, 0))
        }
    }

    /// Öffnen zum Lesen. `nur_ereignisse` (macOS, Stufe 1): `O_EVTONLY`, kein Lesezugriff und kein Abruf.
    /// Ohne Folgen: `O_NOFOLLOW`, macOS `O_NOFOLLOW_ANY` (kein Symlink in keinem Glied; ab macOS 11, sonst
    /// `EINVAL` und dann `O_NOFOLLOW`).
    fn oeffnen(p: &Path, art: OeffnenArt, nur_ereignisse: bool) -> io::Result<fs::File> {
        #[cfg(not(target_os = "macos"))]
        let _ = nur_ereignisse;
        let versuch = |ohne_any: bool| -> io::Result<fs::File> {
            let _ = ohne_any;
            let mut o = fs::OpenOptions::new();
            o.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut fl: libc::c_int = 0;
                if !art.folgen {
                    #[cfg(target_os = "macos")]
                    {
                        fl |= if ohne_any {
                            libc::O_NOFOLLOW
                        } else {
                            libc::O_NOFOLLOW_ANY
                        };
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        fl |= libc::O_NOFOLLOW;
                    }
                    fl |= libc::O_NONBLOCK;
                }
                #[cfg(target_os = "macos")]
                if nur_ereignisse {
                    fl |= libc::O_EVTONLY;
                }
                if fl != 0 {
                    o.custom_flags(fl);
                }
            }
            #[cfg(not(unix))]
            let _ = art;
            o.open(p)
        };
        match versuch(false) {
            #[cfg(target_os = "macos")]
            Err(e) if !art.folgen && e.raw_os_error() == Some(libc::EINVAL) => versuch(true),
            r => r,
        }
    }

    /// Stufe 1: macOS nur für Ereignisse (`O_EVTONLY`, löst keinen Abruf aus); sonst wie Stufe 2 (außerhalb
    /// von Windows und macOS gibt es keinen Abruf).
    pub fn stufe1(p: &Path, art: OeffnenArt) -> io::Result<fs::File> {
        oeffnen(p, art, true)
    }

    /// Hochstufen auf Lesezugriff ohne zweite Pfadöffnung: Stufe 1 ist hier schon lesbar (Duplikat desselben
    /// Handles). macOS: Stufe 1 hat nur `O_EVTONLY`, dort öffnet der Pfad noch einmal (die Abrufsperre und
    /// `O_NOFOLLOW_ANY` sichern das; im geprüften Ordner läuft es über `openat`, siehe `Ordner`).
    pub fn hochstufen(eins: &fs::File, p: &Path, art: OeffnenArt) -> io::Result<fs::File> {
        #[cfg(target_os = "macos")]
        {
            let _ = eins;
            oeffnen(p, art, false)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (p, art);
            eins.try_clone()
        }
    }

    pub fn id(f: &fs::File) -> io::Result<(u64, u64)> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let m = f.metadata()?;
            Ok((m.dev(), m.ino()))
        }
        #[cfg(not(unix))]
        {
            let _ = f;
            Ok((0, 0))
        }
    }
}

#[cfg(windows)]
mod system {
    use super::{Gattung, OeffnenArt, Roh, ATTR_REPARSE_POINT, ATTR_VERZEICHNIS};
    use std::ffi::{c_void, OsStr};
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
        GetFileInformationByHandle, GetFileInformationByHandleEx, ReOpenFile,
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAGS_AND_ATTRIBUTES,
        FILE_SHARE_MODE, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
    };

    /// 100-ns-Schritte zwischen 1601-01-01 und 1970-01-01.
    const FILETIME_BIS_1970: u64 = 116_444_736_000_000_000;

    /// Schließt das Suchhandle (auch bei einem frühen Rückkehr).
    struct Suche(HANDLE);

    impl Drop for Suche {
        fn drop(&mut self) {
            // SAFETY: gültiges Suchhandle aus `FindFirstFileExW`, wird genau einmal geschlossen.
            let _ = unsafe { FindClose(self.0) };
        }
    }

    fn breit(p: &OsStr) -> io::Result<Vec<u16>> {
        let mut v: Vec<u16> = p.encode_wide().collect();
        if v.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        v.push(0);
        Ok(v)
    }

    fn roh_aus(d: &WIN32_FIND_DATAW) -> Roh {
        let attribute = d.dwFileAttributes;
        let tag = if attribute & ATTR_REPARSE_POINT != 0 {
            d.dwReserved0
        } else {
            0
        };
        let ft = (u64::from(d.ftLastWriteTime.dwHighDateTime) << 32)
            | u64::from(d.ftLastWriteTime.dwLowDateTime);
        Roh {
            gattung: if attribute & ATTR_VERZEICHNIS != 0 {
                Gattung::Ordner
            } else {
                Gattung::Datei
            },
            attribute,
            tag,
            groesse: (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow),
            zeit: ft
                .checked_sub(FILETIME_BIS_1970)
                .map_or(0, |x| (x / 10_000) as i64),
            nlink: 1,
        }
    }

    /// Ein Eintrag über `FindFirstFileExW`: öffnet die Datei nicht, folgt keiner Verknüpfung.
    pub fn eins(p: &Path) -> io::Result<Roh> {
        let w = breit(p.as_os_str())?;
        let mut d = WIN32_FIND_DATAW::default();
        // SAFETY: `w` ist NUL-terminiert und lebt über den Aufruf; `d` ist ein beschreibbarer
        // `WIN32_FIND_DATAW` (bei `FindExInfoBasic` ist das der erwartete Puffer).
        let h = unsafe {
            FindFirstFileExW(
                PCWSTR(w.as_ptr()),
                FindExInfoBasic,
                std::ptr::addr_of_mut!(d).cast::<c_void>(),
                FindExSearchNameMatch,
                None,
                FIND_FIRST_EX_LARGE_FETCH,
            )
        }
        .map_err(win_io)?;
        let _suche = Suche(h);
        Ok(roh_aus(&d))
    }

    /// Attribute und Reparse-Tag am offenen Handle (`FileAttributeTagInfo`).
    pub fn offen(f: &fs::File) -> io::Result<(u32, u32)> {
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: gültiges, offenes Handle; `info` ist ein beschreibbarer Puffer der angegebenen Größe.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(f.as_raw_handle()),
                FileAttributeTagInfo,
                std::ptr::addr_of_mut!(info).cast::<c_void>(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        }
        .map_err(win_io)?;
        Ok((info.FileAttributes, info.ReparseTag))
    }

    /// Win32-Fehler als `io::Error` mit erhaltenem Code (HRESULT_FROM_WIN32 → Win32-Code), damit
    /// `raw_os_error()` z. B. ERROR_SHARING_VIOLATION (32) erkennt; andere HRESULTs bleiben `other`.
    pub fn win_io(e: windows::core::Error) -> io::Error {
        let hr = e.code().0 as u32;
        if hr & 0xFFFF_0000 == 0x8007_0000 {
            io::Error::from_raw_os_error((hr & 0xFFFF) as i32)
        } else {
            io::Error::other(e)
        }
    }

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_ALLES: u32 = 0x1 | 0x2 | 0x4;
    pub const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    /// Stufe 1: nur `FILE_READ_ATTRIBUTES`, nichts lesen, alles freigeben; öffnet die Verknüpfung oder den
    /// Platzhalter selbst (`FILE_FLAG_OPEN_REPARSE_POINT`).
    pub fn stufe1(p: &Path, _art: OeffnenArt) -> io::Result<fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_ALLES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
            .open(p)
    }

    /// Hochstufen: `ReOpenFile` auf dasselbe Objekt wie Stufe 1 (kein zweiter Pfadzugriff, also kein Tausch
    /// eines Pfadgliedes und keine neue Auflösung). Lesezugriff; `FILE_FLAG_OPEN_REPARSE_POINT` nur, wenn der
    /// Aufrufer nicht folgen will. Das Urteil davor und danach macht der Aufrufer.
    pub fn hochstufen(eins: &fs::File, _p: &Path, art: OeffnenArt) -> io::Result<fs::File> {
        use std::os::windows::io::FromRawHandle;
        // `ReOpenFile` weist `FILE_FLAG_OPEN_NO_RECALL` mit ERROR_INVALID_PARAMETER ab (auf Windows gemessen,
        // jede Kombination). Der Abrufschutz liegt deshalb im Urteil am Stufe-1-Handle (ohne Lesezugriff, mit
        // NO_RECALL geöffnet), das unmittelbar davor abgeschlossen ist: ein Platzhalter wird nie hochgestuft.
        let mut flags = 0u32;
        if !art.folgen {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }
        // SAFETY: gültiges, offenes Handle (`eins` lebt über den Aufruf); `ReOpenFile` liefert ein neues,
        // eigenes Handle, das die `File` besitzt.
        let h = unsafe {
            ReOpenFile(
                HANDLE(eins.as_raw_handle()),
                GENERIC_READ,
                FILE_SHARE_MODE(art.teilen),
                FILE_FLAGS_AND_ATTRIBUTES(flags),
            )
        }
        .map_err(win_io)?;
        // SAFETY: `h` ist ein frisches, gültiges Handle, das sonst niemand besitzt.
        Ok(unsafe { fs::File::from_raw_handle(h.0) })
    }

    /// Volume-Seriennummer und Datei-Index am Handle.
    pub fn id(f: &fs::File) -> io::Result<(u64, u64)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: gültiges, offenes Handle; `info` ist ein beschreibbarer Puffer.
        unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info) }
            .map_err(win_io)?;
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}

/// Test-Naht: speist Attribute und Tags ein, wie Windows sie für OneDrive-Dateien liefert (die Art des
/// Eintrags, Größe und Zeit bleiben die echten). Gilt nur für den eigenen Test-Faden.
#[cfg(test)]
pub(crate) mod naht {
    use super::Roh;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::path::{Path, PathBuf};

    type Karte = RefCell<BTreeMap<PathBuf, (u32, u32)>>;

    thread_local! {
        /// Die Abrufsperre lässt sich nicht setzen (Fehlerfall von `setiopolicy_np`).
        static SPERRE_VERWEIGERT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Pfad → `st_flags` (macOS), wie `lstat` sie für den Pfad liefert.
        static STAT_FLAGS: RefCell<BTreeMap<PathBuf, u32>> = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → `st_flags` (macOS), wie `fstat` sie am offenen Handle liefert.
        static HANDLE_STAT_FLAGS: RefCell<BTreeMap<PathBuf, u32>> = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → (Attribute, Tag) der Find-Daten.
        static FIND: Karte = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → (Attribute, Tag) am offenen Handle.
        static HANDLE: Karte = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → Handle-Antworten der Reihe nach (eine je Abfrage; danach gilt `HANDLE`).
        static HANDLE_FOLGE: RefCell<BTreeMap<PathBuf, VecDeque<(u32, u32)>>> = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → Datei-Kennungen der Reihe nach (eine je Abfrage; danach die echte).
        static ID_FOLGE: RefCell<BTreeMap<PathBuf, VecDeque<(u64, u64)>>> = const { RefCell::new(BTreeMap::new()) };
        /// Pfad → wie oft Stufe 2 (Lesezugriff) aufgerufen wurde.
        static STUFE2: RefCell<BTreeMap<PathBuf, usize>> = const { RefCell::new(BTreeMap::new()) };
        /// Pfade, deren Handle-Abfrage scheitert (Attribute nicht bestimmbar).
        static HANDLE_AUSFALL: RefCell<BTreeSet<PathBuf>> = const { RefCell::new(BTreeSet::new()) };
        /// Haken unmittelbar vor dem Hochstufen auf Lesezugriff (Test: Pfad dazwischen austauschen).
        static VOR_HOCHSTUFEN: RefCell<Option<Box<dyn Fn(&Path)>>> = const { RefCell::new(None) };
    }

    /// Die Handle-Abfrage dieses Pfades scheitert (wie ein Fehler von `GetFileInformationByHandleEx`).
    pub fn handle_ausfall_setzen(p: &Path) {
        HANDLE_AUSFALL.with(|k| k.borrow_mut().insert(p.to_path_buf()));
    }

    pub(super) fn handle_ausfall(p: &Path) -> bool {
        HANDLE_AUSFALL.with(|k| k.borrow().contains(p))
    }

    /// Ein Haken, der unmittelbar vor dem Hochstufen auf Lesezugriff läuft (nur dieser Faden).
    pub fn vor_hochstufen_haken(f: Option<Box<dyn Fn(&Path)>>) {
        VOR_HOCHSTUFEN.with(|k| *k.borrow_mut() = f);
    }

    pub(super) fn vor_hochstufen(p: &Path) {
        VOR_HOCHSTUFEN.with(|k| {
            if let Some(f) = k.borrow().as_ref() {
                f(p);
            }
        });
    }

    /// Find-Daten dieses Pfades (Attribute, Tag) samt vorgetäuschten `st_flags`, falls gesetzt.
    pub(super) fn find_daten(p: &Path) -> Option<(u32, u32)> {
        let f = FIND.with(|k| k.borrow().get(p).copied());
        let s = STAT_FLAGS.with(|k| k.borrow().get(p).copied());
        if f.is_none() && s.is_none() {
            return None;
        }
        let (mut attribute, tag) = f.unwrap_or((0, 0));
        if let Some(flags) = s {
            attribute |= super::attribute_aus_stat_flags(flags);
        }
        Some((attribute, tag))
    }

    /// Die Handle-Abfragen dieser Datei melden der Reihe nach (Attribute, Tag); danach gilt `handle_setzen`.
    pub fn handle_folge(p: &Path, antworten: &[(u32, u32)]) {
        HANDLE_FOLGE.with(|k| {
            k.borrow_mut()
                .insert(p.to_path_buf(), antworten.iter().copied().collect());
        });
    }

    /// Die Kennungsabfragen dieser Datei melden der Reihe nach diese Werte.
    pub fn id_folge(p: &Path, ids: &[(u64, u64)]) {
        ID_FOLGE.with(|k| {
            k.borrow_mut()
                .insert(p.to_path_buf(), ids.iter().copied().collect());
        });
    }

    /// Wie oft Stufe 2 (Lesezugriff) für diese Datei erreicht wurde.
    pub fn stufe2_aufrufe(p: &Path) -> usize {
        STUFE2.with(|k| k.borrow().get(p).copied().unwrap_or(0))
    }

    pub(super) fn stufe2_gezaehlt(p: &Path) {
        STUFE2.with(|k| *k.borrow_mut().entry(p.to_path_buf()).or_insert(0) += 1);
    }

    pub(super) fn id(p: &Path) -> Option<(u64, u64)> {
        ID_FOLGE.with(|k| k.borrow_mut().get_mut(p).and_then(VecDeque::pop_front))
    }

    /// Das System verweigert in diesem Faden die Abrufsperre (macOS; wie ein Fehler von `setiopolicy_np`).
    #[cfg(target_os = "macos")]
    pub fn sperre_verweigern(an: bool) {
        SPERRE_VERWEIGERT.with(|c| c.set(an));
    }

    #[cfg(target_os = "macos")]
    pub(super) fn sperre_verweigert() -> bool {
        SPERRE_VERWEIGERT.with(std::cell::Cell::get)
    }

    /// `lstat` dieses Pfades meldet diese `st_flags` (z. B. [`super::SF_DATALESS`]); sie werden wie unter macOS
    /// über [`super::attribute_aus_stat_flags`] abgebildet.
    pub fn stat_flags_setzen(p: &Path, flags: u32) {
        STAT_FLAGS.with(|k| k.borrow_mut().insert(p.to_path_buf(), flags));
    }

    /// `fstat` am offenen Handle dieser Datei meldet diese `st_flags`.
    pub fn handle_stat_flags_setzen(p: &Path, flags: u32) {
        HANDLE_STAT_FLAGS.with(|k| k.borrow_mut().insert(p.to_path_buf(), flags));
    }

    /// Öffnet wie Stufe 1 von [`super::oeffnen_ohne_abruf`] (Test: Öffnungsflags prüfen).
    pub fn stufe1_fuer_test(p: &Path, art: super::OeffnenArt) -> std::io::Result<std::fs::File> {
        super::system::stufe1(p, art)
    }

    /// Find-Daten dieses Pfades (Attribute, Tag) vortäuschen.
    pub fn setzen(p: &Path, attribute: u32, tag: u32) {
        FIND.with(|k| k.borrow_mut().insert(p.to_path_buf(), (attribute, tag)));
    }

    /// Das Handle dieser Datei meldet (Attribute, Tag).
    pub fn handle_setzen(p: &Path, attribute: u32, tag: u32) {
        HANDLE.with(|k| k.borrow_mut().insert(p.to_path_buf(), (attribute, tag)));
    }

    pub fn leeren() {
        SPERRE_VERWEIGERT.with(|c| c.set(false));
        STAT_FLAGS.with(|k| k.borrow_mut().clear());
        HANDLE_STAT_FLAGS.with(|k| k.borrow_mut().clear());
        FIND.with(|k| k.borrow_mut().clear());
        HANDLE.with(|k| k.borrow_mut().clear());
        HANDLE_FOLGE.with(|k| k.borrow_mut().clear());
        ID_FOLGE.with(|k| k.borrow_mut().clear());
        STUFE2.with(|k| k.borrow_mut().clear());
        HANDLE_AUSFALL.with(|k| k.borrow_mut().clear());
        VOR_HOCHSTUFEN.with(|k| *k.borrow_mut() = None);
    }

    pub(super) fn anwenden(p: &Path, r: &mut Roh) {
        if let Some((attribute, tag)) = FIND.with(|k| k.borrow().get(p).copied()) {
            r.attribute = attribute;
            r.tag = tag;
        }
        if let Some(f) = STAT_FLAGS.with(|k| k.borrow().get(p).copied()) {
            r.attribute |= super::attribute_aus_stat_flags(f);
        }
    }

    pub(super) fn handle(p: &Path) -> Option<(u32, u32)> {
        if let Some(x) =
            HANDLE_FOLGE.with(|k| k.borrow_mut().get_mut(p).and_then(VecDeque::pop_front))
        {
            return Some(x);
        }
        if let Some(f) = HANDLE_STAT_FLAGS.with(|k| k.borrow().get(p).copied()) {
            return Some((super::attribute_aus_stat_flags(f), 0));
        }
        HANDLE.with(|k| k.borrow().get(p).copied())
    }
}
