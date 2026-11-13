//! Verankerte Ordnerzugriffe fuer den Ladeweg des Nemotron-Modells.
//!
//! [`safe`] prueft Pfade (`symlink_metadata`, dann Oeffnen). Zwischen Pruefung und
//! Oeffnen bleibt ein Fenster, und jeder Schritt durch den Pfad geht erneut durch
//! alle Zwischenordner. Wer in dem Fenster einen Ordner des Pfades durch einen Link
//! ersetzt, lenkt Schreiben, Umbenennen und Loeschen in einen fremden Ordner.
//!
//! Hier ist das anders: [`Anchor`] oeffnet die Kette Datenwurzel -> Modellordner
//! Komponente fuer Komponente, ohne einem Link zu folgen, und haelt den Modellordner
//! (und jeden Ordner davor) als offenes Handle bis zum Ende. Jede Dateioperation
//! (anlegen, anhaengen, abschneiden, lesen, umbenennen, loeschen) geht danach
//! ueber dieses Handle und einen einzelnen Namen, nie ueber einen Pfad:
//!
//! - Unix: `cap-std` (`openat2`/`openat` relativ zum Ordner-Handle, `renameat`,
//!   `unlinkat`); ein Ersetzen des Ordners im Dateisystem aendert nichts daran, dass
//!   alle Operationen im ALTEN Ordner landen. [`Anchor::still_at`] meldet es.
//! - Windows: `cap-std` oeffnet Ordner OHNE `FILE_SHARE_DELETE`; solange das Handle
//!   offen ist, laesst sich der Ordner (und jeder Ordner davor) weder umbenennen noch
//!   loeschen noch durch eine Junction ersetzen. Dateien zum Pruefen werden ohne
//!   `FILE_SHARE_WRITE`/`FILE_SHARE_DELETE` geoeffnet. Ob ein Eintrag im Ordner
//!   eine Junction oder ein Symlink ist, sagen die Attribute (`reparse`), nicht ein
//!   Kennungsvergleich: `cap-std` kennt unter Windows in stabilem Rust keinen
//!   Dateiindex; den liefert hier `GetFileInformationByHandle` ([`safe::file_info`]).
//!
//! Pro Datei zusaetzlich (wie in [`safe`], aber ueber das Handle): vor dem Oeffnen
//! `symlink_metadata` des Namens (Link, Ordner, Geraet: Ablehnung), nach dem Oeffnen
//! die Pruefung des Handles (normale Datei, kein Reparse-Punkt, zum Schreiben nicht
//! mehrfach verlinkt) und der erneute Vergleich Name gegen Handle (Unix: Geraet und
//! Inode; Windows: Art und Reparse-Attribut), BEVOR irgendetwas geaendert wird. Ein
//! Abschneiden geschieht nie beim Oeffnen, sondern erst danach (`set_len`).
//!
//! Grenze, ehrlich: `cap-std` folgt einem Link INNERHALB des Ordners (zeigt er hinaus,
//! ist es ein Fehler). Ein Eintrag, der im Fenster zwischen Namenspruefung und Oeffnen
//! zu einem Link auf eine andere Datei desselben Ordners wird, wird durch den
//! Vergleich Name gegen Handle abgelehnt, ehe geschrieben wird; was ueber den Namen
//! umbenannt wird, prueft ohnehin der Hash beim Laden ([`check_set`]), nicht der Pfad.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata, OpenOptions};
use sha2::{Digest, Sha256};

use super::safe::{self, file_info};

/// Ein Eintrag im Ordner, ohne Link-Folgen (`symlink_metadata`) gelesen.
struct Entry {
    is_link: bool,
    is_dir: bool,
    is_file: bool,
    len: u64,
    /// Kennung (Geraet, Inode); unter Windows liefert `symlink_metadata` keine.
    id: Option<(u64, u64)>,
}

impl Entry {
    fn of(meta: &Metadata) -> Self {
        #[cfg(unix)]
        let id = {
            use cap_std::fs::MetadataExt;
            Some((meta.dev(), meta.ino()))
        };
        #[cfg(not(unix))]
        let id = None;
        Self {
            is_link: is_link(meta),
            is_dir: meta.is_dir(),
            is_file: meta.is_file(),
            len: meta.len(),
            id,
        }
    }

