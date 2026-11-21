//! Gemeinsame Zeittypen und Metadaten einer Medienquelle.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum ZeitGuete {
    Wort,
    Satz,
    Grob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wort {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Alle Zeiten sind absolut. Ein Quellenadapter rechnet lokale Offsets einmalig um.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub woerter: Option<Vec<Wort>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedienQuelle {
    pub id: String,
    pub dauer_ms: u64,
    pub hat_ton: bool,
    pub hat_video: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZeitachsenFehler {
    ProbeNichtGefunden,
    DateiUnlesbar,
    ProbeFehlgeschlagen,
    ProbeAusgabeUnlesbar,
    KeineDauer,
    Zeitueberschreitung,
}

impl std::fmt::Display for ZeitachsenFehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nur die Fehlerart ausgeben, keine Pfade oder Medieninhalte.
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ZeitachsenFehler {}

/// SHA-256 mit festem Lesepuffer, auch fuer grosse Mediendateien.
pub fn sha256_datei(pfad: &Path) -> Result<String, ZeitachsenFehler> {
    let mut datei = File::open(pfad).map_err(|_| ZeitachsenFehler::DateiUnlesbar)?;
    let mut hasher = Sha256::new();
    let mut puffer = [0u8; 64 * 1024];
    loop {
        let gelesen = datei
            .read(&mut puffer)
            .map_err(|_| ZeitachsenFehler::DateiUnlesbar)?;
        if gelesen == 0 {
            break;
        }
        hasher.update(&puffer[..gelesen]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Metadaten aus der JSON-Ausgabe von ffprobe, ohne Prozess oder Dateizugriff.
pub fn quelle_aus_ffprobe(
    id: String,
    ffprobe_json: &str,
) -> Result<MedienQuelle, ZeitachsenFehler> {
    let probe: serde_json::Value =
        serde_json::from_str(ffprobe_json).map_err(|_| ZeitachsenFehler::ProbeAusgabeUnlesbar)?;
    let dauer = probe["format"]["duration"]
        .as_str()
        .and_then(|wert| wert.parse::<f64>().ok())
        .ok_or(ZeitachsenFehler::KeineDauer)?;
    let dauer_ms = (dauer * 1000.0).round();
    // Die obere Grenze ist exklusiv: u64::MAX wird als f64 zu 2^64 gerundet.
    if !dauer.is_finite() || dauer < 0.0 || !dauer_ms.is_finite() || dauer_ms >= u64::MAX as f64 {
        return Err(ZeitachsenFehler::KeineDauer);
    }
    let mut quelle = MedienQuelle {
        id,
        dauer_ms: dauer_ms as u64,
        hat_ton: false,
        hat_video: false,
    };
    if let Some(streams) = probe["streams"].as_array() {
        for stream in streams {
            match stream["codec_type"].as_str() {
                Some("audio") => quelle.hat_ton = true,
                Some("video") if stream["disposition"]["attached_pic"].as_u64() != Some(1) => {
                    quelle.hat_video = true;
                }
                _ => {}
            }
        }
    }
    Ok(quelle)
}

/// Datei identifizieren und ffprobe mit begrenzter Ausgabe auswerten.
pub fn pruefe_medienquelle(pfad: &Path) -> Result<MedienQuelle, ZeitachsenFehler> {
    pruefe_medienquelle_mit(
        OsStr::new("ffprobe"),
        |cmd, pfad| {
            cmd.args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "-i",
            ])
            .arg(pfad);
        },
        pfad,
        Duration::from_secs(20),
    )
}

/// Austauschbares Programm und Frist fuer die begrenzte Metadatenpruefung.
pub fn pruefe_medienquelle_mit(
    programm: &OsStr,
    args: impl FnOnce(&mut Command, &Path),
    pfad: &Path,
    frist: Duration,
) -> Result<MedienQuelle, ZeitachsenFehler> {
    const MAX_AUSGABE: usize = 1024 * 1024;

    let id = sha256_datei(pfad)?;
    let mut cmd = Command::new(programm);
    args(&mut cmd, pfad);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|fehler| {
        if fehler.kind() == std::io::ErrorKind::NotFound {
            ZeitachsenFehler::ProbeNichtGefunden
        } else {
            ZeitachsenFehler::ProbeFehlgeschlagen
        }
    })?;
    let beginn = Instant::now();
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ZeitachsenFehler::ProbeAusgabeUnlesbar);
        }
    };
    let (sender, empfaenger) = mpsc::channel();
    thread::spawn(move || {
        let mut ausgabe = Vec::new();
        // Ein zusaetzliches Byte erkennt Ueberlaenge, ohne unbegrenzt zu lesen.
        let gelesen = stdout
            .take((MAX_AUSGABE + 1) as u64)
            .read_to_end(&mut ausgabe);
        let ergebnis = if gelesen.is_err() || ausgabe.len() > MAX_AUSGABE {
            Err(ZeitachsenFehler::ProbeAusgabeUnlesbar)
        } else {
            Ok(ausgabe)
        };
        let _ = sender.send(ergebnis);
    });
    let mut status = None;
    let mut ausgabe = None;
    let ergebnis = loop {
        if beginn.elapsed() >= frist {
            break Err(ZeitachsenFehler::Zeitueberschreitung);
        }
        if ausgabe.is_none() {
            match empfaenger.try_recv() {
                Ok(Ok(bytes)) => ausgabe = Some(bytes),
                Ok(Err(fehler)) => break Err(fehler),
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(ZeitachsenFehler::ProbeAusgabeUnlesbar);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(ende)) if !ende.success() => {
                    break Err(ZeitachsenFehler::ProbeFehlgeschlagen);
                }
                Ok(ende) => status = ende,
                Err(_) => break Err(ZeitachsenFehler::ProbeFehlgeschlagen),
            }
        }
        if status.is_some() {
            if let Some(bytes) = ausgabe.take() {
                break Ok(bytes);
            }
        }
        thread::sleep(Duration::from_millis(20));
    };
    let ausgabe = match ergebnis {
        Ok(ausgabe) => ausgabe,
        Err(fehler) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fehler);
        }
    };
    let json = std::str::from_utf8(&ausgabe).map_err(|_| ZeitachsenFehler::ProbeAusgabeUnlesbar)?;
    quelle_aus_ffprobe(id, json)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bildmarke {
    pub zeit_ms: u64,
    pub bild_sha: String,
    pub ocr: Option<String>,
    pub beschreibung: Option<String>,
    pub ausgelassen: bool,
}

