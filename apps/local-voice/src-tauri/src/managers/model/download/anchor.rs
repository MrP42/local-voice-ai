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
//!   eine Junction oder ein Symlink ist, sagen die Attribute (`reparse`); ob ein Name
//!   auf genau das Handle zeigt, sagt die 128-Bit-Kennung (`FileIdInfo`, siehe
//!   [`safe::Identity`]), gelesen an einem zweiten Handle desselben Namens, das einem
//!   Reparse-Punkt nicht folgt.
//!
//! Name gegen Handle, auf allen Systemen ([`names_handle`]): nach dem Oeffnen einer
//! Datei UND nach dem Oeffnen jedes Ordners der Kette wird geprueft, dass der Name
//! (ohne Link-Folgen gelesen) keinen Link und nichts anderes als das Erwartete ist und
//! dieselbe Kennung hat wie das offene Handle, BEVOR irgendetwas im Ordner getan wird.
//! Ein Ordner, der im Fenster zwischen Namenspruefung und `open_dir` durch einen Link
//! auf einen Nachbarordner ersetzt wurde, ist damit abgelehnt (unter Unix auch dann,
//! wenn `cap-std` dem relativen Link innerhalb des Elternordners folgt). Ist die
//! Kennung unbekannt (Windows: `FileIdInfo` nicht lesbar, Kennung nur Nullen oder nur
//! `0xFF`), gilt jeder Vergleich als Ablehnung; es gibt keinen Rueckfall auf eine
//! 64-Bit-Kennung.
//!
//! Pro Datei zusaetzlich (wie in [`safe`], aber ueber das Handle): vor dem Oeffnen
//! `symlink_metadata` des Namens (Link, Ordner, Geraet: Ablehnung), nach dem Oeffnen
//! die Pruefung des Handles (normale Datei, kein Reparse-Punkt, zum Schreiben nicht
//! mehrfach verlinkt) und der Vergleich Name gegen Handle. Ein Abschneiden geschieht
//! nie beim Oeffnen, sondern erst danach (`set_len`).
//!
//! # Bedrohungsmodell und Grenze
//!
//! Gemeint ist ein anderer Prozess desselben Benutzers (oder ein Rest eines frueheren
//! Laufs), der im Modellordner oder an den Ordnern davor Namen anlegt, ersetzt oder
//! umbenennt. Die Grenze, in diesen Worten:
//!
//! „Keine Umlenkung von App-Operationen auf unabhängige fremde Dateiobjekte.
//! Zusätzliche Namen und Inhaltsmanipulationen an App-Dateiobjekten durch
//! gleichberechtigte Prozesse desselben Benutzers sind ausgeklammert.“
//!
//! Daraus folgt:
//!
//! - Integritaet wird ausschliesslich am Lader garantiert
//!   (`nemotron::engine::OnnxBlockModel::load_verified`): er liest die Bytes selbst
//!   ueber Handles, prueft sie gegen den Katalog und baut die Sitzung aus genau diesen
//!   Bytes. Der Bereitstellungserfolg (`Fetch::Done`) ist ein Hinweis: ein
//!   abgeschlossener Versuch mit erfolgreichen Lesepruefungen, keine Zusage fuer den
//!   Plattenbestand danach und keine gemeinsame Momentaufnahme des Paars. Jeder weitere
//!   Verbraucher der Dateien (und jeder Paketexport) muss die Bytes, die er tatsaechlich
//!   verwendet, selbst verifizieren.
//! - Eine In-place-Aenderung einer App-Datei nach dem Lesen (gleicher Inode, gleiche
//!   Groesse) sieht weder `check_set` noch der Namensvergleich; der naechste Lader sieht
//!   sie am Hash. Das ist ausgeklammert, nicht verhindert.
//! - Linkzahlpruefungen (`nlink`, `nNumberOfLinks`) sind Momentpruefungen, kein
//!   Herkunftsnachweis. Vertrauensannahme: eine vorgefundene reguläre, einfach
//!   verlinkte `.partial` im Modellordner gilt als App-Datei. Ein Hardlink, der erst
//!   NACH der Pruefung auf unsere Teildatei gelegt wird, ist ein zusaetzlicher Name
//!   und ausgeklammert; geschrieben wird weiter ueber das Handle in den urspruenglichen
//!   Inode, und ein spaeter ueber den Namen untergeschobener Ersatz bleibt unveraendert
//!   (die Pruefung Name gegen Handle vor dem Umbenennen lehnt ihn ab).
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

