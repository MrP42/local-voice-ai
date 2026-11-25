//! Hoerbuch-Uebernahme und dauerhaft nachholbares Entfernen von Hilfsauftraegen.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct TranscriptAdoption {
    pub page_id: String,
    pub append_page: Option<String>,
    pub sync: Vec<SyncPoint>,
    pub durations: Vec<(String, Option<f64>)>,
    pub edition_title: String,
}

fn unlink(audio: &mut AudioVersion, ids: &[String]) {
    if audio.transcript_job.as_ref().is_some_and(|id| ids.contains(id)) {
        audio.transcript_job = None;
    }
    for track in &mut audio.tracks {
        if track.transcript_job.as_ref().is_some_and(|id| ids.contains(id)) {
            track.transcript_job = None;
        }
        track.transcript_jobs.retain(|p| !ids.contains(&p.job));
    }
}

/// Aendert ausschliesslich die Transkriptfassung am aktuell gespeicherten Buch.
pub(crate) fn apply_transcript(
    book: &mut LibraryBook,
    adoption: TranscriptAdoption,
    ids: &[String],
) -> Result<(), String> {
    let audio = book.audio.as_mut().ok_or("audio_missing")?;
    if let Some(page) = &adoption.append_page {
        if !book.text.as_ref().is_some_and(|t| t.page_id == *page && t.origin == "transcript")
            || adoption.page_id != *page {
            return Err("stale_audiobook_completion".into());
        }
    }
    let text = TextVersion {
        page_id: adoption.page_id.clone(), origin: "transcript".into(),
        source_name: Some(audio.file.clone()),
    };
    let separate = book.text.is_some() && adoption.append_page.is_none();
    if separate {
        // Ein inzwischen angehaengter eigener Text und alle Fassungen bleiben erhalten.
        let now = now_ms();
        book.editions.push(Edition {
            id: format!("ed_{}", adoption.page_id),
            lang: editions::lang_code(book.language.as_deref().unwrap_or_default()),
            title: adoption.edition_title, text: Some(text), origin: "import".into(),
            added_ms: now, updated: now, audio_paths: Vec::new(),
            ..Edition::default()
        });
    } else {
        book.text = Some(text);
        audio.sync = adoption.sync;
    }
    for track in &mut audio.tracks {
        if track.duration_ms.is_none() {
            track.duration_ms = adoption.durations.iter().find(|(file, _)| file == &track.file)
                .and_then(|(_, duration)| *duration);
        }
    }
    if audio.tracks.is_empty() && audio.duration_ms.is_none() {
        audio.duration_ms = adoption.durations.iter().find(|(file, _)| file == &audio.file)
            .and_then(|(_, duration)| *duration);
    }
    unlink(audio, ids);
    Ok(())
}

const REMOVALS: &str = ".transcription-removals.json";
type Removals = BTreeMap<String, Vec<String>>;

fn read_removals(root: &Path) -> Result<Removals, String> {
    match std::fs::read(root.join(REMOVALS)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "invalid_transcription_removals".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Removals::new()),
        Err(_) => Err("transcription_removals_unreadable".into()),
    }
}

fn write_removals(root: &Path, pending: &Removals) -> Result<(), String> {
    write_atomic(&root.join(REMOVALS), &serde_json::to_vec(pending).map_err(|e| e.to_string())?)
}

/// Der Aufrufer haelt die Bibliothekssperre. Auch bei Absturz zwischen Buchspeichern
/// und Loeschen bleibt die Absicht erhalten; fehlende Hilfsauftraege sind erledigt.
pub(crate) fn remove_jobs(
    root: &Path,
    book_id: &str,
    ids: &[String],
    mut delete: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let mut pending = read_removals(root)?;
    if !ids.is_empty() {
        let jobs = pending.entry(book_id.to_owned()).or_default();
        jobs.extend_from_slice(ids);
        jobs.sort();
        jobs.dedup();
        write_removals(root, &pending)?;
    }
    for (id, jobs) in pending.clone() {
        let dir = book_dir(root, &id)?;
        if dir.join("buch.json").try_exists().map_err(|_| "book_unreadable")? {
            let mut book = read_book(&dir).ok_or("book_unreadable")?;
            if let Some(audio) = &mut book.audio { unlink(audio, &jobs); }
            for edition in &mut book.editions {
                if let Some(audio) = &mut edition.audio { unlink(audio, &jobs); }
            }
            // Erst die Verweise sicher entfernen, dann darf ein Hilfsauftrag weg.
            write_book(&dir, &book)?;
        }
        for job in &jobs { delete(job)?; }
        pending.remove(&id);
        write_removals(root, &pending)?;
    }
    Ok(())
}