pub trait BildQuelle {
    fn bilder(&mut self, von_ms: u64, bis_ms: u64) -> Result<Vec<Bildmarke>, ZeitachsenFehler>;
}

pub struct LeereBildQuelle;

impl BildQuelle for LeereBildQuelle {
    fn bilder(&mut self, _von_ms: u64, _bis_ms: u64) -> Result<Vec<Bildmarke>, ZeitachsenFehler> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn probe(json: &str) -> Result<MedienQuelle, ZeitachsenFehler> {
        quelle_aus_ffprobe("a".repeat(64), json)
    }

    #[test]
    fn haengende_probe_wird_innerhalb_der_frist_beendet() {
        let datei = tempfile::NamedTempFile::new().unwrap();
        #[cfg(windows)]
        let programm = OsStr::new("cmd");
        #[cfg(not(windows))]
        let programm = OsStr::new("sleep");
        let beginn = Instant::now();
        let ergebnis = pruefe_medienquelle_mit(
            programm,
            |cmd, _| {
                #[cfg(windows)]
                cmd.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
                #[cfg(not(windows))]
                cmd.arg("30");
            },
            datei.path(),
            Duration::from_millis(200),
        );
        assert!(beginn.elapsed() < Duration::from_secs(5));
        assert_eq!(ergebnis, Err(ZeitachsenFehler::Zeitueberschreitung));
        assert_eq!(
            ZeitachsenFehler::Zeitueberschreitung.to_string(),
            "Zeitueberschreitung"
        );
    }

    #[test]
    fn audio_und_video_werden_erkannt() {
        let quelle = probe(concat!(
            r#"{"format":{"duration":"2.5"},"streams":["#,
            r#"{"codec_type":"audio"},{"codec_type":"video"}]}"#,
        ))
        .unwrap();
        assert_eq!(quelle.id, "a".repeat(64));
        assert_eq!(quelle.dauer_ms, 2500);
        assert!(quelle.hat_ton);
        assert!(quelle.hat_video);
    }