use super::safe::{self, file_info, same_identity, Identity};

/// Was ein Name sein soll: eine normale Datei oder ein Ordner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
}

/// Ein Eintrag im Ordner, ohne Link-Folgen (`symlink_metadata`) gelesen.
struct Entry {
    is_link: bool,
    is_dir: bool,
    is_file: bool,
    len: u64,
    /// Kennung (Geraet, Inode); unter Windows liefert `symlink_metadata` keine.
    identity: Option<Identity>,
}

impl Entry {
    fn of(meta: &Metadata) -> Self {
        #[cfg(unix)]
        let identity = {
            use cap_std::fs::MetadataExt;
            Some(Identity {
                volume: meta.dev(),
                id: u128::from(meta.ino()),
            })
        };
        #[cfg(not(unix))]
        let identity = None;
        Self {
            is_link: is_link(meta),
            is_dir: meta.is_dir(),
            is_file: meta.is_file(),
            len: meta.len(),
            identity,
        }
    }

    /// Ist das, ohne Link-Folgen gelesen, ein Eintrag der verlangten Art?
    fn is(&self, kind: Kind) -> bool {
        !self.is_link
            && match kind {
                Kind::File => self.is_file,
                Kind::Dir => self.is_dir,
            }
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

/// Zeigt der Name `name` im Ordner `dir` auf genau das Objekt hinter `info` (ein
/// geoeffnetes Handle) und ist es von der Art `kind`? Der Name wird ohne Link-Folgen
/// gelesen; ein Link, die falsche Art, ein fehlender Name oder eine andere Kennung
/// ist "nein". Eine unbekannte Kennung ist nie gleich ([`same_identity`]): was sich
/// nicht belegen laesst, wird abgelehnt.
///
/// Unter Unix liefert `symlink_metadata` Geraet und Inode. Unter Windows nicht; dort
/// wird der Name ein zweites Mal geoeffnet (nur Attribute lesend, ohne einem
/// Reparse-Punkt zu folgen, alles geteilt) und die Kennung dieses Handles verglichen.
fn names_handle(dir: &Dir, name: &str, info: &safe::FileInfo, kind: Kind) -> io::Result<bool> {
    let Some(e) = entry(dir, name)? else {
        return Ok(false);
    };
    if !e.is(kind) {
        return Ok(false);
    }
    match e.identity {
        Some(_) => Ok(same_identity(e.identity, info.identity)),
        None => reopened_is(dir, name, info, kind),
    }
}

/// Ohne Kennung aus `symlink_metadata` (nur Windows) gibt es keinen Beleg: nein.
#[cfg(not(windows))]
fn reopened_is(_dir: &Dir, _name: &str, _info: &safe::FileInfo, _kind: Kind) -> io::Result<bool> {
    Ok(false)
}

/// Der Name wird zum zweiten Mal geoeffnet, ohne einem Reparse-Punkt zu folgen
/// (`FILE_FLAG_OPEN_REPARSE_POINT`, fuer Ordner mit `FILE_FLAG_BACKUP_SEMANTICS`, so
/// wie `cap-std` selbst Eintraege ohne Folgen oeffnet), und mit dem Handle `info`
/// verglichen: kein Reparse-Punkt, die verlangte Art, dieselbe 128-Bit-Kennung.
#[cfg(windows)]
fn reopened_is(dir: &Dir, name: &str, info: &safe::FileInfo, kind: Kind) -> io::Result<bool> {
    use cap_std::fs::OpenOptionsExt;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const SHARE_ALL: u32 = 0x0000_0007;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS);
    let other = match dir.open_with(name, &options) {
        Ok(f) => f.into_std(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let found = file_info(&other)?;
    let right_kind = match kind {
        Kind::File => found.is_file,
        Kind::Dir => found.is_dir,
    };
    Ok(!found.reparse && right_kind && found.same_file_as(info))
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
/// `None`, wenn er fehlt und nicht angelegt werden soll. `before_open` laeuft
/// unmittelbar vor dem Oeffnen (Test-Haken: genau hier laesst sich der Ordner tauschen).
///
/// Das geoeffnete Handle wird gegen den Namen geprueft, BEVOR die Kette damit
/// weiterlaeuft oder irgendetwas im Ordner getan wird: der Name muss (ohne Link-Folgen)
/// ein Ordner sein, der dieselbe Kennung hat wie das Handle. Wurde der Ordner im
/// Fenster durch einen Link auf einen Nachbarordner ersetzt und hat `open_dir` dem Link
/// gefolgt, ist das Handle der Nachbar, der Name ein Link: Ablehnung.
fn step(parent: &Dir, name: &str, create: bool, before_open: &dyn Fn()) -> io::Result<Option<Dir>> {
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
    if !before.is(Kind::Dir) {
        return Err(safe::refusal());
    }
    before_open();
    let dir = parent.open_dir(name).map_err(|e| {
        let foreign = entry(parent, name)
            .ok()
            .flatten()
            .is_some_and(|now| !now.is(Kind::Dir));
        if foreign {
            safe::refusal()
        } else {
            e
        }
    })?;
    let info = dir_info(&dir)?;
    let moved = before.identity.is_some() && !same_identity(before.identity, info.identity);
    if !info.is_dir || info.reparse || moved || !names_handle(parent, name, &info, Kind::Dir)? {
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
    for (i, part) in parts.iter().enumerate() {
        let parent = chain.last().expect("Kette nicht leer");
        match step(parent, part, create, &|| fire(root, Point::BeforeDir(i)))? {
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
    /// Kennung des Modellordners; `None`: unbekannt, dann besteht keine Pruefung.
    id: Option<Identity>,
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
            id: info.identity,
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
        if same_identity(info.identity, self.id) {
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
        if names_handle(self.dir(), name, &info, Kind::File)? {
            Ok(file)
        } else {
            Err(safe::refusal())
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

    /// Ist `name` noch genau die Datei, die `file` geoeffnet hat? Der Name wird ohne
    /// Link-Folgen gelesen und mit der Kennung des Handles verglichen
    /// ([`names_handle`]); eine unbekannte Kennung ist ein Nein.
    pub(crate) fn names(&self, name: &str, file: &File) -> io::Result<bool> {
        plain(name)?;
        let info = file_info(file)?;
        names_handle(self.dir(), name, &info, Kind::File)
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
    /// Unmittelbar vor dem Oeffnen der Komponente mit diesem Index der Kette (Index in
    /// `parts`, 0 = der erste Ordner unter der Datenwurzel).
    BeforeDir(usize),
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
/// jeder Name noch auf die Datei zeigt, deren Handle gehasht wurde.
///
/// `Valid` heisst: jede Datei wurde durch ihr eigenes Handle gelesen und hatte dabei die
/// verlangte Groesse und Pruefsumme, und ihr Name zeigte am Ende noch auf dieses Handle.
/// Es heisst NICHT: eine gemeinsame Momentaufnahme des Paars (die Dateien werden nacheinander
/// gelesen), und nichts ueber den Plattenbestand danach (eine In-place-Aenderung nach dem
/// Lesen, gleicher Inode, gleiche Groesse, sieht diese Funktion nicht, auch nicht der
/// Namensvergleich). Wer die Bytes verwendet, muss sie selbst verifizieren: der Lader tut
/// es, indem er mit `sink` die Bytes sammelt, die er gelesen und geprueft hat, und nur
/// aus diesen die Sitzung baut.
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
