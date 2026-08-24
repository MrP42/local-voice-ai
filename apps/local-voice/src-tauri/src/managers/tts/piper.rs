//! Piper-Engine (Paket B-E2): winziges VITS-TTS als CPU-Subprozess.
//!
//! Piper wird wie ffmpeg (`crate::media::decode_media_to_wav`) als externes
//! Binary gestartet — KEIN Linking, keine neue Rust-Dependency; das
//! GPL-lizenzierte espeak-ng bleibt im Subprozess. Je Syntheseauftrag läuft
//! genau ein Prozess: Text auf stdin, WAV in eine Temp-Datei, fertig. Das
//! ist bewusst der einfache Weg; ein langlebiger `--json-input`-Prozess ist
//! als Optimierung denkbar (Follow-up), aber erst, wenn er gebraucht wird.
//!
//! Binary und Stimmen legt Paket E3 in das App-Datenverzeichnis; die
//! Pfad-Funktionen hier sind der Ablage-VERTRAG zwischen beiden Paketen:
//!
//! ```text
//! <app_data>/tts/piper/<plattform>/piper(.exe)   das Binary
//! <app_data>/tts/piper/voices/<id>.onnx          eine Stimme
//! <app_data>/tts/piper/voices/<id>.onnx.json     ihre Konfiguration
//! ```
//!
//! Fehlt etwas, bleibt die Engine gewählt, aber nicht einsatzbereit:
//! `ensure_ready`/`synthesize` liefern eine KONSTANTE Fehler-ID (die UI
//! übersetzt) — es gibt keinen stillen Rückfall auf den Fish-GPU-Server.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::engine::{EngineCaps, SynthesisRequest, TtsEngine, TtsEngineKind};
use super::protocol;

/// Fähigkeiten von Piper: eine feste Stimme je Modell, kein Klonen, keine
/// Stil-Tags — dafür CPU-only und in unter einer Sekunde beim ersten Ton.
pub const PIPER_CAPS: EngineCaps = EngineCaps {
    style_tags: false,
    cloning: false,
    voice_switching: false,
    streaming: false,
    needs_gpu: false,
    export_formats: &["wav"],
};

/// Härter als das Fish-Timeout (300 s): Piper spricht einen Satz auf CPU in
/// wenigen Sekunden — wer länger als eine Minute braucht, hängt.
pub const PIPER_TIMEOUT: Duration = Duration::from_secs(60);

/// Konstante Fehler-IDs (Vertrag mit der UI: an den Texten wird verglichen
/// und übersetzt — NICHT umformulieren).
pub const ERR_BINARY_MISSING: &str = "Piper-Programm fehlt — unter Modelle > Vorlesestimmen laden";
pub const ERR_VOICE_MISSING: &str = "Piper-Stimme fehlt — unter Modelle > Vorlesestimmen laden";

/// Aufgelöste, existierende Pfade eines startklaren Piper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiperPaths {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub config: PathBuf,
}

/// Plattform-Unterverzeichnis des Binaries — Teil des Ablage-Vertrags mit
/// Paket E3 (Downloads): dieselbe Kennung muss dort beim Entpacken entstehen.
pub fn platform_subdir() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64"
        } else {
            "macos-x64"
        }
    } else {
        "linux-x64"
    }
}

fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "piper.exe"
    } else {
        "piper"
    }
}

/// Wo das Piper-Binary liegt (ob es liegt, prüft [`piper_paths`]).
pub fn piper_binary_path(app_data: &Path) -> PathBuf {
    app_data
        .join("tts")
        .join("piper")
        .join(platform_subdir())
        .join(binary_name())
}

/// Wo Modell und Konfiguration einer Stimme liegen.
pub fn piper_voice_paths(app_data: &Path, voice_id: &str) -> (PathBuf, PathBuf) {
    let dir = app_data.join("tts").join("piper").join("voices");
    (
        dir.join(format!("{voice_id}.onnx")),
        dir.join(format!("{voice_id}.onnx.json")),
    )
}