    #[test]
    fn nur_audio_hat_kein_video() {
        let quelle =
            probe(r#"{"format":{"duration":"1"},"streams":[{"codec_type":"audio"}]}"#).unwrap();
        assert!(quelle.hat_ton);
        assert!(!quelle.hat_video);
    }

    #[test]
    fn nur_video_hat_keinen_ton() {
        let quelle =
            probe(r#"{"format":{"duration":"1"},"streams":[{"codec_type":"video"}]}"#).unwrap();
        assert!(!quelle.hat_ton);
        assert!(quelle.hat_video);
    }

    #[test]
    fn cover_zaehlt_nicht_als_video() {
        let quelle = probe(concat!(
            r#"{"format":{"duration":"1"},"streams":[{"codec_type":"audio"},"#,
            r#"{"codec_type":"video","disposition":{"attached_pic":1}}]}"#,
        ))
        .unwrap();
        assert!(quelle.hat_ton);
        assert!(!quelle.hat_video);
        let quelle = probe(concat!(
            r#"{"format":{"duration":"1"},"streams":["#,
            r#"{"codec_type":"video","disposition":{"attached_pic":1}},"#,
            r#"{"codec_type":"video","disposition":{"attached_pic":0}}]}"#,
        ))
        .unwrap();
        assert!(quelle.hat_video);
    }

    #[test]
    fn fehlende_dauer_wird_abgelehnt() {
        for json in [r#"{"streams":[]}"#, r#"{"format":{},"streams":[]}"#] {
            assert_eq!(probe(json), Err(ZeitachsenFehler::KeineDauer));
        }
    }

    #[test]
    fn unlesbare_und_nicht_endliche_dauer_wird_abgelehnt() {
        for dauer in [
            "N/A",
            "NaN",
            "inf",
            "-inf",
            "1e309",
            "1e308",
            "18446744073709552",
        ] {
            let json = serde_json::json!({"format": {"duration": dauer}});
            assert_eq!(probe(&json.to_string()), Err(ZeitachsenFehler::KeineDauer));
        }
        assert_eq!(
            probe(r#"{"format":{"duration":1}}"#),
            Err(ZeitachsenFehler::KeineDauer)
        );
    }

    #[test]
    fn dauer_wird_auf_millisekunden_gerundet() {
        for (dauer, erwartet) in [("1.2345", 1235), ("1.2344", 1234), ("0", 0)] {
            let json = serde_json::json!({"format": {"duration": dauer}});
            assert_eq!(probe(&json.to_string()).unwrap().dauer_ms, erwartet);
        }
    }

    #[test]
    fn negative_dauer_wird_auch_vor_der_rundung_abgelehnt() {
        for dauer in ["-1", "-0.0001"] {
            let json = serde_json::json!({"format": {"duration": dauer}});
            assert_eq!(probe(&json.to_string()), Err(ZeitachsenFehler::KeineDauer));
        }
    }

    #[test]
    fn kaputtes_json_wird_abgelehnt() {
        assert_eq!(probe("{"), Err(ZeitachsenFehler::ProbeAusgabeUnlesbar));
    }

    #[test]
    fn leere_streams_haben_weder_ton_noch_video() {
        let quelle = probe(r#"{"format":{"duration":"1"},"streams":[]}"#).unwrap();
        assert!(!quelle.hat_ton);
        assert!(!quelle.hat_video);
    }

    #[test]
    fn datei_hash_entspricht_dem_bekannten_abc_hash() {
        let mut datei = tempfile::NamedTempFile::new().unwrap();
        datei.write_all(b"abc").unwrap();
        assert_eq!(
            sha256_datei(datei.path()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            sha256_datei(&dir.path().join("fehlt")),
            Err(ZeitachsenFehler::DateiUnlesbar)
        );
    }

    #[test]
    fn datei_hash_liest_mehrere_bloecke_einschliesslich_rest() {
        let bytes: Vec<u8> = (0..(2 * 64 * 1024 + 17)).map(|i| (i % 251) as u8).collect();
        let mut datei = tempfile::NamedTempFile::new().unwrap();
        datei.write_all(&bytes).unwrap();
        assert_eq!(
            sha256_datei(datei.path()).unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
    }

    #[test]
    fn leere_bildquelle_liefert_fuer_jeden_bereich_leer() {
        let mut quelle = LeereBildQuelle;
        for (von, bis) in [(0, 0), (10, 100), (100, 10), (0, u64::MAX)] {
            assert_eq!(quelle.bilder(von, bis), Ok(vec![]));
        }
    }

    #[test]
    fn zeittypen_behalten_serde_format_und_reexport() {
        let wort: crate::media_translation::Wort = Wort {
            start_ms: 100,
            end_ms: 200,
            text: "Hallo".to_string(),
        };
        let segment: crate::media_translation::Segment = Segment {
            start_ms: 100,
            end_ms: 300,
            text: "Hallo!".to_string(),
            woerter: Some(vec![wort]),
        };
        let erwartet = serde_json::json!({
            "start_ms": 100, "end_ms": 300, "text": "Hallo!",
            "woerter": [{"start_ms": 100, "end_ms": 200, "text": "Hallo"}]
        });
        assert_eq!(serde_json::to_value(&segment).unwrap(), erwartet);
        assert_eq!(
            serde_json::from_value::<Segment>(erwartet).unwrap(),
            segment
        );
        for (guete, name) in [
            (ZeitGuete::Wort, "Wort"),
            (ZeitGuete::Satz, "Satz"),
            (ZeitGuete::Grob, "Grob"),
        ] {
            let reexport: crate::media_translation::ZeitGuete = guete;
            let json = serde_json::Value::String(name.to_string());
            assert_eq!(serde_json::to_value(reexport).unwrap(), json);
            assert_eq!(serde_json::from_value::<ZeitGuete>(json).unwrap(), guete);
        }
    }
}
