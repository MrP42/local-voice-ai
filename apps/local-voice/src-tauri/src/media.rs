//! Dokumente und Medien als Eingabe (statt nur Texteingabe/WAV).
//!
//! - Text aus Dokumenten: TXT/MD direkt, PDF über pdf-extract, DOCX über
//!   zip + Tag-Stripping von word/document.xml.
//! - Beliebige Audio-/Videoformate (mp3, m4a, mp4, mov, mkv, …) werden über
//!   das lokale ffmpeg zu Mono-WAV dekodiert; ohne ffmpeg gibt es eine
//!   sprechende Fehlermeldung statt eines stillen Scheiterns.

use std::path::{Path, PathBuf};

pub const DOCUMENT_EXTENSIONS: [&str; 4] = ["txt", "md", "pdf", "docx"];
pub const MEDIA_EXTENSIONS: [&str; 13] = [
    "wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wma", "mp4", "mov", "mkv", "webm", "avi",
];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

/// XML-Textinhalt aus DOCX-Body-XML: Absätze werden Zeilenumbrüche, Tags
/// fallen weg, die fünf XML-Entities werden aufgelöst.
pub fn docx_xml_to_text(xml: &str) -> String {
    let with_breaks = xml
        .replace("</w:p>", "\n")
        .replace("<w:tab/>", "\t")
        .replace("<w:br/>", "\n");
    let re = regex::Regex::new("<[^>]+>").expect("static regex");
    let stripped = re.replace_all(&with_breaks, "");
    stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

fn extract_docx(path: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("not a DOCX (zip) file: {e}"))?;
    let mut doc = archive
        .by_name("word/document.xml")
        .map_err(|_| "not a DOCX file (word/document.xml missing)".to_string())?;
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut doc, &mut xml).map_err(|e| e.to_string())?;
    Ok(docx_xml_to_text(&xml))
}

/// Text aus einem Dokument extrahieren. Der Rückgabetext ist unverändert —
/// Kürzung auf `tts_max_chars` passiert erst beim Sprechen.
pub fn extract_document_text(path: &Path) -> Result<String, String> {
    let text = match extension_of(path).as_str() {
        "txt" | "md" => {
            let bytes =
                std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                .to_string()
        }
        "pdf" => pdf_extract::extract_text(path)
            .map_err(|e| format!("PDF text extraction failed: {e}"))?,
        "docx" => extract_docx(path)?,
        other => {
            return Err(format!(
                "Nicht unterstütztes Dokumentformat '.{other}' — unterstützt: {}",
                DOCUMENT_EXTENSIONS.join(", ")
            ))
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Das Dokument enthält keinen extrahierbaren Text".into());
    }
    Ok(trimmed.to_string())
}

/// Sichtbaren Text aus HTML ziehen — Artikel-Niveau (kein Browser-Ersatz):
/// Script/Style/Head raus, Block-Enden werden Zeilenumbrüche, Tags weg,
/// gängige Entities aufgelöst, Leerraum verdichtet.
pub fn html_to_text(html: &str) -> String {
    let no_hidden = regex::Regex::new(r"(?is)<(script|style|noscript|head|svg|nav|footer)\b.*?</(script|style|noscript|head|svg|nav|footer)>")
        .expect("static regex")
        .replace_all(html, " ");
    let with_breaks =
        regex::Regex::new(r"(?i)<(br\s*/?|/p|/div|/h[1-6]|/li|/tr|/section|/article)[^>]*>")
            .expect("static regex")
            .replace_all(&no_hidden, "\n");
    let stripped = regex::Regex::new("<[^>]+>")
        .expect("static regex")
        .replace_all(&with_breaks, " ");
    let decoded = stripped
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    // Leerraum verdichten: Zeilen trimmen, leere Mehrfachzeilen zusammenfassen.
    let mut out: Vec<String> = Vec::new();
    for line in decoded.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
        } else {
            out.push(collapsed);
        }
    }
    out.join("\n").trim().to_string()
}