/// Pfad-Auflösung: alles vorhanden → die drei Pfade; sonst die konstante
/// Fehler-ID dessen, was fehlt. Kein Setting-Override (`tts_piper_dir` o. Ä.)
/// — der Katalog (E3) ist der einzige Weg, Dateien hierher zu bekommen.
pub fn piper_paths(app_data: &Path, voice_id: &str) -> Result<PiperPaths, &'static str> {
    let binary = piper_binary_path(app_data);
    if !binary.is_file() {
        return Err(ERR_BINARY_MISSING);
    }
    let (model, config) = piper_voice_paths(app_data, voice_id);
    if !model.is_file() || !config.is_file() {
        return Err(ERR_VOICE_MISSING);
    }
    Ok(PiperPaths {
        binary,
        model,
        config,
    })
}

/// `--length_scale` aus dem Wiedergabetempo: Kehrwert (doppeltes Tempo =
/// halbe Länge), geklemmt auf 0.5–2.0 — außerhalb klingt VITS hörbar
/// artefaktig. Unsinnige Werte (0, negativ, NaN) fallen auf 1.0 zurück.
pub fn length_scale_for_speed(speed: f32) -> f32 {
    if !speed.is_finite() || speed <= 0.0 {
        return 1.0;
    }
    (1.0 / speed).clamp(0.5, 2.0)
}

/// Kommandozeile eines Piper-Aufrufs — pur und damit testbar.
///
/// Die Reihenfolge ist FEST (model, config, output_file, dann optional
/// length_scale): die Fake-Binaries der Prozesstests greifen den
/// Ausgabepfad über seine Argumentposition (Nr. 6) ab.
pub fn piper_args(
    paths: &PiperPaths,
    out_wav: &Path,
    speed: Option<f32>,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--model".into(),
        paths.model.clone().into(),
        "--config".into(),
        paths.config.clone().into(),
        "--output_file".into(),
        out_wav.to_path_buf().into(),
    ];
    if let Some(speed) = speed {
        args.push("--length_scale".into());
        args.push(length_scale_for_speed(speed).to_string().into());
    }
    args
}

/// Die letzten `max_lines` nicht leeren Zeilen — eine Fehlermeldung ist kein
/// Protokollfenster, aber die letzten Zeilen von Pipers stderr sagen fast
/// immer, WAS fehlte (Modell unlesbar, Phonemizer-Daten weg, …).
fn stderr_tail(stderr: &str, max_lines: usize) -> String {
    let mut lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    lines.join(" | ")
}

/// Ein Piper-Lauf, blockierend: Prozess starten, Text auf stdin, auf die
/// WAV-Datei warten (mit Timeout), Bytes validiert zurückgeben. Die
/// Temp-Datei räumt sich über ihren Drop auf — auch auf jedem Fehlerpfad.
pub fn run_piper_blocking(
    paths: &PiperPaths,
    text: &str,
    speed: Option<f32>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let out = tempfile::Builder::new()
        .prefix("lva-piper-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| format!("Piper: keine Temp-Datei: {e}"))?
        .into_temp_path();

    let mut cmd = std::process::Command::new(&paths.binary);
    cmd.args(piper_args(paths, &out, speed))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (Muster: media.rs)
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{ERR_BINARY_MISSING} ({e})"))?;

    // Der Text als GENAU EINE Zeile: Piper spricht je stdin-Zeile eine
    // Äußerung — ein Satz mit hartem Umbruch käme sonst zweigeteilt heraus.
    // Sätze sind durch `tts_max_chars` gedeckelt (≤ ~20 KB) und passen damit
    // sicher in den Pipe-Puffer; ein Schreibfehler wird bewusst verschluckt:
    // stirbt Piper früh, ist die Pipe zu, und die Diagnose liefern dann
    // Exit-Status und stderr, nicht der Write.
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let _ = stdin.write_all(line.as_bytes());
        let _ = stdin.write_all(b"\n");
        // Drop schließt stdin: Piper sieht EOF und beendet sich nach der Zeile.
    }

    // stderr nebenläufig leeren, damit ein gesprächiges Piper nicht am
    // vollen Pipe-Puffer hängen bleibt, während wir auf das Ende warten.
    let mut stderr_pipe = child.stderr.take();
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(e) = stderr_pipe.as_mut() {
            let _ = e.read_to_string(&mut buf);
        }
        buf
    });

    // Warten mit Timeout — `std::process` kennt keins, also try_wait-Schleife.
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // kein Zombie
                    return Err(format!(
                        "Piper hat nach {} s nicht geantwortet und wurde beendet",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("Piper-Prozess nicht abfragbar: {e}")),
        }
    };
    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "Piper endete mit {status}: {}",
            stderr_tail(&stderr, 10)
        ));
    }
    let bytes = std::fs::read(&out).map_err(|e| format!("Piper-Ausgabe nicht lesbar: {e}"))?;
    if !protocol::looks_like_wav(&bytes) {
        return Err(format!(
            "Piper lieferte kein WAV: {}",
            stderr_tail(&stderr, 10)
        ));
    }
    Ok(bytes)
}

