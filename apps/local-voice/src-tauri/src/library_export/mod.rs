//! Export aus Bibliothek und Vorlesen (lva#236): jede Fassung eines Buchs
//! einzeln als Datei.
//!
//! - Text (Original, Übersetzung, Kurzversion, KI-Dokument): EPUB (eigener
//!   Schreiber, [`epub`]), PDF und Word (gestalteter Dokumentexport mit dem
//!   neutralen Briefkopf: ein Buch trägt keinen Firmenkopf), Markdown, Text.
//!   Steuerzeichen des Vorlesens fallen vorher weg ([`clean`]).
//! - Hörbuch: Originaldateien unverändert, oder über ffmpeg zu EINER Datei
//!   (M4B mit Kapitelmarken je Teil, oder MP3). ffmpeg ist das des Nutzers
//!   (PATH, wie beim Hörbuch-Import); ohne ffmpeg bleibt „Original“.
//!
//! Die Oberfläche löst den Text der Fassung selbst auf (Seite, Übersetzungs-
//! Cache, KI-Dokument) und reicht ihn herein: so gibt es EINEN Exportweg für
//! alles, was sie anzeigt, auch für eine eben korrigierte Übersetzung.
//! Geschrieben wird über `<ziel>.part` und Umbenennen.

pub mod clean;
pub mod epub;

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Angaben des Buchs für Kopf und Cover der Exportdatei.
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct LibraryExportMeta {
    pub title: String,
    pub authors: Vec<String>,
    /// Sprachcode der Fassung (`de`, `en`); fehlt: unbekannt.
    pub language: Option<String>,
    /// Absoluter Pfad des Covers (mit oder ohne `?v=`), nur für EPUB.
    pub cover_path: Option<String>,
}

/// Ein Teil des Hörbuchs als Kapitelmarke (M4B).
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct LibraryExportChapter {
    pub title: String,
    pub duration_ms: Option<f64>,
}

const TEXT_FORMATS: [&str; 5] = ["epub", "pdf", "docx", "md", "txt"];

/// Eine Textfassung exportieren. Das Format ergibt sich aus der Endung
/// (`epub`, `pdf`, `docx`, `md`, `txt`). Rückgabe: der geschriebene Pfad.
#[tauri::command]
#[specta::specta]
pub async fn library_export_text(
    meta: LibraryExportMeta,
    markdown: String,
    path: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || export_text(&meta, &markdown, Path::new(&path)))
        .await
        .map_err(|e| format!("export_failed: {e}"))?
}

fn export_text(meta: &LibraryExportMeta, markdown: &str, path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !TEXT_FORMATS.contains(&ext.as_str()) {
        return Err("export_unsupported_format".into());
    }
    let body = clean::for_export(markdown);
    if body.trim().is_empty() {
        return Err("export_empty".into());
    }
    if ext == "epub" {
        let cover = meta.cover_path.as_deref().and_then(read_cover);
        let modified = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let id = format!("urn:ulid:{}", ulid::Ulid::new());
        let bytes = epub::build(
            &epub::EpubMeta {
                id: &id,
                title: meta.title.trim(),
                authors: &meta.authors,
                language: meta.language.as_deref().unwrap_or(""),
                modified: &modified,
            },
            &body,
            cover
                .as_ref()
                .map(|(bytes, media)| epub::EpubCover {
                    bytes,
                    media_type: media,
                })
                .as_ref(),
        )?;
        write_atomic(path, &bytes)?;
        return Ok(path.to_string_lossy().into_owned());
    }
    // Ein Kopf für PDF/Word: Titel, darunter die Autoren. Beginnt der Text
    // schon mit dem Titel als Hauptüberschrift, keine zweite davor.
    let byline = meta
        .authors
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let first_heading = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.strip_prefix("# "))
        .map(str::trim);
    let markdown = if meta.title.trim().is_empty()
        || first_heading.is_some_and(|h| h.eq_ignore_ascii_case(meta.title.trim()))
    {
        format!("{body}\n")
    } else if byline.is_empty() {
        format!("# {}\n\n{body}\n", meta.title.trim())
    } else {
        format!("# {}\n\n*{byline}*\n\n{body}\n", meta.title.trim())
    };
    let neutral = crate::managers::letterhead::builtins()
        .into_iter()
        .next()
        .ok_or("export_failed: kein Briefkopf")?;
    crate::managers::meetings::doc_export::write(
        path,
        &markdown,
        &crate::managers::meetings::doc_export::Design {
            letterhead: &neutral,
            logo: None,
        },
    )?;
    Ok(path.to_string_lossy().into_owned())
}

/// Cover lesen (`?v=` am Pfad abschneiden); nur JPEG und PNG.
fn read_cover(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let clean = path.split('?').next().unwrap_or(path);
    let bytes = std::fs::read(clean).ok()?;
    let media = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else {
        return None;
    };
    Some((bytes, media))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("export_failed: {e}"))?;
    }
    let part = path.with_extension(format!(
        "{}.part",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&part, bytes).map_err(|e| format!("export_failed: {e}"))?;
    std::fs::rename(&part, path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("export_failed: {e}")
    })
}