    /// Ist das eine normale Datei (kein Link, kein Ordner), die zu `info` passt?
    fn is_the_file(&self, info: &safe::FileInfo) -> bool {
        !self.is_link && self.is_file && self.id.is_none_or(|id| id == (info.volume, info.index))
    }
}

/// Symbolischer Link oder (unter Windows) irgendein Reparse-Punkt.
fn is_link(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if meta.file_attributes() & 0x0000_0400 != 0 {
            return true;
        }
    }
    false
}

fn entry(dir: &Dir, name: &str) -> io::Result<Option<Entry>> {
    match dir.symlink_metadata(name) {
        Ok(meta) => Ok(Some(Entry::of(&meta))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn dir_info(dir: &Dir) -> io::Result<safe::FileInfo> {
    file_info(&dir.try_clone()?.into_std_file())
}

/// Ein einzelner, harmloser Dateiname: nie ein Pfad.
fn plain(name: &str) -> io::Result<()> {
    if crate::catalog::is_plain_name(name) {
        Ok(())
    } else {
        Err(safe::refusal())
    }
}

/// Ein Schritt der Kette: der Unterordner `name` von `parent`, ohne Link zu folgen.
/// `None`, wenn er fehlt und nicht angelegt werden soll.
fn step(parent: &Dir, name: &str, create: bool) -> io::Result<Option<Dir>> {
    plain(name)?;
    let mut before = entry(parent, name)?;
    if before.is_none() {
        if !create {
            return Ok(None);
        }
        match parent.create_dir(name) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        before = entry(parent, name)?;
    }
    let Some(before) = before else {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    };
    if before.is_link || !before.is_dir {
        return Err(safe::refusal());
    }
    let dir = parent.open_dir(name).map_err(|e| {
        let foreign = entry(parent, name)
            .ok()
            .flatten()
            .is_some_and(|now| now.is_link || !now.is_dir);
        if foreign {
            safe::refusal()
        } else {
            e
        }
    })?;
    let info = dir_info(&dir)?;
    let same = before.id.is_none_or(|id| id == (info.volume, info.index));
    if !info.is_dir || info.reparse || !same {
        return Err(safe::refusal());
    }
    Ok(Some(dir))
}

/// Die Kette `root` + `parts`, jeder Ordner als Handle. `root` selbst darf ein Link
/// sein (ein verschobener Datenordner), alles darunter nicht.
fn walk(root: &Path, parts: &[String], create: bool) -> io::Result<Option<Vec<Dir>>> {
    if create {
        std::fs::create_dir_all(root)?;
    }
    let first = match Dir::open_ambient_dir(root, ambient_authority()) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut chain = vec![first];
    for part in parts {
        let parent = chain.last().expect("Kette nicht leer");
        match step(parent, part, create)? {
            Some(dir) => chain.push(dir),
            None => return Ok(None),
        }
    }
    Ok(Some(chain))
}

/// Der Modellordner als offenes Handle samt der Kette davor.
pub(crate) struct Anchor {
    chain: Vec<Dir>,
    root: PathBuf,
    parts: Vec<String>,
    id: (u64, u64),
}

/// Ohne Pfade: ein Anker taucht in Fehlertexten und Logs nie mit seinem Ordner auf.
impl std::fmt::Debug for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Anchor")
            .field("depth", &self.parts.len())
            .finish_non_exhaustive()
    }
}

impl Anchor {
    /// Oeffnet `root`/`parts...` Komponente fuer Komponente ohne Link-Folgen. Mit
    /// `create` werden fehlende Ordner angelegt, sonst ist ein fehlender Rest `None`.
    /// Ein Link oder Nicht-Ordner auf dem Weg ist eine Ablehnung ([`safe::is_refusal`]).
    pub(crate) fn open(root: &Path, parts: &[&str], create: bool) -> io::Result<Option<Self>> {
        let parts: Vec<String> = parts.iter().map(|p| (*p).to_string()).collect();
        let Some(chain) = walk(root, &parts, create)? else {
            return Ok(None);
        };
        let info = dir_info(chain.last().expect("Kette nicht leer"))?;
        Ok(Some(Self {
            chain,
            root: root.to_path_buf(),
            parts,
            id: (info.volume, info.index),
        }))
    }

    fn dir(&self) -> &Dir {
        self.chain.last().expect("Kette nicht leer")
    }

    /// Die Datenwurzel, ab der die Kette geoeffnet wurde (nur fuer Tests und Meldungen).
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Liegt der Ordner noch dort, wo der Pfad ihn hinfuehrt? Die Kette wird erneut
    /// ohne Link-Folgen abgelaufen; ein ersetzter Ordner, ein Link an seiner Stelle
    /// oder ein anderer Ordner mit demselben Namen ist eine Ablehnung. Wer Erfolg
    /// meldet, prueft das zuletzt: die Dateien, die der Lader unter dem Pfad sucht,
    /// liegen sonst nicht dort, wo sie verifiziert wurden.
    pub(crate) fn still_at(&self) -> io::Result<()> {
        let Some(chain) = walk(&self.root, &self.parts, false)? else {
            return Err(safe::refusal());
        };
        let info = dir_info(chain.last().expect("Kette nicht leer"))?;
        if (info.volume, info.index) == self.id {
            Ok(())
        } else {
            Err(safe::refusal())
        }
    }

    /// Laenge der normalen Datei `name`; `None`, wenn sie fehlt. Ein Link oder etwas
    /// anderes als eine normale Datei ist eine Ablehnung.
    pub(crate) fn plain_len(&self, name: &str) -> io::Result<Option<u64>> {
        plain(name)?;
        match entry(self.dir(), name)? {
            None => Ok(None),
            Some(e) if e.is_link || !e.is_file => Err(safe::refusal()),
            Some(e) => Ok(Some(e.len)),
        }
    }

    /// Wurde das Oeffnen abgelehnt, weil dort jetzt ein Link oder Nicht-Datei liegt?
    fn refused_or(&self, name: &str, e: io::Error) -> io::Error {
        let foreign = entry(self.dir(), name)
            .ok()
            .flatten()
            .is_some_and(|now| now.is_link || !now.is_file);
        if foreign {
            safe::refusal()
        } else {
            e
        }
    }

    /// Oeffnet die normale Datei `name` (zum Schreiben auch lesbar, nie abgeschnitten
    /// oder angehaengt: der Aufrufer setzt die Position). Alle Pruefungen laufen vor
    /// jeder Aenderung; `create` legt eine fehlende Datei an (nur mit `write`).
    fn open_checked(&self, name: &str, write: bool, create: bool) -> io::Result<File> {
        plain(name)?;
        match entry(self.dir(), name)? {
            Some(e) if e.is_link || !e.is_file => return Err(safe::refusal()),
            None if !create => return Err(io::Error::from(io::ErrorKind::NotFound)),
            _ => {}
        }
        let mut options = OpenOptions::new();
        options.read(true);
        if write {
            options.write(true);
        }
        if create {
            options.create(true);
        }
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            if !write {
                // FILE_SHARE_READ: waehrend wir lesen, schreibt, loescht und ersetzt
                // niemand diese Datei.
                options.share_mode(0x0000_0001);
            }
        }
        let file = self
            .dir()
            .open_with(name, &options)
            .map_err(|e| self.refused_or(name, e))?
            .into_std();
        let info = file_info(&file)?;
        if info.reparse || !info.is_file || (write && info.links > 1) {
            return Err(safe::refusal());
        }
        match entry(self.dir(), name)? {
            Some(e) if e.is_the_file(&info) => Ok(file),
            _ => Err(safe::refusal()),
        }
    }

    /// Oeffnet die normale Datei `name` zum Lesen, ohne einem Link zu folgen.
    /// `NotFound`, wenn sie fehlt.
    pub(crate) fn open_read(&self, name: &str) -> io::Result<File> {
        self.open_checked(name, false, false)
    }

    /// Oeffnet (oder legt an) die Teildatei `name` zum Lesen und Schreiben, ohne sie
    /// zu aendern. Eine Datei mit mehreren Namen (Hardlink), ein Link und eine
    /// Nicht-Datei sind abgelehnt, bevor ein Byte geschrieben oder abgeschnitten ist.
    pub(crate) fn open_partial(&self, name: &str) -> io::Result<File> {
        self.open_checked(name, true, true)
    }

    /// Ist `name` noch genau die Datei, die `file` geoeffnet hat? (Unix: gleiche
    /// Kennung; Windows: normale Datei ohne Reparse-Punkt, die Kennung liefert
    /// `symlink_metadata` dort nicht; ersetzen laesst sich die Datei dort ohnehin nicht,
    /// solange das Handle ohne `FILE_SHARE_DELETE` offen ist.)
    pub(crate) fn names(&self, name: &str, file: &File) -> io::Result<bool> {
        plain(name)?;
        let info = file_info(file)?;
        Ok(entry(self.dir(), name)?.is_some_and(|e| e.is_the_file(&info)))
    }

    /// Loescht die normale Datei `name`; fehlt sie, ist das kein Fehler. Ein Link oder
    /// etwas anderes als eine normale Datei wird abgelehnt und bleibt liegen.
    pub(crate) fn remove(&self, name: &str) -> io::Result<()> {
        if self.plain_len(name)?.is_none() {
            return Ok(());
        }
        match self.dir().remove_file(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Entfernt den EINTRAG `name`, auch wenn er ein Link ist (ohne ihm zu folgen: ein
    /// Link verschwindet, sein Ziel bleibt). Ein Ordner wird abgelehnt. Fehlt der
    /// Eintrag, ist das kein Fehler. Das ist der letzte Ausweg beim Zurueckziehen des
    /// Graphen: ein Eintrag, der nicht umbenannt werden kann, muss trotzdem weg.
    pub(crate) fn unlink(&self, name: &str) -> io::Result<()> {
        plain(name)?;
        match entry(self.dir(), name)? {
            None => Ok(()),
            Some(e) if e.is_dir && !e.is_link => Err(safe::refusal()),
            Some(_) => match self.dir().remove_file(name) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        }
    }

    /// Benennt `from` ueber `to` um, beide im Ordner. Beide Enden muessen normale
    /// Dateien sein (`to` darf fehlen); ein Link an einem Ende wird abgelehnt.
    pub(crate) fn rename_over(&self, from: &str, to: &str) -> io::Result<()> {
        if self.plain_len(from)?.is_none() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        self.plain_len(to)?;
        self.dir().rename(from, self.dir(), to)
    }
}

/// Was eine Datei gegenueber dem Katalog sein soll.
pub(crate) struct Wanted<'a> {
    pub(crate) name: &'a str,
    pub(crate) size: u64,
    pub(crate) sha256: Option<&'a str>,
}

/// Ergebnis der Pruefung einer Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    Valid,
    Missing,
    /// Hat nicht die Groesse aus dem Katalog (die gefundene steht dabei).
    Size(u64),
    /// Richtige Groesse, falsche SHA-256.
    Hash,
    /// Waehrend der Pruefung gewachsen oder geschrumpft, oder der Name zeigt nicht
    /// mehr auf die Datei, die geprueft wurde.
    Changed,
}

/// Schreibmarken fuer Tests: wo sich ein Tausch einschieben laesst.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Point {
    /// Alle Dateien sind geoeffnet, noch keine gelesen.
    Opened,
    /// Die Datei mit diesem Index ist gelesen und gehasht.
    Checked(usize),
    /// Der Lader hat alle Bytes im Speicher, noch keine Sitzung gebaut.
    Loaded,
}