/// Die Piper-Engine des Kerns: hält die aufgelösten Pfade ODER den Grund,
/// warum sie nicht einsatzbereit ist. Aufgelöst wird bei jedem
/// Settings-Refresh (also vor jedem Auftrag) neu — nachgeladene Dateien
/// wirken damit ohne App-Neustart.
#[derive(Clone, Debug)]
pub struct PiperEngine {
    /// Gewählte Stimme (`tts_piper_voice`); geht in den Cache-Tag ein.
    voice_id: Option<String>,
    /// Ok: startklar. Err: konstante Fehler-ID, was fehlt.
    resolved: Result<PiperPaths, &'static str>,
}

impl PiperEngine {
    /// Aus App-Datenverzeichnis und Settings-Stimme auflösen. Ohne
    /// Datenverzeichnis kann kein Binary liegen; ohne gewählte Stimme ist
    /// die Meldung „Stimme fehlt" nur dann dran, wenn das Binary schon da
    /// ist — sonst ist das Binary die erste Baustelle.
    pub fn resolve(app_data: Option<&Path>, voice_id: Option<&str>) -> Self {
        let resolved = match (app_data, voice_id) {
            (None, _) => Err(ERR_BINARY_MISSING),
            (Some(data), None) => {
                if piper_binary_path(data).is_file() {
                    Err(ERR_VOICE_MISSING)
                } else {
                    Err(ERR_BINARY_MISSING)
                }
            }
            (Some(data), Some(voice)) => piper_paths(data, voice),
        };
        Self {
            voice_id: voice_id.map(str::to_string),
            resolved,
        }
    }

    /// Warum die Engine nicht einsatzbereit ist (None = bereit) — Grundlage
    /// der Warnzeile beim Engine-Wechsel in `TtsCore::set_engine`.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        self.resolved.as_ref().err().copied()
    }
}

impl TtsEngine for PiperEngine {
    fn kind(&self) -> TtsEngineKind {
        TtsEngineKind::Piper
    }

    fn caps(&self) -> EngineCaps {
        PIPER_CAPS
    }

    /// `"piper/<voice_id>"` — der Parameter (die Fish-Referenzstimme) ist
    /// für Piper bedeutungslos; was den Klang bestimmt, ist das Modell.
    fn cache_tag(&self, _voice: Option<&str>) -> String {
        format!("piper/{}", self.voice_id.as_deref().unwrap_or(""))
    }