/// Ist ffmpeg erreichbar? Dann bietet der Exportdialog M4B und MP3 an.
#[tauri::command]
#[specta::specta]
pub async fn library_ffmpeg_available() -> bool {
    tauri::async_runtime::spawn_blocking(|| {
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.arg("-version");
        no_window(&mut cmd);
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Hörbuch exportieren.
///
/// - `original`: ein Teil → Kopie nach `out_path` (Endung des Originals);
///   mehrere Teile → `out_path` ist ein Ordner, darin „01 – Titel.ext“ …
/// - `m4b` / `mp3`: alle Teile über ffmpeg in EINE Datei; M4B mit einer
///   Kapitelmarke je Teil, wenn alle Teile eine Dauer haben.
///
/// Rückgabe: die geschriebenen Dateien.
#[tauri::command]
#[specta::specta]
pub async fn library_export_audio(
    paths: Vec<String>,
    chapters: Vec<LibraryExportChapter>,
    out_path: String,
    format: String,
    title: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let parts: Vec<PathBuf> = paths
            .iter()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        if parts.is_empty() {
            return Err("export_no_audio".to_string());
        }
        if let Some(missing) = parts.iter().find(|p| !p.exists()) {
            return Err(format!("export_audio_missing: {}", missing.display()));
        }
        match format.as_str() {
            "original" => copy_original(&parts, Path::new(&out_path)),
            "m4b" | "mp3" => {
                convert(&parts, &chapters, Path::new(&out_path), &format, &title).map(|p| vec![p])
            }
            _ => Err("export_unsupported_format".to_string()),
        }
    })
    .await
    .map_err(|e| format!("export_failed: {e}"))?
}

/// Endung anhängen, statt sie zu ersetzen: „Dr. Faust – Hörbuch“ bleibt ganz
/// (`with_extension` hielte „ Faust – Hörbuch“ für die Endung).
pub(crate) fn append_ext(out: &Path, ext: &str) -> PathBuf {
    let suffix = format!(".{ext}");
    if out
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        return out.to_path_buf();
    }
    let mut s = out.as_os_str().to_owned();
    s.push(&suffix);
    PathBuf::from(s)
}

fn copy_original(parts: &[PathBuf], out: &Path) -> Result<Vec<String>, String> {
    if parts.len() == 1 {
        let ext = parts[0].extension().and_then(|e| e.to_str()).unwrap_or("");
        let target = if ext.is_empty() {
            out.to_path_buf()
        } else {
            append_ext(out, ext)
        };
        copy_atomic(&parts[0], &target)?;
        return Ok(vec![target.to_string_lossy().into_owned()]);
    }
    std::fs::create_dir_all(out).map_err(|e| format!("export_failed: {e}"))?;
    let width = parts.len().to_string().len().max(2);
    let mut written = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let stem = part.file_stem().and_then(|s| s.to_str()).unwrap_or("Teil");
        let ext = part.extension().and_then(|e| e.to_str()).unwrap_or("");
        let name = clean::safe_file_name(&format!("{:0width$} – {stem}", i + 1));
        let target = out.join(if ext.is_empty() {
            name
        } else {
            format!("{name}.{ext}")
        });
        copy_atomic(part, &target)?;
        written.push(target.to_string_lossy().into_owned());
    }
    Ok(written)
}

fn copy_atomic(from: &Path, to: &Path) -> Result<(), String> {
    let part = to.with_extension(format!(
        "{}.part",
        to.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::copy(from, &part).map_err(|e| format!("export_failed: {e}"))?;
    std::fs::rename(&part, to).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("export_failed: {e}")
    })
}

/// Kapitel-Metadaten für ffmpeg (`;FFMETADATA1`), wenn jede Dauer bekannt ist.
pub(crate) fn ffmetadata(title: &str, chapters: &[LibraryExportChapter]) -> Option<String> {
    if chapters.is_empty()
        || chapters
            .iter()
            .any(|c| !c.duration_ms.is_some_and(|d| d > 0.0))
    {
        return None;
    }
    let esc = |s: &str| {
        s.chars()
            .flat_map(|c| match c {
                '=' | ';' | '#' | '\\' | '\n' => vec!['\\', c],
                c => vec![c],
            })
            .collect::<String>()
    };
    let mut out = format!(";FFMETADATA1\ntitle={}\n", esc(title));
    let mut start = 0u64;
    for c in chapters {
        let end = start + c.duration_ms.unwrap_or(0.0).round() as u64;
        out.push_str(&format!(
            "\n[CHAPTER]\nTIMEBASE=1/1000\nSTART={start}\nEND={end}\ntitle={}\n",
            esc(c.title.trim())
        ));
        start = end;
    }
    Some(out)
}