/// Webseite laden und als Fließtext zurückgeben (für die KI-Zusammenfassung).
pub async fn extract_url_text(url: &str) -> Result<String, String> {
    let full_url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let client = reqwest::Client::builder()
        .user_agent("LocalVoiceAI/0.1 (+local reader)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&full_url)
        .send()
        .await
        .map_err(|e| format!("Seite nicht erreichbar: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Seite antwortete mit {}", response.status()));
    }
    let html = response.text().await.map_err(|e| e.to_string())?;
    let text = html_to_text(&html);
    if text.chars().count() < 200 {
        return Err(
            "Die Seite lieferte kaum lesbaren Text (evtl. JavaScript-Seite oder Paywall)".into(),
        );
    }
    Ok(text)
}

/// Beliebiges Audio/Video über ffmpeg zu Mono-WAV mit gegebener Samplerate.
pub fn decode_media_to_wav(input: &Path, out_wav: &Path, sample_rate: u32) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-i",
        &input.to_string_lossy(),
        "-vn",
        "-ac",
        "1",
        "-ar",
        &sample_rate.to_string(),
        &out_wav.to_string_lossy(),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| {
        format!("ffmpeg nicht gefunden ({e}) — Installation z. B.: winget install ffmpeg")
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        return Err(format!(
            "ffmpeg konnte {} nicht dekodieren: {tail}",
            input.display()
        ));
    }
    if !out_wav.exists() {
        return Err("ffmpeg lieferte keine Ausgabedatei".into());
    }
    Ok(())
}

/// Fehlertext, wenn eine Dekodierung vom Nutzer abgebrochen wurde (P8a).
pub const DECODE_CANCELLED: &str = "decode_cancelled";

/// Wie ein abbrechbarer Kindprozess endete.
#[derive(Debug, PartialEq, Eq)]
pub enum ChildEnd {
    /// Der Prozess ist von selbst zu Ende gegangen.
    Finished {
        success: bool,
        /// Die letzten Zeilen der Fehlerausgabe (hoechstens ~4 KB gelesen).
        stderr_tail: String,
    },
    /// `cancel` wurde gesetzt: der Prozess wurde beendet (nie gestartet, wenn
    /// das Flag schon vor dem Start stand).
    Cancelled,
}

/// Startet `cmd`, wartet darauf und beendet den Prozess, sobald `cancel`
/// gesetzt wird. Beenden geschieht nur ueber DIESEN Prozess (Handle, also seine
/// PID), nie ueber den Programmnamen; unter Windows haengt der Prozess dazu in
/// einem Job-Objekt (`process_guard`, KILL_ON_JOB_CLOSE): auch eventuelle
/// Kindprozesse gehen mit, nichts bleibt verwaist. `pid_out` bekommt die PID
/// (Tests, Diagnose). Stdin und Stdout sind zu; die Fehlerausgabe wird
/// nebenher gelesen (sonst blockiert ein voller Pipe-Puffer den Prozess).
pub fn run_child_cancellable(
    mut cmd: std::process::Command,
    cancel: &std::sync::atomic::AtomicBool,
    pid_out: Option<&std::sync::atomic::AtomicU32>,
) -> std::io::Result<ChildEnd> {
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::atomic::Ordering;

    if cancel.load(Ordering::Acquire) {
        return Ok(ChildEnd::Cancelled);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    if let Some(out) = pid_out {
        out.store(child.id(), Ordering::Release);
    }
    // Ohne Speicherdeckel (Dekodieren braucht wenig), mit CPU-Deckel,
    // niedriger Prioritaet und KILL_ON_JOB_CLOSE.
    #[cfg(windows)]
    let _job = crate::process_guard::ProcessGuard::attach(
        &child,
        None,
        crate::process_guard::CPU_CAP_PERCENT,
    );

    let reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            const KEEP: usize = 4096;
            let mut tail: Vec<u8> = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > KEEP {
                    tail.drain(..tail.len() - KEEP);
                }
            }
            String::from_utf8_lossy(&tail).into_owned()
        })
    });
    let stderr_of = |reader: Option<std::thread::JoinHandle<String>>| {
        reader.and_then(|r| r.join().ok()).unwrap_or_default()
    };

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(ChildEnd::Finished {
                    success: status.success(),
                    stderr_tail: stderr_of(reader),
                });
            }
            Ok(None) => {
                if cancel.load(Ordering::Acquire) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stderr_of(reader);
                    return Ok(ChildEnd::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_of(reader);
                return Err(e);
            }
        }
    }
}