    async fn ensure_ready(&self) -> Result<(), String> {
        self.resolved
            .as_ref()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Ein Satz = ein Subprozess, ausgelagert auf einen Blocking-Thread —
    /// der try_wait-Timeout darf keinen Async-Worker blockieren.
    async fn synthesize(&self, req: SynthesisRequest<'_>) -> Result<Vec<u8>, String> {
        let paths = self.resolved.clone().map_err(str::to_string)?;
        let text = req.text.to_string();
        let speed = req.speed;
        tokio::task::spawn_blocking(move || run_piper_blocking(&paths, &text, speed, PIPER_TIMEOUT))
            .await
            .map_err(|e| format!("Piper-Lauf abgebrochen: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Pfad-Auflösung
    // ------------------------------------------------------------------

    #[test]
    fn das_binary_liegt_im_plattform_unterverzeichnis() {
        let p = piper_binary_path(Path::new("data"));
        let expected: PathBuf = ["data", "tts", "piper", platform_subdir(), binary_name()]
            .iter()
            .collect();
        assert_eq!(p, expected);
        if cfg!(windows) {
            assert!(p.ends_with("windows-x64/piper.exe"));
        }
    }

    #[test]
    fn stimmen_liegen_als_onnx_paar_im_voices_verzeichnis() {
        let (model, config) = piper_voice_paths(Path::new("data"), "eva");
        let voices: PathBuf = ["data", "tts", "piper", "voices"].iter().collect();
        assert_eq!(model, voices.join("eva.onnx"));
        assert_eq!(config, voices.join("eva.onnx.json"));
    }

    /// Ein tempdir-Baukasten: legt wahlweise Binary und/oder Stimme an.
    fn data_dir(binary: bool, voice: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if binary {
            let bin = piper_binary_path(dir.path());
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, b"fake").unwrap();
        }
        if let Some(id) = voice {
            let (model, config) = piper_voice_paths(dir.path(), id);
            std::fs::create_dir_all(model.parent().unwrap()).unwrap();
            std::fs::write(&model, b"onnx").unwrap();
            std::fs::write(&config, b"{}").unwrap();
        }
        dir
    }

    #[test]
    fn fehlendes_binary_meldet_die_binary_id_auch_wenn_die_stimme_da_ist() {
        let dir = data_dir(false, Some("eva"));
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_BINARY_MISSING));
    }

    #[test]
    fn fehlende_stimme_oder_fehlende_config_melden_die_stimmen_id() {
        let dir = data_dir(true, None);
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_VOICE_MISSING));

