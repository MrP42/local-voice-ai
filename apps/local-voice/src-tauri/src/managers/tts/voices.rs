//! Verwaltung der Referenzstimmen (TP2, zero-shot Voice Cloning).
//!
//! Referenzen liegen im Server-Format direkt beim Fish-Server:
//! `<fish_dir>/references/<voice_id>/sample.wav` + `sample.lab`. Die App
//! dupliziert nichts; die Stimmenliste ist ein Verzeichnis-Scan und
//! funktioniert auch bei gestopptem Server.

use std::path::{Path, PathBuf};

/// Praefix interner, nicht vom Nutzer angelegter Referenzen.
pub const INTERNAL_PREFIX: &str = "__";

/// Verzeichnisname der aus einem Seed erzeugten Standardstimme.
pub fn seed_voice_id(seed: i64) -> String {
    format!("{INTERNAL_PREFIX}seed_{seed}")
}

/// Ist die Referenz dieser Stimme vollstaendig (WAV samt Transkript)?
pub fn voice_is_complete(fish_dir: &Path, id: &str) -> bool {
    voice_sample(fish_dir, id).is_some()
}

/// 16-kHz-Samples unterhalb dieser Dauer taugen nicht als Referenz.
pub const MIN_REFERENCE_SECS: usize = 3;
const SAMPLE_RATE: usize = 16_000;

pub fn reference_long_enough(sample_count: usize) -> bool {
    sample_count >= MIN_REFERENCE_SECS * SAMPLE_RATE
}

/// Nutzereingaben werden Verzeichnisnamen und JSON-Werte: klein, ASCII,
/// `a-z0-9_-`, deutsche Umlaute transliteriert, max 40 Zeichen.
pub fn sanitize_voice_id(raw: &str) -> Option<String> {
    let mapped: String = raw
        .to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Mehrfach-Bindestriche zusammenfassen, Ränder trimmen, Länge kappen.
    let mut collapsed = String::new();
    for c in mapped.chars() {
        if c == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(c);
    }
    let trimmed: String = collapsed.trim_matches('-').chars().take(40).collect();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn references_dir(fish_dir: &Path) -> PathBuf {
    fish_dir.join("references")
}

pub fn voice_dir(fish_dir: &Path, id: &str) -> PathBuf {
    references_dir(fish_dir).join(id)
}

/// Interner Ordnername der Stil-Referenz einer Stimme: `__style_<voice>_<style>`.
/// Dieselbe Kennung wird als `VoiceStyle::reference` in der Registry
/// abgelegt (siehe `registry::VoiceStyle`).
fn style_voice_id(voice: &str, style: &str) -> String {
    format!("{INTERNAL_PREFIX}style_{voice}_{style}")
}

/// Verzeichnis der Stil-Referenzaufnahme einer Stimme. Liegt unter demselben
/// `references/`-Wurzelverzeichnis wie jede andere Stimme, ist aber wegen des
/// `__`-Präfixes aus `list_voices` ausgenommen (siehe dort).
pub fn style_dir(fish_dir: &Path, voice: &str, style: &str) -> PathBuf {
    voice_dir(fish_dir, &style_voice_id(voice, style))
}

/// Die Stil-Referenz wie jede andere Referenz speichern (gleiches Format,
/// gleiche Pegelung) — nur unter dem Stil-Ordnernamen statt dem der Stimme.
/// Rückgabe bei Erfolg: die interne reference_id (`__style_<voice>_<style>`),
/// zum Ablegen in `VoiceStyle::reference`.
pub fn save_style_voice(
    fish_dir: &Path,
    voice: &str,
    style: &str,
    samples: &[f32],
    transcript: &str,
    enhance: Option<super::enhance::Strength>,
) -> Result<String, String> {
    let id = style_voice_id(voice, style);
    save_voice(fish_dir, &id, samples, transcript, enhance)?;
    Ok(id)
}

/// Alle Stimmen mit mindestens einem WAV samt gleichnamiger .lab-Datei —
/// dieselbe Gültigkeitsregel, die der Fish-Server beim Laden anwendet.
pub fn list_voices(fish_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(references_dir(fish_dir)) else {
        return Vec::new();
    };
    let mut voices: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        // Interne Referenzen (Praefix `__`) gehoeren nicht in die Stimmenliste:
        // die Standardstimme legt sich eine an, damit sie ueber Saetze hinweg
        // dieselbe bleibt — als eigener Eintrag waere sie dieselbe Stimme
        // zweimal, einmal loeschbar.
        .filter(|e| !e.file_name().to_string_lossy().starts_with(INTERNAL_PREFIX))
        .filter(|e| {
            std::fs::read_dir(e.path())
                .map(|files| {
                    files.flatten().any(|f| {
                        let p = f.path();
                        p.extension().is_some_and(|ext| ext == "wav")
                            && p.with_extension("lab").exists()
                    })
                })
                .unwrap_or(false)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    voices.sort();
    voices
}

/// Die Referenzaufnahme einer Stimme samt ihrem Transkript — genau die Datei,
/// aus der Fish Speech die Stimme nachbildet. Sie ist damit auch die
/// ehrlichste Hoerprobe: kein erzeugtes Beispiel, das erst einen Serverstart
/// und Sekunden GPU-Zeit kostet, sondern die Stimme selbst.
///
/// Genommen wird das erste WAV mit gleichnamiger .lab-Datei — dieselbe
/// Gueltigkeitsregel wie in `list_voices`, damit die Liste und die Hoerprobe
/// nicht auseinanderlaufen koennen.
pub fn voice_sample(fish_dir: &Path, id: &str) -> Option<(PathBuf, String)> {
    let dir = voice_dir(fish_dir, id);
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "wav") && path.with_extension("lab").exists()
        })
        .collect();
    // Deterministic: the same voice must always preview the same take.
    candidates.sort();
    let wav = candidates.into_iter().next()?;
    let transcript = std::fs::read_to_string(wav.with_extension("lab"))
        .unwrap_or_default()
        .trim()
        .to_string();
    Some((wav, transcript))
}