#[cfg(test)]
pub(crate) mod hook {
    use super::Point;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    type Callback = Arc<dyn Fn(Point) + Send + Sync>;
    static HOOKS: Mutex<Vec<(PathBuf, Callback)>> = Mutex::new(Vec::new());

    /// Haengt `f` an die Datenwurzel `root`; es feuert fuer jeden Anker dieser Wurzel,
    /// auf jedem Thread, und nur fuer sie (Tests laufen parallel).
    pub(crate) struct Guard(PathBuf);

    pub(crate) fn set(root: &Path, f: impl Fn(Point) + Send + Sync + 'static) -> Guard {
        HOOKS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((root.to_path_buf(), Arc::new(f)));
        Guard(root.to_path_buf())
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            HOOKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(root, _)| root != &self.0);
        }
    }

    pub(crate) fn fire(root: &Path, point: Point) {
        let callback = HOOKS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(r, _)| r == root)
            .map(|(_, f)| f.clone());
        if let Some(callback) = callback {
            callback(point);
        }
    }
}

/// Meldet `point` an einen Test-Haken, falls einer an dieser Wurzel haengt.
#[cfg(test)]
pub(crate) fn fire(root: &Path, point: Point) {
    hook::fire(root, point);
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn fire(_root: &Path, _point: Point) {}

/// Empfaenger der gelesenen Bytes: Dateiindex und Block.
pub(crate) type Sink<'a> = &'a mut dyn FnMut(usize, &[u8]);

/// Prueft alle Dateien `wanted` im Ordner des Ankers nach Groesse und SHA-256, ueber
/// EIN Handle je Datei: erst werden alle Handles geoeffnet und bleiben es, dann wird
/// jede Datei durch ihr Handle gelesen und gehasht, zuletzt prueft der Anker, dass
/// jeder Name noch auf die Datei zeigt, deren Handle gehasht wurde. `Valid` heisst
/// also: ein Paar, wie es zum Ende der Pruefung unter diesen Namen lag, mit genau
/// diesen Bytes.
///
/// `sink` bekommt die gelesenen Bytes (Index der Datei, Block): der Lader sammelt sie
/// und baut die Sitzung aus GENAU diesen geprueften Bytes, ohne die Datei ein zweites
/// Mal ueber den Pfad zu oeffnen. Ohne `sink` wird nur gehasht. Es werden nie mehr als
/// `size + 1` Bytes je Datei gelesen, ein wachsender Tausch fuellt keinen Speicher.
/// Ohne Pruefsumme im Katalog (`sha256: None`) und ohne `sink` zaehlt nur die Groesse.
///
/// `Ok(None)`: waehrend des Lesens abgebrochen (`cancel`). Ein Link oder eine
/// Nicht-Datei am Namen ist ein Fehler (Ablehnung), keine Datei, der gefolgt wird.
pub(crate) fn check_set(
    anchor: &Anchor,
    wanted: &[Wanted<'_>],
    cancel: &dyn Fn() -> bool,
    mut sink: Option<Sink<'_>>,
) -> io::Result<Option<Vec<Check>>> {
    let mut files: Vec<Option<File>> = Vec::with_capacity(wanted.len());
    for w in wanted {
        match anchor.open_read(w.name) {
            Ok(file) => files.push(Some(file)),
            Err(e) if e.kind() == io::ErrorKind::NotFound && !safe::is_refusal(&e) => {
                files.push(None)
            }
            Err(e) => return Err(e),
        }
    }
    fire(anchor.root(), Point::Opened);

    let mut states = Vec::with_capacity(wanted.len());
    let mut block = vec![0u8; 1 << 20];
    for (i, (w, file)) in wanted.iter().zip(files.iter_mut()).enumerate() {
        let Some(file) = file else {
            states.push(Check::Missing);
            continue;
        };
        let len = file.metadata()?.len();
        if len != w.size {
            states.push(Check::Size(len));
            fire(anchor.root(), Point::Checked(i));
            continue;
        }
        let mut hasher = w.sha256.map(|_| Sha256::new());
        let mut total = len;
        if hasher.is_some() || sink.is_some() {
            total = 0;
            let mut reader = file.by_ref().take(w.size + 1);
            loop {
                if cancel() {
                    return Ok(None);
                }
                let n = reader.read(&mut block)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if let Some(hasher) = hasher.as_mut() {
                    hasher.update(&block[..n]);
                }
                if let Some(sink) = sink.as_mut() {
                    sink(i, &block[..n]);
                }
            }
        }
        let digest_matches = match (hasher, w.sha256) {
            (Some(hasher), Some(expected)) => {
                format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected)
            }
            _ => true,
        };
        states.push(if total != w.size {
            Check::Changed
        } else if !digest_matches {
            Check::Hash
        } else {
            Check::Valid
        });
        fire(anchor.root(), Point::Checked(i));
    }

    for ((w, file), state) in wanted.iter().zip(&files).zip(states.iter_mut()) {
        let Some(file) = file else { continue };
        if *state == Check::Valid && !anchor.names(w.name, file)? {
            *state = Check::Changed;
        }
    }
    Ok(Some(states))
}

#[cfg(test)]
mod tests;
