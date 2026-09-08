//! Herkunft einer erzeugten Aufnahme.
//!
//! Eine exportierte Datei allein sagt nicht, aus welchem Text sie entstand
//! und wer sie gesprochen hat. Wer ein Hoerspiel spaeter nachbessern will,
//! braucht genau das: den Text zurueck in den Editor holen, die eine falsche
//! Zeile aendern, erneut erzeugen — die unveraenderten Saetze kommen dann aus
//! dem Satz-Cache, und nur die geaenderten gehen durch die Engine.
//!
//! Abgelegt wird das als Beileger `<datei>.json` neben der Aufnahme, nicht in
//! einer Datenbank: die Aufnahme kann kopiert, verschoben oder geloescht
//! werden, ohne dass ein zweiter Ort davon wissen muss. Was fehlt, fehlt —
//! ein Beileger ohne Aufnahme ist wertlos, eine Aufnahme ohne Beileger bleibt
//! abspielbar.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

/// Endung des Beilegers. Bewusst an den vollen Dateinamen angehaengt
/// (`stueck.wav.json`), damit `stueck.wav` und `stueck.mp3` sich nicht
/// gegenseitig ueberschreiben.
const NOTE_SUFFIX: &str = ".json";

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AudioNote {
    /// Der Text, aus dem die Aufnahme entstand — vollstaendig, damit er
    /// zurueck in den Editor kann.
    pub text: String,
    /// Kennung der verwendeten Stimme; `None` ist die Standardstimme.
    pub voice: Option<String>,
    /// Seed der Standardstimme zum Zeitpunkt der Aufnahme.
    pub seed: i64,
    /// Zeitpunkt in Millisekunden seit dem 01.01.1970.
    pub created_ms: i64,
}

/// Pfad des Beilegers zu einer Aufnahme.
pub fn note_path(audio: &Path) -> PathBuf {
    let mut name = audio.as_os_str().to_os_string();
    name.push(NOTE_SUFFIX);
    PathBuf::from(name)
}

/// Ist dieser Dateiname ein Beileger? Die Dateileiste blendet sie aus — sie
/// gehoeren zur Aufnahme daneben und nicht in die Liste.
pub fn is_note(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(NOTE_SUFFIX)
        && [".wav", ".mp3", ".opus", ".flac", ".ogg", ".m4a"]
            .iter()
            .any(|ext| lower.trim_end_matches(NOTE_SUFFIX).ends_with(ext))
}

/// Schreibt den Beileger. Ein Fehlschlag ist kein Grund, den Export als
/// gescheitert zu melden: die Aufnahme selbst liegt dann bereits fertig da.
pub fn write(audio: &Path, note: &AudioNote) {
    let path = note_path(audio);
    match serde_json::to_vec_pretty(note) {
        Ok(bytes) => {
            if let Err(error) = std::fs::write(&path, bytes) {
                log::warn!("Herkunft nicht abgelegt ({}): {error}", path.display());
            }
        }
        Err(error) => log::warn!("Herkunft nicht serialisierbar: {error}"),
    }
}

/// Liest den Beileger einer Aufnahme, falls es einen gibt.
pub fn read(audio: &Path) -> Option<AudioNote> {
    let bytes = std::fs::read(note_path(audio)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Jetzt, in Millisekunden seit dem 01.01.1970.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn der_beileger_haengt_an_der_vollen_dateiendung() {
        // Sonst teilten sich stueck.wav und stueck.mp3 einen Beileger, und
        // der zweite Export ueberschriebe die Herkunft des ersten.
        assert_eq!(
            note_path(Path::new("C:/p/stueck.wav")),
            PathBuf::from("C:/p/stueck.wav.json")
        );
        assert_ne!(
            note_path(Path::new("C:/p/stueck.wav")),
            note_path(Path::new("C:/p/stueck.mp3"))
        );
    }

    #[test]
    fn nur_beileger_von_aufnahmen_gelten_als_beileger() {
        assert!(is_note("stueck.wav.json"));
        assert!(is_note("Stueck.MP3.JSON"));
        // Eine gewoehnliche JSON-Datei im Projektordner bleibt sichtbar.
        assert!(!is_note("notizen.json"));
        assert!(!is_note("stueck.wav"));
    }

    #[test]
    fn herkunft_ueberlebt_den_weg_durch_die_platte() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("stueck.wav");
        let note = AudioNote {
            text: "Erste Zeile.\nZweite Zeile.".to_string(),
            voice: Some("erzaehlerin".to_string()),
            seed: 42,
            created_ms: 1_757_000_000_000,
        };
        write(&audio, &note);
        assert_eq!(read(&audio), Some(note));
    }

    #[test]
    fn ohne_beileger_kommt_nichts_zurueck() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("fremd.wav")), None);
    }
}