        // Modell da, aber die .onnx.json fehlt → ebenfalls Stimme unvollständig.
        let dir = data_dir(true, Some("eva"));
        let (_, config) = piper_voice_paths(dir.path(), "eva");
        std::fs::remove_file(config).unwrap();
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_VOICE_MISSING));
    }

    #[test]
    fn vollstaendige_ablage_liefert_die_drei_pfade() {
        let dir = data_dir(true, Some("eva"));
        let paths = piper_paths(dir.path(), "eva").unwrap();
        assert_eq!(paths.binary, piper_binary_path(dir.path()));
        assert!(paths.model.ends_with("eva.onnx"));
        assert!(paths.config.ends_with("eva.onnx.json"));
    }

    #[test]
    fn resolve_benennt_die_erste_baustelle() {
        // Kein Datenverzeichnis → das Binary ist die erste Baustelle.
        let e = PiperEngine::resolve(None, Some("eva"));
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_MISSING));

        // Binary da, keine Stimme gewählt → die Stimme fehlt.
        let dir = data_dir(true, None);
        let e = PiperEngine::resolve(Some(dir.path()), None);
        assert_eq!(e.unavailable_reason(), Some(ERR_VOICE_MISSING));

        // Nichts da, nichts gewählt → erst das Binary.
        let empty = tempfile::tempdir().unwrap();
        let e = PiperEngine::resolve(Some(empty.path()), None);
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_MISSING));

        // Alles da → bereit.
        let dir = data_dir(true, Some("eva"));
        let e = PiperEngine::resolve(Some(dir.path()), Some("eva"));
        assert_eq!(e.unavailable_reason(), None);
    }

    // ------------------------------------------------------------------
    // Kommandozeile und Tempo
    // ------------------------------------------------------------------

    #[test]
    fn das_tempo_wird_zum_gekehrten_und_geklemmten_length_scale() {
        assert_eq!(length_scale_for_speed(1.0), 1.0);
        assert_eq!(length_scale_for_speed(2.0), 0.5);
        assert_eq!(length_scale_for_speed(0.5), 2.0);
        assert_eq!(length_scale_for_speed(4.0), 0.5, "unten geklemmt");
        assert_eq!(length_scale_for_speed(0.25), 2.0, "oben geklemmt");
        assert_eq!(length_scale_for_speed(0.0), 1.0, "0 wäre unendlich");
        assert_eq!(length_scale_for_speed(-1.0), 1.0);
        assert_eq!(length_scale_for_speed(f32::NAN), 1.0);
    }

    fn dummy_paths() -> PiperPaths {
        PiperPaths {
            binary: PathBuf::from("piper.exe"),
            model: PathBuf::from("eva.onnx"),
            config: PathBuf::from("eva.onnx.json"),
        }
    }

    #[test]
    fn die_kommandozeile_traegt_model_config_und_ausgabedatei_in_fester_reihenfolge() {
        let args = piper_args(&dummy_paths(), Path::new("out.wav"), None);
        let expected: Vec<std::ffi::OsString> = vec![
            "--model".into(),
            "eva.onnx".into(),
            "--config".into(),
            "eva.onnx.json".into(),
            "--output_file".into(),
            "out.wav".into(),
        ];
        assert_eq!(args, expected, "Reihenfolge ist Vertrag (Fake-Tests: %6)");
    }

    #[test]
    fn ein_tempo_ergaenzt_den_geklemmten_length_scale() {
        let args = piper_args(&dummy_paths(), Path::new("out.wav"), Some(2.0));
        assert_eq!(args.len(), 8);
        assert_eq!(args[6], std::ffi::OsString::from("--length_scale"));
        assert_eq!(args[7], std::ffi::OsString::from("0.5"));
    }

    // ------------------------------------------------------------------
    // Caps, Cache-Tag, stderr-Kürzung
    // ------------------------------------------------------------------

    #[test]
    fn piper_ist_cpu_only_und_exportiert_nur_wav() {
        assert!(!PIPER_CAPS.needs_gpu);
        assert!(!PIPER_CAPS.cloning && !PIPER_CAPS.voice_switching && !PIPER_CAPS.style_tags);
        assert!(!PIPER_CAPS.streaming);
        assert_eq!(PIPER_CAPS.export_formats, &["wav"]);
    }

    #[test]
    fn der_cache_tag_traegt_die_piper_stimme_nicht_die_fish_referenz() {
        let dir = data_dir(true, Some("eva"));
        let e = PiperEngine::resolve(Some(dir.path()), Some("eva"));
        assert_eq!(e.cache_tag(None), "piper/eva");
        assert_eq!(
            e.cache_tag(Some("patrick")),
            "piper/eva",
            "die Fish-Referenzstimme ändert am Piper-Klang nichts"
        );
        let ohne = PiperEngine::resolve(None, None);
        assert_eq!(ohne.cache_tag(None), "piper/");
    }

    #[test]
    fn stderr_wird_auf_die_letzten_zehn_zeilen_gekuerzt() {
        let long = (1..=14)
            .map(|i| format!("zeile {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = stderr_tail(&long, 10);
        assert!(!tail.contains("zeile 4"));
        assert!(tail.starts_with("zeile 5"));
        assert!(tail.ends_with("zeile 14"));
        assert_eq!(stderr_tail("  \n\n  ", 10), "", "Leerzeilen zählen nicht");
    }

    // ------------------------------------------------------------------
    // Prozesspfad mit Fake-Binaries (Muster media.rs: echte Subprozesse)
    // ------------------------------------------------------------------

    /// Ein Fake-Piper als Skript: `Ok` kopiert eine Mini-WAV an den
    /// Ausgabepfad (Argument 6), `Broken` schreibt Müll dorthin, `Fail`
    /// endet mit Exit-Code 3 und einer stderr-Zeile.
    enum Fake {
        Ok,
        Broken,
        Fail,
    }

    fn write_fake_piper(dir: &Path, fake: Fake) -> PathBuf {
        let template = dir.join("vorlage.wav");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&template, &wav).unwrap();

        #[cfg(windows)]
        {
            let script = dir.join("fake-piper.cmd");
            let body = match fake {
                Fake::Ok => format!(
                    "@echo off\r\ncopy /y /b \"{}\" \"%~6\" > nul\r\n",
                    template.display()
                ),
                Fake::Broken => "@echo off\r\necho kein-riff > \"%~6\"\r\n".to_string(),
                Fake::Fail => {
                    "@echo off\r\necho Modelldatei nicht lesbar 1>&2\r\nexit /b 3\r\n".to_string()
                }
            };
            std::fs::write(&script, body).unwrap();
            script
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.join("fake-piper.sh");
            let body = match fake {
                Fake::Ok => format!("#!/bin/sh\ncp \"{}\" \"$6\"\n", template.display()),
                Fake::Broken => "#!/bin/sh\nprintf 'kein-riff' > \"$6\"\n".to_string(),
                Fake::Fail => {
                    "#!/bin/sh\necho 'Modelldatei nicht lesbar' >&2\nexit 3\n".to_string()
                }
            };
            std::fs::write(&script, body).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }
    }

    fn fake_paths(dir: &Path, fake: Fake) -> PiperPaths {
        PiperPaths {
            binary: write_fake_piper(dir, fake),
            model: dir.join("eva.onnx"),
            config: dir.join("eva.onnx.json"),
        }
    }

    #[test]
    fn der_prozesspfad_liefert_die_wav_bytes_der_ausgabedatei() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Ok);
        let bytes = run_piper_blocking(&paths, "Hallo Welt.", None, PIPER_TIMEOUT).unwrap();
        assert!(protocol::looks_like_wav(&bytes));
        assert_eq!(bytes.len(), 4 + 4096);
    }

    #[test]
    fn ein_fehlendes_binary_meldet_die_binary_id() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PiperPaths {
            binary: dir.path().join("gibt-es-nicht.exe"),
            model: dir.path().join("eva.onnx"),
            config: dir.path().join("eva.onnx.json"),
        };
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains(ERR_BINARY_MISSING), "war: {err}");
    }

    #[test]
    fn ein_scheiternder_prozess_traegt_seinen_stderr_in_der_meldung() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Fail);
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains("Modelldatei nicht lesbar"), "war: {err}");
    }

    #[test]
    fn eine_kaputte_ausgabedatei_wird_als_kein_wav_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Broken);
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains("kein WAV"), "war: {err}");
    }

    /// Der ganze Trait-Pfad asynchron: eine startklare Engine spricht über
    /// den Fake, eine nicht aufgelöste liefert ihre konstante Fehler-ID.
    #[tokio::test(flavor = "multi_thread")]
    async fn die_engine_synthetisiert_ueber_den_subprozess_oder_meldet_warum_nicht() {
        let dir = tempfile::tempdir().unwrap();
        let ready = PiperEngine {
            voice_id: Some("eva".into()),
            resolved: Ok(fake_paths(dir.path(), Fake::Ok)),
        };
        ready.ensure_ready().await.unwrap();
        let req = SynthesisRequest {
            text: "Hallo Welt.",
            voice: None,
            seed: 42,
            speed: None,
        };
        let bytes = ready.synthesize(req).await.unwrap();
        assert!(protocol::looks_like_wav(&bytes));

        let missing = PiperEngine::resolve(None, None);
        assert_eq!(
            missing.ensure_ready().await.unwrap_err(),
            ERR_BINARY_MISSING
        );
        assert_eq!(
            missing.synthesize(req).await.unwrap_err(),
            ERR_BINARY_MISSING
        );
    }
}