/// Zielpegel der Referenzaufnahmen: -20 LUFS nach ITU-R BS.1770-4.
///
/// Warum nicht RMS über die ganze Datei (so war es bis v0.8.2): RMS zählt
/// Pausen mit. Eine bedächtig gesprochene Referenz mit Atempausen misst
/// dadurch leiser, als sie klingt, und wird zu weit hochgezogen — die
/// nächste Stimme, dicht gesprochen, zu wenig. Genau so entstehen zwei
/// unterschiedlich laute Sprecher trotz „Normalisierung". Die gegatete
/// Lautheitsmessung in [`super::loudness`] misst nur das Gesprochene.
///
/// Rein rechnerisch und ohne Seiteneffekt, damit die Regel prüfbar ist:
/// Stille bleibt unverändert (Faktor 1), zu Lautes wird leiser, zu Leises
/// lauter — aber nie so weit, dass die Spitze anschlägt.
pub fn normalize_gain(samples: &[f32], sample_rate: u32) -> f32 {
    super::loudness::gain_for_mono(samples, sample_rate)
}

/// Aufnahme (16 kHz mono f32) als Referenz speichern: sample.wav (16-bit PCM)
/// plus sample.lab (Transkript, UTF-8 ohne BOM).
pub fn save_voice(
    fish_dir: &Path,
    id: &str,
    samples: &[f32],
    transcript: &str,
    enhance: Option<super::enhance::Strength>,
) -> Result<(), String> {
    if transcript.trim().is_empty() {
        return Err("transcript must not be empty".into());
    }
    if !reference_long_enough(samples.len()) {
        return Err(format!(
            "reference too short: need at least {MIN_REFERENCE_SECS} s of audio"
        ));
    }
    let dir = voice_dir(fish_dir, id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let wav_path = dir.join("sample.wav");
    let mut writer = hound::WavWriter::create(&wav_path, spec)
        .map_err(|e| format!("could not write {}: {e}", wav_path.display()))?;
    // Klangbearbeitung VOR dem Pegeln: der Zielpegel soll fuer das gelten,
    // was am Ende in der Datei steht, nicht fuer eine Zwischenstufe.
    //
    // Hier bringt sie am meisten: Fish Speech bildet die Stimme aus dieser
    // Aufnahme nach — mitsamt Luefterrauschen. Was hier hineingeraet, steckt
    // in jeder spaeteren Synthese.
    let owned;
    let samples: &[f32] = match enhance {
        Some(strength) => {
            let mut work = samples.to_vec();
            super::enhance::process(&mut work, SAMPLE_RATE as u32, strength);
            owned = work;
            &owned
        }
        None => samples,
    };
    // Alle Stimmen auf denselben Pegel: sonst ist in einem Dialog eine
    // Sprecherin dauernd zu leise und die nächste zu laut.
    let gain = normalize_gain(samples, SAMPLE_RATE as u32);
    for &s in samples {
        let clamped = ((s * gain).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(clamped)
            .map_err(|e| format!("wav write failed: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("wav finalize failed: {e}"))?;

    write_lab(&dir, transcript)
}

/// Vorhandene WAV-Datei übernehmen — Abtastrate und Kanäle bleiben, wie sie
/// sind (der Fish-Server resampled selbst, Studioqualität bleibt erhalten),
/// nur der Pegel wird auf `loudness::TARGET_LUFS` gezogen. Plus Transkript als .lab.
pub fn import_voice(
    fish_dir: &Path,
    id: &str,
    source_wav: &Path,
    transcript: &str,
    enhance: Option<super::enhance::Strength>,
) -> Result<(), String> {
    if transcript.trim().is_empty() {
        return Err("transcript must not be empty".into());
    }
    // Frühe Validierung: muss als WAV lesbar sein, bevor irgendetwas entsteht.
    let mut reader = hound::WavReader::open(source_wav)
        .map_err(|e| format!("not a readable WAV file ({}): {e}", source_wav.display()))?;
    let spec = reader.spec();
    let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV nicht lesbar: {e}"))?,
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / scale))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV nicht lesbar: {e}"))?,
    };
    // Klangbearbeitung nur bei einkanaligen Quellen: die Kette ist fuer eine
    // Sprachspur gebaut und auch nur dafuer geprueft. Eine Stereodatei
    // bekommt sie nicht, statt sie mit ungepruefter Kanalbehandlung zu
    // veraendern.
    let samples: Vec<f32> = match (enhance, spec.channels) {
        (Some(strength), 1) => {
            let mut work = samples;
            super::enhance::process(&mut work, spec.sample_rate, strength);
            work
        }
        _ => samples,
    };
    // Gemessen wird über den Mono-Downmix — dieselbe Sicht, die auch der
    // Fish-Server auf die Referenz hat. Gedeckelt wird gegen die Spitze
    // *aller* Kanäle, sonst clippt bei Stereo der lautere Kanal.
    let channels = spec.channels.max(1) as usize;
    let mono: Vec<f32> = if channels == 1 {
        samples.clone()
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    let gain =
        super::loudness::gain_to_target(&mono, spec.sample_rate, super::loudness::peak(&samples));

    let dir = voice_dir(fish_dir, id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let wav_path = dir.join("sample.wav");
    // Immer 16-bit Int geschrieben: das ist das Format, das auch die eigene
    // Aufnahme erzeugt, und der Server verlangt nichts anderes.
    let out_spec = hound::WavSpec {
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
        ..spec
    };
    let mut writer = hound::WavWriter::create(&wav_path, out_spec)
        .map_err(|e| format!("could not write {}: {e}", wav_path.display()))?;
    for sample in samples {
        let value = ((sample * gain).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(value)
            .map_err(|e| format!("wav write failed: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("wav finalize failed: {e}"))?;
    write_lab(&dir, transcript)
}

/// Marker, dass die Stimme bereits mit dem Lautheitsmaß gepegelt wurde.
/// Der Name trägt die Version: eine spätere Änderung des Verfahrens bekommt
/// einen neuen Marker und läuft dadurch von selbst noch einmal.
const NORMALIZED_MARKER: &str = ".loudness-v2";
/// Sicherungskopie der Originalaufnahme. Sie ist die Quelle jeder erneuten
/// Normalisierung — sonst würde wiederholtes Pegeln den Klang aufschaukeln.
const ORIGINAL_SUFFIX: &str = "orig.wav";

/// Bestandsstimmen einmalig auf das Lautheitsmaß nachziehen.
///
/// Vor v0.8.3 wurden Referenzen über das RMS der ganzen Datei gepegelt; wer
/// damals drei Stimmen angelegt hat, hat drei unterschiedlich laute. Ohne
/// diesen Durchlauf bliebe der Bestand schief, und der Nutzer müsste jede
/// Stimme neu aufnehmen.
///
/// Idempotent über `NORMALIZED_MARKER`, verlustfrei über die Sicherung in
/// `sample.orig.wav`. Fehler bei einer Stimme brechen den Lauf nicht ab —
/// eine unlesbare Datei darf die übrigen nicht blockieren. Rückgabe: Anzahl
/// der tatsächlich nachgezogenen Aufnahmen.
pub fn renormalize_existing(fish_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(references_dir(fish_dir)) else {
        return 0;
    };
    let mut done = 0usize;
    for voice in entries.flatten().filter(|e| e.path().is_dir()) {
        let dir = voice.path();
        if dir.join(NORMALIZED_MARKER).exists() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let wavs: Vec<PathBuf> = files
            .flatten()
            .map(|f| f.path())
            .filter(|p| {
                p.extension().is_some_and(|ext| ext == "wav")
                    && !p.to_string_lossy().ends_with(ORIGINAL_SUFFIX)
                    && p.with_extension("lab").exists()
            })
            .collect();
        for wav in &wavs {
            match renormalize_file(wav) {
                Ok(()) => done += 1,
                Err(e) => log::warn!("could not renormalize {}: {e}", wav.display()),
            }
        }
        // Marker auch dann setzen, wenn nichts zu tun war: sonst versucht es
        // die App bei jedem Start erneut.
        if let Err(e) = std::fs::write(dir.join(NORMALIZED_MARKER), b"") {
            log::warn!("could not mark {} as normalized: {e}", dir.display());
        }
    }
    done
}

/// Eine Referenzdatei aus ihrem Original neu pegeln.
fn renormalize_file(wav: &Path) -> Result<(), String> {
    let original = wav.with_extension(ORIGINAL_SUFFIX);
    if !original.exists() {
        std::fs::copy(wav, &original)
            .map_err(|e| format!("could not back up {}: {e}", wav.display()))?;
    }
    let (samples, spec) = read_wav(&original)?;
    let channels = spec.channels.max(1) as usize;
    let mono: Vec<f32> = if channels == 1 {
        samples.clone()
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    let gain =
        super::loudness::gain_to_target(&mono, spec.sample_rate, super::loudness::peak(&samples));
    write_pcm16(wav, &samples, gain, spec)
}

/// WAV vollständig als f32 lesen (Bittiefe und Format egal), samt Spezifikation.
fn read_wav(path: &Path) -> Result<(Vec<f32>, hound::WavSpec), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("not a readable WAV ({}): {e}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV nicht lesbar: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV nicht lesbar: {e}"))?
        }
    };
    Ok((samples, spec))
}

/// Samples mit Faktor als 16-bit-PCM schreiben — das Format, das der
/// Fish-Server erwartet und das auch die eigene Aufnahme erzeugt.
fn write_pcm16(
    path: &Path,
    samples: &[f32],
    gain: f32,
    spec: hound::WavSpec,
) -> Result<(), String> {
    let out_spec = hound::WavSpec {
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
        ..spec
    };
    let mut writer = hound::WavWriter::create(path, out_spec)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    for &sample in samples {
        let value = ((sample * gain).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(value)
            .map_err(|e| format!("wav write failed: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("wav finalize failed: {e}"))
}

fn write_lab(dir: &Path, transcript: &str) -> Result<(), String> {
    // Frisch geschriebene Referenzen sind bereits gepegelt: Marker setzen,
    // damit `renormalize_existing` sie nicht noch einmal anfasst.
    if let Err(e) = std::fs::write(dir.join(NORMALIZED_MARKER), b"") {
        log::warn!("could not mark {} as normalized: {e}", dir.display());
    }
    let lab_path = dir.join("sample.lab");
    std::fs::write(&lab_path, transcript.trim().as_bytes())
        .map_err(|e| format!("could not write {}: {e}", lab_path.display()))
}

/// Beliebiges PCM-WAV (Rate/Kanäle/Bittiefe egal) als 16-kHz-Mono-f32 laden —
/// nur für die STT-Transkription beim Import; die Referenzdatei selbst wird
/// unverändert kopiert. Lineares Resampling reicht für Spracherkennung.
pub fn load_wav_mono_16k(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("not a readable WAV: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        (hound::SampleFormat::Int, bits) => {
            let scale = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };

    // Downmix: Kanäle mitteln.
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    if spec.sample_rate == SAMPLE_RATE as u32 {
        return Ok(mono);
    }
    if mono.is_empty() {
        return Ok(mono);
    }
    let ratio = spec.sample_rate as f64 / SAMPLE_RATE as f64;
    let out_len = ((mono.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono[idx.min(mono.len() - 1)];
        let b = mono[(idx + 1).min(mono.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    Ok(out)
}

/// Name der Registerdatei: eine Markdown-Übersicht aller Stimmen, direkt im
/// Referenzverzeichnis. Für Menschen gedacht — die Quelle der Wahrheit
/// bleiben die Verzeichnisse selbst.
const REGISTRY_MD: &str = "stimmen.md";

/// Dateiname, in dem eine aus einem Seed gesicherte Stimme ihren Seed trägt.
/// Nur solche Stimmen haben einen: eine aufgenommene Stimme hat eine
/// Aufnahme als Herkunft, keinen Zahlenwert.
const SEED_FILE: &str = "seed.txt";

/// Den Seed einer gesicherten Stimme ablegen.
pub fn write_seed_marker(fish_dir: &Path, id: &str, seed: i64) {
    let path = voice_dir(fish_dir, id).join(SEED_FILE);
    if let Err(e) = std::fs::write(&path, seed.to_string()) {
        log::warn!("could not record seed for {id}: {e}");
    }
}

/// Das Stimmen-Register neu schreiben.
///
/// Immer vollständig aus dem Bestand erzeugt, nie fortgeschrieben: eine
/// angehängte Zeile kann veralten (Stimme gelöscht, umbenannt), ein
/// Neuaufbau kann es nicht. Bei einer Handvoll Stimmen kostet das nichts.
pub fn update_registry(fish_dir: &Path) {
    let mut lines: Vec<String> = vec![
        "# Stimmen".to_string(),
        String::new(),
        "Automatisch gepflegt von Local Voice AI — Änderungen hier werden beim".to_string(),
        "nächsten Speichern oder Löschen einer Stimme überschrieben.".to_string(),
        String::new(),
        "| Stimme | Herkunft | Transkript der Referenz |".to_string(),
        "|---|---|---|".to_string(),
    ];
    for id in list_voices(fish_dir) {
        let dir = voice_dir(fish_dir, &id);
        let origin = std::fs::read_to_string(dir.join(SEED_FILE))
            .map(|s| format!("Seed {}", s.trim()))
            .unwrap_or_else(|_| "Aufnahme/Import".to_string());
        let transcript = voice_sample(fish_dir, &id)
            .map(|(_, t)| {
                let short: String = t.chars().take(80).collect();
                if t.chars().count() > 80 {
                    format!("{short}…")
                } else {
                    short
                }
            })
            .unwrap_or_default()
            // Ein | im Transkript zerrisse die Tabelle.
            .replace('|', "/");
        lines.push(format!("| {id} | {origin} | {transcript} |"));
    }
    let path = references_dir(fish_dir).join(REGISTRY_MD);
    if let Err(e) = std::fs::write(&path, lines.join("\n") + "\n") {
        log::warn!("could not write {}: {e}", path.display());
    }
}

/// Referenzverzeichnis entfernen. Eine nicht (mehr) existierende Stimme ist
/// kein Fehler — das Ziel „weg" ist erreicht.
///
/// Räumt zusätzlich alle Stil-Referenzordner dieser Stimme ab
/// (`__style_<id>_*`, siehe `style_dir`) — ohne diese Kaskade blieben
/// verwaiste Stil-Aufnahmen liegen, für die keine Stimme mehr existiert.
pub fn delete_voice(fish_dir: &Path, id: &str) -> Result<(), String> {
    let dir = voice_dir(fish_dir, id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("could not delete {}: {e}", dir.display()))?;
    }
    delete_style_dirs(fish_dir, id);
    Ok(())
}

/// Alle Stil-Referenzordner einer Stimme entfernen. Eine einzelne
/// fehlgeschlagene Löschung bricht die übrigen nicht ab — verwaiste Ordner
/// sind bedauerlich, dürfen aber das Löschen der Stimme selbst nicht
/// blockieren.
fn delete_style_dirs(fish_dir: &Path, id: &str) {
    let prefix = format!("{INTERNAL_PREFIX}style_{id}_");
    let Ok(entries) = std::fs::read_dir(references_dir(fish_dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && entry.path().is_dir() {
            if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                log::warn!("could not delete style dir {name}: {e}");
            }
        }
    }
}

/// Erweiterungen, in denen eine Avatar-Datei je Stimme abgelegt sein kann —
/// genau eine gleichzeitig, siehe `save_avatar`.
const AVATAR_EXTENSIONS: [&str; 3] = ["png", "webp", "jpg"];

/// Obergrenze einer Avatar-Datei: 2 MiB. Ein Avatar ist ein kleines Icon/Foto
/// neben dem Stimmennamen, kein Liefergegenstand — alles darüber ist
/// vermutlich ein Versehen (falsche Datei gewählt).
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// Avatar-Datei im Stimmenordner ablegen. Ersetzt eine vorhandene Avatar-
/// Datei ANDERER Erweiterung (immer höchstens ein Avatar je Stimme) und
/// liefert den geschriebenen Dateinamen (`avatar.<ext>`) zurück.
///
/// `ext` ohne führenden Punkt, eine von `png`/`webp`/`jpg` (case-insensitiv).
/// Die Bytes kommen vom Frontend-Command als `Vec<u8>`, nicht als
/// Base64-String: das Projekt hat kein direktes base64-Crate, und ein neues
/// nur für den Avatar-Upload wollte die Aufgabe ausdrücklich vermeiden.
pub fn save_avatar(fish_dir: &Path, id: &str, bytes: &[u8], ext: &str) -> Result<String, String> {
    let ext = ext.trim_start_matches('.').to_lowercase();
    if !AVATAR_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("nicht unterstuetzter Avatar-Typ: {ext}"));
    }
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err(format!(
            "Avatar zu gross ({} Bytes, erlaubt sind {MAX_AVATAR_BYTES})",
            bytes.len()
        ));
    }
    let dir = voice_dir(fish_dir, id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    clear_avatar(fish_dir, id);
    let filename = format!("avatar.{ext}");
    let path = dir.join(&filename);
    std::fs::write(&path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(filename)
}

/// Vorhandene Avatar-Datei(en) einer Stimme entfernen. Keine Fehler, wenn
/// keine existiert — „weg" ist bereits das Ziel.
pub fn clear_avatar(fish_dir: &Path, id: &str) {
    let dir = voice_dir(fish_dir, id);
    for ext in AVATAR_EXTENSIONS {
        let _ = std::fs::remove_file(dir.join(format!("avatar.{ext}")));
    }
}

/// Pfad der Avatar-Datei einer Stimme, falls eine existiert.
pub fn avatar_path(fish_dir: &Path, id: &str) -> Option<PathBuf> {
    let dir = voice_dir(fish_dir, id);
    AVATAR_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("avatar.{ext}")))
        .find(|p| p.exists())
}

/// Den Seed einer gesicherten Stimme lesen, falls sie aus einem Seed
/// hervorgegangen ist (siehe `write_seed_marker`).
pub fn read_seed_marker(fish_dir: &Path, id: &str) -> Option<i64> {
    std::fs::read_to_string(voice_dir(fish_dir, id).join(SEED_FILE))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    /// Sinus von 3 s bei 16 kHz — lang genug für die 400-ms-Messblöcke.
    fn ton(amplitude: f32) -> Vec<f32> {
        (0..48_000)
            .map(|i| amplitude * ((i as f32) * 0.12).sin())
            .collect()
    }

    #[test]
    fn leise_aufnahmen_werden_lauter_laute_leiser() {
        assert!(
            super::normalize_gain(&ton(0.01), 16_000) > 1.0,
            "eine leise Aufnahme muss angehoben werden"
        );
        assert!(
            super::normalize_gain(&ton(0.9), 16_000) < 1.0,
            "eine laute Aufnahme muss abgesenkt werden"
        );
    }

    #[test]
    fn die_verstaerkung_treibt_nie_ins_clipping() {
        // Sehr leise Grundlage mit einem einzelnen lauten Einsatz: die
        // gewünschte Anhebung wäre groß, die Spitze verbietet sie.
        let mut samples = ton(0.001);
        samples[500] = 0.95;
        let gain = super::normalize_gain(&samples, 16_000);
        let peak = samples
            .iter()
            .fold(0.0f32, |acc, s| acc.max((s * gain).abs()));
        assert!(peak <= 1.0, "Spitze nach Normalisierung: {peak}");
    }

    #[test]
    fn stille_bleibt_stille() {
        assert_eq!(super::normalize_gain(&[0.0; 48_000], 16_000), 1.0);
        assert_eq!(super::normalize_gain(&[], 16_000), 1.0);
    }

    use super::*;

    #[test]
    fn voice_ids_are_sanitized_for_filesystem_and_json() {
        assert_eq!(sanitize_voice_id("Patrick"), Some("patrick".into()));
        assert_eq!(sanitize_voice_id("Müller ß"), Some("mueller-ss".into()));
        assert_eq!(
            sanitize_voice_id("  mein  Mikro!!  "),
            Some("mein-mikro".into())
        );
        assert_eq!(sanitize_voice_id("!!!"), None);
        assert_eq!(sanitize_voice_id(""), None);
        let long = "x".repeat(80);
        assert_eq!(sanitize_voice_id(&long).unwrap().len(), 40);
    }

    #[test]
    fn save_list_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        assert!(list_voices(fish).is_empty(), "leer ohne references/");

        let samples = vec![0.1f32; 4 * 16_000]; // 4 s
        save_voice(
            fish,
            "patrick",
            &samples,
            "Hallo, das ist meine Stimme.",
            None,
        )
        .unwrap();
        assert_eq!(list_voices(fish), vec!["patrick".to_string()]);

        // .lab-Inhalt: getrimmt, UTF-8 ohne BOM.
        let lab = std::fs::read(fish.join("references/patrick/sample.lab")).unwrap();
        assert_eq!(lab, b"Hallo, das ist meine Stimme.");
        assert!(!lab.starts_with(&[0xEF, 0xBB, 0xBF]), "kein BOM");

        // WAV ist als 16-kHz-Mono-PCM lesbar.
        let reader = hound::WavReader::open(fish.join("references/patrick/sample.wav")).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);

        delete_voice(fish, "patrick").unwrap();
        assert!(list_voices(fish).is_empty());
        delete_voice(fish, "patrick").unwrap(); // idempotent
    }

    #[test]
    fn too_short_or_untranscribed_references_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let short = vec![0.1f32; 16_000]; // 1 s
        assert!(save_voice(dir.path(), "kurz", &short, "text", None).is_err());
        let ok_len = vec![0.1f32; 4 * 16_000];
        assert!(save_voice(dir.path(), "leer", &ok_len, "   ", None).is_err());
        assert!(
            list_voices(dir.path()).is_empty(),
            "nichts halb Gespeichertes"
        );
    }

    #[test]
    fn a_wav_without_matching_lab_is_not_a_voice() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("references/kaputt");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("sample.wav"), b"RIFF").unwrap();
        assert!(list_voices(dir.path()).is_empty());
    }

    #[test]
    fn arbitrary_wavs_load_as_mono_16k_for_transcription() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("studio.wav");
        // 44,1 kHz stereo, 1 s Sinus links, Stille rechts.
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..44_100u32 {
            let s = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            w.write_sample(s).unwrap(); // links
            w.write_sample(0i16).unwrap(); // rechts
        }
        w.finalize().unwrap();

        let mono = load_wav_mono_16k(&path).unwrap();
        assert!(
            (mono.len() as i64 - 16_000).abs() <= 2,
            "1 s bei 44,1 kHz muss ~16000 Samples ergeben, war {}",
            mono.len()
        );
        // Downmix halbiert die Amplitude (ein stummer Kanal).
        let peak = mono.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(
            peak > 0.05 && peak < 0.2,
            "Peak {peak} außerhalb des Downmix-Erwartungsbereichs"
        );
    }

    #[test]
    fn import_rejects_non_wav_sources() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("nicht-wav.wav");
        std::fs::write(&bogus, b"definitiv kein wav").unwrap();
        assert!(import_voice(dir.path(), "x", &bogus, "text", None).is_err());
        assert!(list_voices(dir.path()).is_empty());
    }

    // ---- Stil-Referenzen & ihre Loeschkaskade ------------------------------

    #[test]
    fn stil_referenz_landet_im_praefixierten_ordner_und_ist_keine_eigene_stimme() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let samples = vec![0.1f32; 4 * 16_000];
        let reference_id =
            save_style_voice(fish, "anna", "fluesternd", &samples, "Ganz leise.", None).unwrap();
        assert_eq!(reference_id, "__style_anna_fluesternd");
        assert!(style_dir(fish, "anna", "fluesternd")
            .join("sample.wav")
            .exists());
        // Stil-Ordner sind intern (Praefix) und tauchen nie in list_voices auf.
        assert!(list_voices(fish).is_empty());
    }

    #[test]
    fn das_loeschen_einer_stimme_reisst_alle_ihre_stilordner_mit() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let samples = vec![0.1f32; 4 * 16_000];
        save_voice(fish, "anna", &samples, "Guten Tag.", None).unwrap();
        save_style_voice(fish, "anna", "fluesternd", &samples, "Ganz leise.", None).unwrap();
        save_style_voice(fish, "anna", "aufgeregt", &samples, "Toll!", None).unwrap();
        // Eine andere Stimme darf von der Kaskade nicht beruehrt werden.
        save_style_voice(fish, "olga", "fluesternd", &samples, "Psst.", None).unwrap();

        delete_voice(fish, "anna").unwrap();

        assert!(!voice_dir(fish, "anna").exists());
        assert!(!style_dir(fish, "anna", "fluesternd").exists());
        assert!(!style_dir(fish, "anna", "aufgeregt").exists());
        assert!(
            style_dir(fish, "olga", "fluesternd").exists(),
            "Stilordner anderer Stimmen bleiben unberuehrt"
        );
    }

    // ---- Avatar -------------------------------------------------------------

    #[test]
    fn avatar_wird_gespeichert_gefunden_und_bei_neuem_typ_ersetzt() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let png_bytes = vec![1u8, 2, 3, 4];
        save_avatar(fish, "anna", &png_bytes, "png").unwrap();
        assert_eq!(
            avatar_path(fish, "anna"),
            Some(voice_dir(fish, "anna").join("avatar.png"))
        );

        // Ersetzen durch einen anderen Dateityp darf keine zwei Avatare
        // hinterlassen.
        let webp_bytes = vec![5u8, 6, 7];
        save_avatar(fish, "anna", &webp_bytes, ".webp").unwrap();
        assert_eq!(
            avatar_path(fish, "anna"),
            Some(voice_dir(fish, "anna").join("avatar.webp"))
        );
        assert!(!voice_dir(fish, "anna").join("avatar.png").exists());

        clear_avatar(fish, "anna");
        assert_eq!(avatar_path(fish, "anna"), None);
    }

    #[test]
    fn zu_grosse_oder_unbekannte_avatare_werden_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let too_big = vec![0u8; MAX_AVATAR_BYTES + 1];
        assert!(save_avatar(fish, "anna", &too_big, "png").is_err());
        assert!(save_avatar(fish, "anna", &[1, 2, 3], "gif").is_err());
        assert_eq!(avatar_path(fish, "anna"), None);
    }

    #[test]
    fn seed_marker_wird_gelesen_wenn_vorhanden() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        assert_eq!(read_seed_marker(fish, "seedvoice"), None);
        std::fs::create_dir_all(voice_dir(fish, "seedvoice")).unwrap();
        write_seed_marker(fish, "seedvoice", 42);
        assert_eq!(read_seed_marker(fish, "seedvoice"), Some(42));
    }
}
