//! Speicher des geprueften Verzeichnisses je Geraet: nur Verzeichnis, nie Geheimnisse.
//!
//! Schreiben erfolgt ueber Hilfsdatei, fsync und Umbenennen. Vergleich-und-Tausch ist
//! nur prozessweit atomar: die Sperre gilt von der Pruefung bis nach dem Umbenennen.
//! Es gilt ein Schreiber je Verzeichnis; fremde Prozesse kennen diese Sperre nicht.
//! Ein Austausch des Verzeichnisses zwischen Pruefung und Zugriff durch einen anderen
//! Prozess liegt ausserhalb dieser Grenze. Hier gibt es kein Logging.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

use super::kodierung::{lies_json, Wert};
use super::verzeichnis::{kodiertes_verzeichnis, pruefe_verzeichnis};
use crate::voice_call::assistenten::{hilfsdatei_anlegen, ist_verlinkt, ZAEHLER};

pub const VERZEICHNIS: &str = "schnittstellen";
pub const DATEI: &str = "verzeichnis.json";
pub const MAX_BYTES: usize = 256 * 1024;

static SPERRE: Mutex<()> = Mutex::new(());

#[derive(Debug, PartialEq)]
pub enum SpeicherFehler {
    Ungueltig(Wert),
    ZuGross,
    Konflikt,
    Beschaedigt,
    UnsicherePfad,
    Lesen,
    Schreiben,
}

pub struct Gelesen {
    pub bytes: Vec<u8>,
    pub verzeichnis: Wert,
}

#[derive(Debug, Clone)]
pub struct VerzeichnisSpeicher {
    verzeichnis: PathBuf,
}

fn wurzel_pruefen(verzeichnis: &Path) -> Result<(), SpeicherFehler> {
    match fs::symlink_metadata(verzeichnis) {
        Ok(meta) if ist_verlinkt(&meta) => Err(SpeicherFehler::UnsicherePfad),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(SpeicherFehler::Lesen),
    }
}

impl VerzeichnisSpeicher {
    pub fn neu(verzeichnis: PathBuf) -> Self {
        Self { verzeichnis }
    }

    pub fn lesen(&self) -> Result<Option<Gelesen>, SpeicherFehler> {
        let _sperre = SPERRE.lock().unwrap_or_else(|e| e.into_inner());
        self.lesen_unter_sperre()
    }

    fn lesen_unter_sperre(&self) -> Result<Option<Gelesen>, SpeicherFehler> {
        wurzel_pruefen(&self.verzeichnis)?;
        let pfad = self.verzeichnis.join(DATEI);
        let meta = match fs::symlink_metadata(&pfad) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SpeicherFehler::Lesen),
        };
        if ist_verlinkt(&meta) || !meta.is_file() {
            return Err(SpeicherFehler::UnsicherePfad);
        }
        if meta.len() > MAX_BYTES as u64 {
            return Err(SpeicherFehler::ZuGross);
        }
        let mut optionen = OpenOptions::new();
        optionen.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            optionen.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let datei = optionen.open(pfad).map_err(|_| SpeicherFehler::Lesen)?;
        let meta = datei.metadata().map_err(|_| SpeicherFehler::Lesen)?;
        if ist_verlinkt(&meta) || !meta.is_file() {
            return Err(SpeicherFehler::UnsicherePfad);
        }
        if meta.len() > MAX_BYTES as u64 {
            return Err(SpeicherFehler::ZuGross);
        }
        // Auch eine nach der Metadatenpruefung wachsende Datei bleibt begrenzt.
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        datei
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SpeicherFehler::Lesen)?;
        if bytes.len() > MAX_BYTES {
            return Err(SpeicherFehler::ZuGross);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| SpeicherFehler::Beschaedigt)?;
        let wert = lies_json(text).map_err(|_| SpeicherFehler::Beschaedigt)?;
        let verzeichnis = pruefe_verzeichnis(&wert).map_err(|_| SpeicherFehler::Beschaedigt)?;
        Ok(Some(Gelesen { bytes, verzeichnis }))
    }

    pub fn speichern(
        &self,
        neu: &Wert,
        erwartet: Option<&[u8]>,
    ) -> Result<Vec<u8>, SpeicherFehler> {
        let _sperre = SPERRE.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = kodiertes_verzeichnis(neu)
            .map_err(SpeicherFehler::Ungueltig)?
            .into_bytes();
        if bytes.len() > MAX_BYTES {
            return Err(SpeicherFehler::ZuGross);
        }
        let alt = self.lesen_unter_sperre().map_err(|e| match e {
            SpeicherFehler::Lesen => SpeicherFehler::Beschaedigt,
            e => e,
        })?;
        match alt {
            Some(alt) if erwartet != Some(alt.bytes.as_slice()) => {
                return Err(SpeicherFehler::Konflikt);
            }
            None if erwartet.is_some() => return Err(SpeicherFehler::Konflikt),
            _ => {}
        }
        schreibe_atomar_mit(
            &self.verzeichnis,
            DATEI,
            &bytes,
            &|d, b| d.write_all(b),
            &ZAEHLER,
        )?;
        Ok(bytes)
    }
}

// Fuer Gegenproben im benachbarten Testmodul sichtbar; Aufrufer haelt die Sperre.
pub(super) fn schreibe_atomar_mit(
    verzeichnis: &Path,
    name: &str,
    bytes: &[u8],
    schreiber: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
    zaehler: &AtomicU64,
) -> Result<(), SpeicherFehler> {
    wurzel_pruefen(verzeichnis)?;
    fs::create_dir_all(verzeichnis).map_err(|_| SpeicherFehler::Schreiben)?;
    // Erst nach dem Anlegen steht fest, wo die Hilfsdatei entstehen wuerde.
    wurzel_pruefen(verzeichnis)?;
    let (hilfsdatei, mut datei) = hilfsdatei_anlegen(verzeichnis, ".lva-schn", zaehler)
        .map_err(|_| SpeicherFehler::Schreiben)?;
    let ergebnis = (|| -> io::Result<()> {
        schreiber(&mut datei, bytes)?;
        datei.sync_all()?;
        drop(datei);
        fs::rename(&hilfsdatei, verzeichnis.join(name))
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&hilfsdatei);
        return Err(SpeicherFehler::Schreiben);
    }
    #[cfg(unix)]
    File::open(verzeichnis)
        .and_then(|d| d.sync_all())
        .map_err(|_| SpeicherFehler::Schreiben)?;
    Ok(())
}