pub(crate) fn root(app: &AppHandle) -> Result<PathBuf, String> { library_root(app) }

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> LibraryBook {
        LibraryBook { id: "book_synthetic".into(), title: "Synthetic".into(),
            audio: Some(AudioVersion { file: "synthetic.wav".into(),
                transcript_job: Some("job_a".into()), ..Default::default() }),
            ..Default::default() }
    }

    #[test]
    fn adoption_fills_missing_durations_by_file() {
        for multipart in [false, true] {
            let mut current = book();
            if multipart {
                current.audio.as_mut().unwrap().tracks = vec![
                    AudioTrack { file: "second.wav".into(), ..Default::default() },
                    AudioTrack { file: "first.wav".into(), duration_ms: Some(7000.0), ..Default::default() },
                    AudioTrack { file: "unknown.wav".into(), ..Default::default() },
                ];
            }
            apply_transcript(&mut current, TranscriptAdoption {
                page_id: "synthetic_page".into(), append_page: None, sync: vec![],
                durations: vec![("first.wav".into(), Some(9000.0)),
                    ("second.wav".into(), Some(15000.0)),
                    ("synthetic.wav".into(), Some(20000.0))],
                edition_title: "Synthetic".into(),
            }, &["job_a".into()]).unwrap();
            let audio = current.audio.unwrap();
            if multipart {
                assert_eq!(audio.tracks[0].duration_ms, Some(15000.0));
                assert_eq!(audio.tracks[1].duration_ms, Some(7000.0));
                assert_eq!(audio.tracks[2].duration_ms, None);
                assert_eq!(audio.duration_ms, None);
            } else {
                assert_eq!(audio.duration_ms, Some(20000.0));
            }
        }
    }

    #[test]
    fn adoption_preserves_a_text_added_after_frontend_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut current = book();
        let path = book_dir(dir.path(), &current.id).unwrap();
        write_book(&path, &current).unwrap();
        let frontend_snapshot = read_book(&path).unwrap();
        // Nach dem Frontend-Lesen wird ein eigener Text samt Fassung gespeichert.
        current.text = Some(TextVersion { page_id: "own_page".into(), origin: "document".into(), ..Default::default() });
        editions::normalize(&mut current);
        write_book(&path, &current).unwrap();
        let _guard = lock();
        let mut current = read_book(&path).unwrap();
        let before = current.clone();
        apply_transcript(&mut current, TranscriptAdoption {
            page_id: "transcript_page".into(), append_page: None, sync: vec![],
            durations: vec![], edition_title: "Synthetic transcript".into(),
        }, &["job_a".into()]).unwrap();
        assert_eq!(current.text, before.text);
        assert_eq!(&current.editions[..before.editions.len()], before.editions.as_slice());
        assert_eq!(current.editions.last().unwrap().text.as_ref().unwrap().page_id, "transcript_page");
        assert!(current.audio.as_ref().unwrap().transcript_job.is_none());
        write_book(&path, &current).unwrap();
        assert_eq!(read_book(&path).unwrap().text, before.text);
        // Gegenprobe: Speichern des Frontend-Snapshots verliert den eigenen Text.
        write_book(&path, &frontend_snapshot).unwrap();
        assert_ne!(read_book(&path).unwrap().text, before.text);
    }

    #[test]
    fn removal_replays_after_crash_and_partly_deleted_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let b = book();
        let path = book_dir(root, &b.id).unwrap();
        write_book(&path, &b).unwrap();
        let ids = vec!["job_a".into(), "job_missing".into(), "job_b".into()];
        let mut existing = std::collections::BTreeSet::from(["job_a".to_string(), "job_b".to_string()]);
        let result = remove_jobs(root, &b.id, &ids, |id| {
            assert!(read_book(&path).unwrap().audio.unwrap().transcript_job.is_none());
            if id == "job_b" { return Err("synthetic_crash".into()); }
            existing.remove(id);
            Ok(())
        });
        assert!(result.is_err());
        assert!(!existing.contains("job_a"));
        assert!(!read_removals(root).unwrap().is_empty());
        // Neuer Aufruf/Start kennt nur die dauerhafte Absicht, keine UI-IDs mehr.
        remove_jobs(root, "", &[], |id| { existing.remove(id); Ok(()) }).unwrap();
        assert!(existing.is_empty());
        assert!(read_removals(root).unwrap().is_empty());
        remove_jobs(root, &b.id, &ids, |_| Ok(())).unwrap();
    }

    #[test]
    fn unreadable_book_never_deletes_helpers_and_keeps_intent() {
        let dir = tempfile::tempdir().unwrap();
        let b = book();
        let path = book_dir(dir.path(), &b.id).unwrap();
        write_book(&path, &b).unwrap();
        std::fs::write(path.join("buch.json"), b"synthetic-invalid-json").unwrap();
        let mut deleted = false;
        assert!(remove_jobs(dir.path(), &b.id, &["job_a".into()], |_| { deleted = true; Ok(()) }).is_err());
        assert!(!deleted);
        assert!(!read_removals(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn failed_book_write_keeps_helpers_until_replay() {
        let dir = tempfile::tempdir().unwrap();
        let b = book();
        let path = book_dir(dir.path(), &b.id).unwrap();
        write_book(&path, &b).unwrap();
        // Genau die temporaere Buchdatei ist unbeschreibbar; das Journal geht.
        let blocker = path.join(format!("buch.tmp-{}", std::process::id()));
        std::fs::create_dir(&blocker).unwrap();
        let mut deletes = 0;
        assert!(remove_jobs(dir.path(), &b.id, &["job_a".into()], |_| { deletes += 1; Ok(()) }).is_err());
        assert_eq!(deletes, 0);
        assert_eq!(read_book(&path).unwrap().audio.unwrap().transcript_job.as_deref(), Some("job_a"));
        assert!(!read_removals(dir.path()).unwrap().is_empty());
        std::fs::remove_dir(&blocker).unwrap();
        remove_jobs(dir.path(), "", &[], |_| { deletes += 1; Ok(()) }).unwrap();
        assert_eq!(deletes, 1);
        assert!(read_book(&path).unwrap().audio.unwrap().transcript_job.is_none());
    }
}