fn convert(
    parts: &[PathBuf],
    chapters: &[LibraryExportChapter],
    out: &Path,
    format: &str,
    title: &str,
) -> Result<String, String> {
    let target = append_ext(out, format);
    // Der Concat-Demuxer braucht gleich kodierte Teile; gemischte Hörbücher
    // (etwa m4a und mp3) lassen sich nur als Originaldateien weitergeben.
    let ext_of = |p: &PathBuf| {
        p.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    if parts.iter().any(|p| ext_of(p) != ext_of(&parts[0])) {
        return Err("export_mixed_audio".to_string());
    }
    let work = tempfile::tempdir().map_err(|e| format!("export_failed: {e}"))?;
    // Concat-Liste: ffmpeg verlangt einfache Anführungszeichen maskiert.
    let list: String = parts
        .iter()
        .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\'', "'\\''")))
        .collect();
    let list_path = work.path().join("parts.txt");
    std::fs::write(&list_path, list).map_err(|e| format!("export_failed: {e}"))?;
    let part = work.path().join(format!("out.{format}"));
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ])
    .arg(&list_path);
    let meta = if format == "m4b" {
        ffmetadata(title, chapters)
    } else {
        None
    };
    if let Some(meta) = &meta {
        let meta_path = work.path().join("chapters.txt");
        std::fs::write(&meta_path, meta).map_err(|e| format!("export_failed: {e}"))?;
        cmd.arg("-i")
            .arg(&meta_path)
            .args(["-map_metadata", "1", "-map_chapters", "1"]);
    }
    cmd.args(["-map", "0:a", "-vn", "-metadata"])
        .arg(format!("title={title}"));
    if format == "m4b" {
        cmd.args([
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]);
    } else {
        cmd.args(["-c:a", "libmp3lame", "-b:a", "128k", "-f", "mp3"]);
    }
    cmd.arg(&part);
    no_window(&mut cmd);
    let output = cmd
        .output()
        .map_err(|_| "export_ffmpeg_missing".to_string())?;
    if !output.status.success() || !part.exists() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        return Err(format!("export_ffmpeg_failed: {tail}"));
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("export_failed: {e}"))?;
    }
    // Ein Umbenennen über Laufwerke hinweg scheitert: dann kopieren.
    if std::fs::rename(&part, &target).is_err() {
        copy_atomic(&part, &target)?;
    }
    Ok(target.to_string_lossy().into_owned())
}

fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_ext_keeps_dotted_titles() {
        assert_eq!(
            append_ext(Path::new("/x/Dr. Faust – Hörbuch"), "m4b"),
            PathBuf::from("/x/Dr. Faust – Hörbuch.m4b")
        );
        assert_eq!(
            append_ext(Path::new("/x/Vol. 2.MP3"), "mp3"),
            PathBuf::from("/x/Vol. 2.MP3")
        );
    }

    fn ch(title: &str, d: Option<f64>) -> LibraryExportChapter {
        LibraryExportChapter {
            title: title.into(),
            duration_ms: d,
        }
    }

    #[test]
    fn ffmetadata_needs_every_duration() {
        assert!(ffmetadata("T", &[ch("a", Some(1000.0)), ch("b", None)]).is_none());
        assert!(ffmetadata("T", &[]).is_none());
        let meta =
            ffmetadata("A=B", &[ch("Eins", Some(1500.0)), ch("Zwei;", Some(500.0))]).unwrap();
        assert!(meta.starts_with(";FFMETADATA1\ntitle=A\\=B\n"));
        assert!(meta.contains("START=0\nEND=1500\ntitle=Eins"));
        assert!(meta.contains("START=1500\nEND=2000\ntitle=Zwei\\;"));
    }

    #[test]
    fn copies_parts_into_folder() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        std::fs::write(&a, b"A").unwrap();
        std::fs::write(&b, b"B").unwrap();
        let out = dir.path().join("Buch");
        let written = copy_original(&[a.clone(), b], &out).unwrap();
        assert_eq!(written.len(), 2);
        assert!(out.join("01 – a.mp3").exists());
        assert!(out.join("02 – b.mp3").exists());
        // Der Dialog übergibt den Namen ohne Endung; die Originaldatei behält ihre.
        let single = copy_original(&[a.clone()], &dir.path().join("Einzeln")).unwrap();
        assert!(single[0].ends_with("Einzeln.mp3"));
        let dotted = copy_original(&[a], &dir.path().join("Dr. Faust – Hörbuch")).unwrap();
        assert!(dotted[0].ends_with("Dr. Faust – Hörbuch.mp3"));
    }

    #[test]
    fn text_export_rejects_unknown_format_and_empty_text() {
        let meta = LibraryExportMeta {
            title: "T".into(),
            authors: vec![],
            language: None,
            cover_path: None,
        };
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            export_text(&meta, "x", &dir.path().join("a.odt")).unwrap_err(),
            "export_unsupported_format"
        );
        assert_eq!(
            export_text(&meta, "{{S. 3}}", &dir.path().join("a.epub")).unwrap_err(),
            "export_empty"
        );
        let out = export_text(&meta, "# A\n\nText", &dir.path().join("a.epub")).unwrap();
        assert!(Path::new(&out).exists());
    }
}