/// Wie [`decode_media_to_wav`], aber abbrechbar: `cancel` beendet ffmpeg (und
/// entfernt die halbe Ausgabedatei); der Fehler ist dann [`DECODE_CANCELLED`].
pub fn decode_media_to_wav_cancellable(
    input: &Path,
    out_wav: &Path,
    sample_rate: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-y",
        "-i",
        &input.to_string_lossy(),
        "-vn",
        "-ac",
        "1",
        "-ar",
        &sample_rate.to_string(),
        &out_wav.to_string_lossy(),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let end = run_child_cancellable(cmd, cancel, None).map_err(|e| {
        format!("ffmpeg nicht gefunden ({e}) — Installation z. B.: winget install ffmpeg")
    })?;
    match end {
        ChildEnd::Cancelled => {
            let _ = std::fs::remove_file(out_wav);
            Err(DECODE_CANCELLED.to_string())
        }
        ChildEnd::Finished {
            success: false,
            stderr_tail,
        } => {
            let tail: String = stderr_tail
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .join(" | ");
            Err(format!(
                "ffmpeg konnte {} nicht dekodieren: {tail}",
                input.display()
            ))
        }
        ChildEnd::Finished { success: true, .. } => {
            if out_wav.exists() {
                Ok(())
            } else {
                Err("ffmpeg lieferte keine Ausgabedatei".into())
            }
        }
    }
}

