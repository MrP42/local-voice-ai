//! Unveraenderliche Firmenrichtlinie neben der kanonisch aufgeloesten EXE (lva#882).
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

const LIMIT: u64 = 64 * 1024;

mod wirkung;
pub(crate) use wirkung::*;

const FELDNAMEN: &[&str] = &[
    "keine_programm_downloads",
    "kein_selbstupdate",
    "kein_autostart",
    "keine_tastatur_hooks",
    "keine_besprechungserkennung",
    "kein_systemton",
];

/// Erst mit Wirkstelle und Test im selben PR erweitern (Paket B).
pub const DURCHGESETZT: &[&str] = &[
    "keine_programm_downloads",
    "kein_selbstupdate",
    "kein_autostart",
    "keine_tastatur_hooks",
    "keine_besprechungserkennung",
    "kein_systemton",
];

/// Einschraenkungen werden ausschliesslich verschaerft; fehlende Felder sind aus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, specta::Type)]
#[serde(default)]
pub struct Richtlinie {
    pub keine_programm_downloads: bool,
    pub kein_selbstupdate: bool,
    pub kein_autostart: bool,
    pub keine_tastatur_hooks: bool,
    pub keine_besprechungserkennung: bool,
    pub kein_systemton: bool,
}

impl Richtlinie {
    fn streng() -> Self {
        Self {
            keine_programm_downloads: true,
            kein_selbstupdate: true,
            kein_autostart: true,
            keine_tastatur_hooks: true,
            keine_besprechungserkennung: true,
            kein_systemton: true,
        }
    }
}

/// Auch die spaetere Maschinenrichtlinie kann keine Einschraenkung aufheben.
pub fn zusammenfuehren(a: Richtlinie, b: Richtlinie) -> Richtlinie {
    Richtlinie {
        keine_programm_downloads: a.keine_programm_downloads || b.keine_programm_downloads,
        kein_selbstupdate: a.kein_selbstupdate || b.kein_selbstupdate,
        kein_autostart: a.kein_autostart || b.kein_autostart,
        keine_tastatur_hooks: a.keine_tastatur_hooks || b.keine_tastatur_hooks,
        keine_besprechungserkennung: a.keine_besprechungserkennung || b.keine_besprechungserkennung,
        kein_systemton: a.kein_systemton || b.kein_systemton,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Herkunft {
    Keine,
    Datei,
    DateiFehlerhaft,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct RichtlinienAusgabe {
    #[serde(flatten)]
    pub richtlinie: Richtlinie,
    pub herkunft: Herkunft,
    pub durchgesetzt: Vec<String>,
}

struct Snapshot {
    ausgabe: RichtlinienAusgabe,
    unbekannt: Vec<String>,
}

impl Snapshot {
    fn neu(richtlinie: Richtlinie, herkunft: Herkunft) -> Self {
        Self {
            ausgabe: RichtlinienAusgabe {
                richtlinie,
                herkunft,
                durchgesetzt: DURCHGESETZT.iter().map(|name| (*name).to_owned()).collect(),
            },
            unbekannt: Vec::new(),
        }
    }

    fn fehlerhaft() -> Self {
        Self::neu(Richtlinie::streng(), Herkunft::DateiFehlerhaft)
    }

    fn meldungen(&self) -> Vec<String> {
        let mut meldungen = Vec::new();
        if self.ausgabe.herkunft == Herkunft::DateiFehlerhaft {
            meldungen.push("Firmenrichtlinie fehlerhaft; strengster Zustand aktiv".to_owned());
        }
        for feld in &self.unbekannt {
            // JSON-Quoting verhindert auch bei fremden Feldnamen zusaetzliche Logzeilen.
            if let Ok(name) = serde_json::to_string(feld) {
                meldungen.push(format!("Firmenrichtlinie: unbekanntes Feld {}", name));
            }
        }
        meldungen
    }

    fn protokollieren(&self) {
        for meldung in self.meldungen() {
            log::warn!("{}", meldung);
        }
    }
}

static SNAPSHOT: OnceLock<Snapshot> = OnceLock::new();
static PROTOKOLL: Once = Once::new();

/// Vor jeder App-Initialisierung lesen; danach bleiben selbst Dateiaenderungen wirkungslos.
pub fn initialisieren() {
    snapshot(&SNAPSHOT, std::env::current_exe);
}

/// Unveraenderlicher Produktivstand; Tests pruefen Entscheidungen mit einem eigenen Wert.
pub(crate) fn aktuell() -> &'static Richtlinie {
    &snapshot(&SNAPSHOT, std::env::current_exe).ausgabe.richtlinie
}

fn snapshot(speicher: &OnceLock<Snapshot>, exe: impl FnOnce() -> io::Result<PathBuf>) -> &Snapshot {
    speicher.get_or_init(|| aus_exe(exe()))
}

/// Nach Einrichtung des App-Loggers die beim fruehen Lesen gesammelten Befunde melden.
pub fn protokollieren() {
    initialisieren();
    PROTOKOLL.call_once(|| {
        if let Some(snapshot) = SNAPSHOT.get() {
            snapshot.protokollieren();
        }
    });
}

#[tauri::command]
#[specta::specta]
pub fn richtlinie_lesen() -> RichtlinienAusgabe {
    snapshot(&SNAPSHOT, std::env::current_exe)
        .ausgabe
        .clone()
}

/// Derselbe Snapshot und dasselbe JSON fuer CLI und Oberflaeche.
pub fn zeigen(out: &mut impl Write) -> io::Result<()> {
    let stand = snapshot(&SNAPSHOT, std::env::current_exe);
    // Dieser fruehe CLI-Weg hat keinen App-Logger; Befunde ausschliesslich nach stderr.
    for meldung in stand.meldungen() {
        let _ = writeln!(io::stderr().lock(), "{}", meldung);
    }
    schreiben(&stand.ausgabe, out)
}

fn schreiben(ausgabe: &RichtlinienAusgabe, out: &mut impl Write) -> io::Result<()> {
    serde_json::to_writer(&mut *out, ausgabe)?;
    out.write_all(b"\n")
}

fn aus_exe(exe: io::Result<PathBuf>) -> Snapshot {
    let Ok(exe) = exe.and_then(fs::canonicalize) else {
        return Snapshot::fehlerhaft();
    };
    let Some(ordner) = exe.parent() else {
        return Snapshot::fehlerhaft();
    };
    aus_datei(&ordner.join("richtlinie.json"))
}

fn aus_datei(pfad: &Path) -> Snapshot {
    // Nur ein fehlender Name gilt als fehlendes Profil; spaetere Oeffnungs-/Lesefehler nicht.
    match fs::symlink_metadata(pfad) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Snapshot::neu(Richtlinie::default(), Herkunft::Keine);
        }
        Err(_) => return Snapshot::fehlerhaft(),
        Ok(meta) if !regulaer(&meta) => return Snapshot::fehlerhaft(),
        Ok(_) => {}
    }
    let mut optionen = OpenOptions::new();
    optionen.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: auch bei Austausch seit symlink_metadata nicht folgen.
        optionen.custom_flags(0x0020_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        optionen.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let Ok(datei) = optionen.open(pfad) else {
        return Snapshot::fehlerhaft();
    };
    let Ok(meta) = datei.metadata() else {
        return Snapshot::fehlerhaft();
    };
    if !regulaer(&meta) {
        return Snapshot::fehlerhaft();
    }
    // Metadaten und begrenzter Leser beziehen sich auf dasselbe Handle.
    aus_leser(datei, meta.len())
}

fn regulaer(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false; // FILE_ATTRIBUTE_REPARSE_POINT
        }
    }
    meta.is_file() && !meta.file_type().is_symlink()
}

fn aus_leser(leser: impl Read, groesse: u64) -> Snapshot {
    if groesse > LIMIT {
        return Snapshot::fehlerhaft();
    }
    let mut bytes = Vec::new();
    if leser.take(LIMIT + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > LIMIT {
        return Snapshot::fehlerhaft();
    }
    let Ok(richtlinie) = serde_json::from_slice::<Richtlinie>(&bytes) else {
        return Snapshot::fehlerhaft();
    };
    let Ok(serde_json::Value::Object(felder)) = serde_json::from_slice(&bytes) else {
        return Snapshot::fehlerhaft();
    };
    let unbekannt = felder
        .keys()
        .filter(|name| !FELDNAMEN.contains(&name.as_str()))
        .cloned()
        .collect();
    let mut stand = Snapshot::neu(richtlinie, Herkunft::Datei);
    stand.unbekannt = unbekannt;
    stand
}

#[cfg(test)]
mod tests;