/// Wie [`ensure_wav`] mit abbrechbarer Dekodierung (Import, P8a).
pub fn ensure_wav_cancellable(
    input: &Path,
    sample_rate: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(PathBuf, Option<tempfile::TempPath>), String> {
    if extension_of(input) == "wav" {
        return Ok((input.to_path_buf(), None));
    }
    let tmp = tempfile::Builder::new()
        .prefix("lva-media-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?
        .into_temp_path();
    decode_media_to_wav_cancellable(input, &tmp, sample_rate, cancel)?;
    Ok((tmp.to_path_buf(), Some(tmp)))
}

/// Eingabedatei als WAV bereitstellen: WAV geht direkt durch, alles andere
/// wird in eine Tempdatei dekodiert. Rückgabe: (Pfad, Option<Tempdatei zum
/// Aufräumen durch den Aufrufer über NamedTempFile-Drop>).
pub fn ensure_wav(
    input: &Path,
    sample_rate: u32,
) -> Result<(PathBuf, Option<tempfile::TempPath>), String> {
    if extension_of(input) == "wav" {
        return Ok((input.to_path_buf(), None));
    }
    let tmp = tempfile::Builder::new()
        .prefix("lva-media-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?
        .into_temp_path();
    decode_media_to_wav(input, &tmp, sample_rate)?;
    Ok((tmp.to_path_buf(), Some(tmp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plain_text_and_markdown_read_directly_with_bom_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notiz.txt");
        std::fs::write(&p, b"\xEF\xBB\xBFHallo Dokument.\nZweite Zeile.").unwrap();
        assert_eq!(
            extract_document_text(&p).unwrap(),
            "Hallo Dokument.\nZweite Zeile."
        );
    }

    #[test]
    fn docx_body_xml_becomes_readable_paragraphs() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Erster Absatz mit &amp; Zeichen.</w:t></w:r></w:p><w:p><w:r><w:t>Zweiter</w:t></w:r><w:r><w:t> Absatz.</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(
            docx_xml_to_text(xml),
            "Erster Absatz mit & Zeichen.\nZweiter Absatz."
        );
    }

    #[test]
    fn a_real_docx_zip_is_extracted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("brief.docx");
        let file = std::fs::File::create(&p).unwrap();
        let mut z = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        z.start_file("word/document.xml", options).unwrap();
        z.write_all("<w:document><w:body><w:p><w:r><w:t>Inhalt aus Word.</w:t></w:r></w:p></w:body></w:document>".as_bytes()).unwrap();
        z.finish().unwrap();
        assert_eq!(extract_document_text(&p).unwrap(), "Inhalt aus Word.");
    }

    #[test]
    fn html_becomes_readable_prose_without_script_noise() {
        let html = r#"<html><head><title>x</title><style>p{color:red}</style></head>
<body><script>var a=1;</script><h1>Überschrift</h1><p>Erster &amp; wichtigster Absatz.</p>
<div>Zweiter&nbsp;Absatz.</div><footer>Impressum</footer></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("Überschrift"));
        assert!(text.contains("Erster & wichtigster Absatz."));
        assert!(text.contains("Zweiter Absatz."));
        assert!(
            !text.contains("var a"),
            "Script-Inhalt darf nicht auftauchen"
        );
        assert!(!text.contains("color:red"));
        assert!(!text.contains("Impressum"), "Footer wird entfernt");
    }

    #[test]
    fn unsupported_and_empty_documents_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("x.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(extract_document_text(&exe)
            .unwrap_err()
            .contains("txt, md, pdf, docx"));
        let empty = dir.path().join("leer.txt");
        std::fs::write(&empty, b"   \n").unwrap();
        assert!(extract_document_text(&empty).is_err());
    }

    /// Braucht ein installiertes ffmpeg; ohne wird übersprungen (lokal ist es
    /// vorhanden, der Test deckt den echten Dekodierpfad ab).
    #[test]
    fn media_files_decode_to_mono_16k_wav_via_ffmpeg() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("ffmpeg fehlt — Test übersprungen");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // 1 s 440-Hz-Ton als WAV schreiben, per ffmpeg nach mp3, dann zurück.
        let src = dir.path().join("ton.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..44_100u32 {
            w.write_sample(((i as f32 * 0.0627).sin() * 12000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        let mp3 = dir.path().join("ton.mp3");
        decode_media_to_wav(&src, &dir.path().join("umweg.wav"), 44_100).unwrap();
        assert!(std::process::Command::new("ffmpeg")
            .args(["-y", "-i", &src.to_string_lossy(), &mp3.to_string_lossy()])
            .output()
            .unwrap()
            .status
            .success());

        let (wav_path, _guard) = ensure_wav(&mp3, 16_000).unwrap();
        let reader = hound::WavReader::open(&wav_path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        let secs = reader.duration() as f32 / 16_000.0;
        assert!((secs - 1.0).abs() < 0.15, "Dauer blieb ~1 s, war {secs}");
    }

    // ---- P8a: abbrechbare Kindprozesse ---------------------------------------

    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let mut sys = System::new();
        let pid = Pid::from_u32(pid);
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        sys.process(pid).is_some()
    }

    #[cfg(windows)]
    fn ping_seconds(n: u32) -> std::process::Command {
        // Ein einzelner Prozess (kein cmd davor), der ~n Sekunden laeuft.
        let mut cmd = std::process::Command::new("ping");
        cmd.args(["-n", &n.to_string(), "127.0.0.1"]);
        cmd
    }

    #[cfg(windows)]
    #[test]
    fn a_stopped_child_is_killed_by_its_pid_and_does_not_linger() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        use std::sync::Arc;
        let cancel = Arc::new(AtomicBool::new(false));
        let pid = Arc::new(AtomicU32::new(0));
        let stopper = {
            let (cancel, pid) = (Arc::clone(&cancel), Arc::clone(&pid));
            std::thread::spawn(move || {
                while pid.load(Ordering::Acquire) == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                cancel.store(true, Ordering::Release);
            })
        };
        let started = std::time::Instant::now();
        let end = run_child_cancellable(ping_seconds(60), &cancel, Some(&pid)).unwrap();
        stopper.join().unwrap();
        assert_eq!(end, ChildEnd::Cancelled);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "kein Warten auf das natuerliche Ende (60 s)"
        );
        let pid = pid.load(Ordering::Acquire);
        assert_ne!(pid, 0);
        assert!(!process_alive(pid), "der Prozess {pid} ist beendet");
    }

    #[cfg(windows)]
    #[test]
    fn a_cancel_flag_set_before_the_start_never_spawns_anything() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        let cancel = AtomicBool::new(true);
        let pid = AtomicU32::new(0);
        let end = run_child_cancellable(ping_seconds(60), &cancel, Some(&pid)).unwrap();
        assert_eq!(end, ChildEnd::Cancelled);
        assert_eq!(pid.load(Ordering::Acquire), 0, "kein Prozess gestartet");
    }

    #[cfg(windows)]
    #[test]
    fn a_child_that_ends_on_its_own_reports_success_and_the_stderr_tail() {
        use std::sync::atomic::AtomicBool;
        let cancel = AtomicBool::new(false);
        assert_eq!(
            run_child_cancellable(ping_seconds(1), &cancel, None).unwrap(),
            ChildEnd::Finished {
                success: true,
                stderr_tail: String::new()
            }
        );
        let mut failing = std::process::Command::new("cmd");
        failing.args(["/C", "echo boom 1>&2 & exit 3"]);
        match run_child_cancellable(failing, &cancel, None).unwrap() {
            ChildEnd::Finished {
                success,
                stderr_tail,
            } => {
                assert!(!success);
                assert!(stderr_tail.contains("boom"), "{stderr_tail:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_hang() {
        use std::sync::atomic::AtomicBool;
        let cancel = AtomicBool::new(false);
        let cmd = std::process::Command::new("lva-programm-gibt-es-nicht");
        assert!(run_child_cancellable(cmd, &cancel, None).is_err());
    }

    /// Mit echtem ffmpeg: ohne Abbruch dekodiert es wie bisher, mit gesetztem
    /// Abbruch wird nichts gestartet und keine Datei hinterlassen.
    #[test]
    fn the_cancellable_decode_matches_the_plain_one_and_leaves_nothing_when_cancelled() {
        use std::sync::atomic::AtomicBool;
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("ffmpeg fehlt — Test uebersprungen");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("ton.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..44_100u32 {
            w.write_sample(((i as f32 * 0.0627).sin() * 12000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();

        let out = dir.path().join("out.wav");
        let go = AtomicBool::new(false);
        decode_media_to_wav_cancellable(&src, &out, 16_000, &go).unwrap();
        let reader = hound::WavReader::open(&out).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert!((reader.duration() as f32 / 16_000.0 - 1.0).abs() < 0.15);

        let out2 = dir.path().join("out2.wav");
        let stop = AtomicBool::new(true);
        assert_eq!(
            decode_media_to_wav_cancellable(&src, &out2, 16_000, &stop),
            Err(DECODE_CANCELLED.to_string())
        );
        assert!(!out2.exists(), "keine halbe Datei");
        // Ein Nicht-Audio-Eingang scheitert mit dem ffmpeg-Text, nicht mit Abbruch.
        let junk = dir.path().join("kaputt.mp3");
        std::fs::write(&junk, b"das ist kein Audio").unwrap();
        let err = decode_media_to_wav_cancellable(&junk, &dir.path().join("x.wav"), 16_000, &go)
            .unwrap_err();
        assert!(err.contains("nicht dekodieren"), "{err}");
    }
}
