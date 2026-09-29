//! Tauri-Commands fuer Suche und Ordner der Besprechungsliste (M4, P4d).
//! Duenne Huelle ueber die Store-Funktionen aus `meetings::search::index`
//! (P4a); die Fehlercodes des Stores (`folder_not_found`,
//! `folder_name_invalid`, `folder_name_taken`, `meeting_not_found`, ...) gehen
//! unveraendert an die UI, die sie uebersetzt.
//!
//! Aufbau (P4b haengt `meeting_index_status` und den Modell-Download an):
//! 1. Logik ohne Tauri (testbar mit einem Store im Tempdir)
//! 2. Commands: Suche
//! 3. Commands: Ordner
//! 4. (P4b) Commands: Index-Status
//!
//! Datenschutz: Suchtext und Ordnernamen stehen in keiner Fehlermeldung und in
//! keinem Log.

use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::search::index::{Folder, MeetingFilter, MeetingSearchPage};
use crate::managers::meetings::store::MeetingStore;

// ---------------------------------------------------------------------------
// 1. Logik ohne Tauri
// ---------------------------------------------------------------------------

/// Laengere Suchtexte kappt der Command (ein eingefuegter Absatz soll keine
/// FTS-Abfrage mit hunderten Termen erzeugen).
pub const MAX_QUERY_CHARS: usize = 200;
/// Groesste Seite, die die Liste auf einmal holen darf.
pub const MAX_PAGE: u32 = 100;
/// Herkunftswerte von `meetings.source`, die der Filter kennt.
const SOURCES: [&str; 3] = ["live", "import", "subtitle"];

fn normalize_query(query: &str) -> String {
    query.trim().chars().take(MAX_QUERY_CHARS).collect()
}

fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_PAGE)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Leere Felder der UI (`""`) bedeuten "kein Filter"; unbekannte Herkunft oder
/// ein Zeitraum mit Ende vor dem Anfang ist `filter_invalid`.
fn normalize_filter(filter: MeetingFilter) -> Result<MeetingFilter, String> {
    let source = non_empty(filter.source);
    if source.as_deref().is_some_and(|s| !SOURCES.contains(&s)) {
        return Err("filter_invalid".to_string());
    }
    if let (Some(from), Some(to)) = (filter.from, filter.to) {
        if from > to {
            return Err("filter_invalid".to_string());
        }
    }
    Ok(MeetingFilter {
        folder_id: non_empty(filter.folder_id),
        from: filter.from,
        to: filter.to,
        source,
        has_notes: filter.has_notes.filter(|v| *v),
        person_id: non_empty(filter.person_id),
    })
}

fn search(
    store: &MeetingStore,
    query: &str,
    filter: MeetingFilter,
    offset: u32,
    limit: u32,
) -> Result<MeetingSearchPage, String> {
    let filter = normalize_filter(filter)?;
    store
        .search_meetings(&normalize_query(query), &filter, offset, clamp_limit(limit))
        .map_err(|e| e.to_string())
}

fn save_folder(
    store: &MeetingStore,
    id: Option<String>,
    name: &str,
    color: Option<String>,
) -> Result<Folder, String> {
    let id = non_empty(id);
    store
        .folder_save(id.as_deref(), name, color.as_deref())
        .map_err(|e| e.to_string())
}

fn set_folders(
    store: &MeetingStore,
    meeting_id: &str,
    folder_ids: &[String],
) -> Result<(), String> {
    store
        .set_meeting_folders(meeting_id, folder_ids)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 2. Commands: Suche
// ---------------------------------------------------------------------------

/// Listensuche: ohne Suchterme eine reine Filterliste (neueste zuerst), sonst
/// Besprechungen mit Treffern im Such-Index samt Snippet (`<mark>`). Findet nur
/// Besprechungen, die der Indexer (P4b) schon erfasst hat, auch fuer den Titel.
#[tauri::command]
#[specta::specta]
pub async fn meetings_search(
    store: State<'_, Arc<MeetingStore>>,
    query: String,
    filter: MeetingFilter,
    offset: u32,
    limit: u32,
) -> Result<MeetingSearchPage, String> {
    search(&store, &query, filter, offset, limit)
}

// ---------------------------------------------------------------------------
// 3. Commands: Ordner
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn meeting_folders_list(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<Folder>, String> {
    store.folders_list().map_err(|e| e.to_string())
}

/// Legt einen Ordner an (`id` leer) oder benennt ihn um.
#[tauri::command]
#[specta::specta]
pub async fn meeting_folders_save(
    store: State<'_, Arc<MeetingStore>>,
    id: Option<String>,
    name: String,
    color: Option<String>,
) -> Result<Folder, String> {
    save_folder(&store, id, &name, color)
}

/// Loescht einen Ordner; die Besprechungen darin bleiben.
#[tauri::command]
#[specta::specta]
pub async fn meeting_folders_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    store.folder_delete(&id).map_err(|e| e.to_string())
}

/// Setzt die Ordner einer Besprechung auf genau `folder_ids` (n:m).
#[tauri::command]
#[specta::specta]
pub async fn meetings_set_folders(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    folder_ids: Vec<String>,
) -> Result<(), String> {
    set_folders(&store, &meeting_id, &folder_ids)
}

/// Ordner-IDs einer Besprechung (Vorbelegung des Ordner-Dialogs).
#[tauri::command]
#[specta::specta]
pub async fn meetings_get_folders(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<String>, String> {
    store
        .meeting_folder_ids(&meeting_id)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 4. Index-Status (P4b)
// ---------------------------------------------------------------------------

/// Stand des Such-Index fuer die Einstellungszeile "Semantische Suche".
/// Besprechungen: `total` fertige, davon `lexical_done` mit Stichwortindex
/// (inkl. eingebetteter) und `embedded` mit allen Vektoren.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct IndexStatus {
    pub total: u32,
    pub pending: u32,
    pub lexical_done: u32,
    pub embedded: u32,
    pub chunks: u32,
    pub vectors: u32,
    /// Embedding-Modell heruntergeladen.
    pub model_ready: bool,
    /// Download laeuft.
    pub downloading: bool,
    /// Einstellung `meeting_semantic_search`.
    pub enabled: bool,
    /// Der Indexer hat Arbeit (Queue, Entprellung oder fehlende Vektoren).
    pub running: bool,
    /// Der Embedding-Server laeuft gerade.
    pub server_running: bool,
    /// Code des letzten Fehlers der Vektorstufe (`memory_low`, `no_model`, ...).
    pub last_error: Option<String>,
}

/// Zaehlt den Stand aus dem Store; der Rest kommt vom Aufrufer.
fn index_status_from(store: &MeetingStore, model: &str) -> Result<IndexStatus, String> {
    let counts = store.index_counts().map_err(|e| e.to_string())?;
    Ok(IndexStatus {
        total: counts.meetings,
        pending: counts.pending,
        lexical_done: counts.lexical + counts.embedded,
        embedded: counts.embedded,
        chunks: counts.chunks,
        vectors: store.vector_count(model).map_err(|e| e.to_string())?,
        ..IndexStatus::default()
    })
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_index_status(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
) -> Result<IndexStatus, String> {
    use crate::managers::llm;
    use crate::managers::meetings::search::indexer::MeetingIndexer;
    use tauri::Manager;
    let mut status = index_status_from(&store, llm::EMBED_MODEL_ID)?;
    status.model_ready = llm::embedding_model_ready(llm::EMBED_MODEL_ID);
    status.downloading = llm::embedding_model_downloading(llm::EMBED_MODEL_ID);
    status.enabled = crate::settings::get_settings(&app).meeting_semantic_search;
    status.server_running = llm::embedding_running();
    if let Some(indexer) = app.try_state::<Arc<MeetingIndexer>>() {
        status.running = indexer.busy();
        status.last_error = indexer.last_error();
    }
    Ok(status)
}

/// Laedt das Embedding-Modell (635 MB) -- nur auf Knopfdruck (E6). Danach
/// holt der Indexer die Vektoren im Hintergrund nach.
#[tauri::command]
#[specta::specta]
pub async fn meeting_embedding_model_download(
    app: tauri::AppHandle,
    runtime: State<'_, Arc<crate::managers::llm::LlmRuntimeManager>>,
) -> Result<(), String> {
    use crate::managers::meetings::search::indexer::{submit, IndexJob};
    runtime
        .download(crate::managers::llm::EMBED_MODEL_ID)
        .await?;
    submit(&app, IndexJob::EmbedPending);
    Ok(())
}

/// Einstellung `meeting_semantic_search`. Ausschalten beendet einen laufenden
/// Embedding-Server sofort; Einschalten stoesst das Nachholen an.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_semantic_search_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    use crate::managers::meetings::search::indexer::{submit, IndexJob};
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_semantic_search = enabled;
    crate::settings::write_settings(&app, settings);
    if enabled {
        submit(&app, IndexJob::EmbedPending);
    } else {
        crate::managers::llm::stop_embedding();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::chunking::ChunkSource;
    use crate::managers::meetings::search::index::tests::{draft, ready_meeting, state, tmp_store};
    use crate::managers::meetings::search::index::STATUS_LEXICAL;

    fn filter() -> MeetingFilter {
        MeetingFilter::default()
    }

    #[test]
    fn query_is_trimmed_and_capped() {
        assert_eq!(normalize_query("  Budget  "), "Budget");
        let long = "ä".repeat(MAX_QUERY_CHARS + 50);
        assert_eq!(normalize_query(&long).chars().count(), MAX_QUERY_CHARS);
    }

    #[test]
    fn limit_is_clamped_to_one_page() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(25), 25);
        assert_eq!(clamp_limit(10_000), MAX_PAGE);
    }

    #[test]
    fn empty_filter_fields_mean_no_filter() {
        let f = normalize_filter(MeetingFilter {
            folder_id: Some("  ".into()),
            source: Some(String::new()),
            has_notes: Some(false),
            ..filter()
        })
        .unwrap();
        assert_eq!(f.folder_id, None);
        assert_eq!(f.source, None);
        assert_eq!(f.has_notes, None);
    }

    #[test]
    fn unknown_source_and_reversed_range_are_rejected() {
        let bad_source = MeetingFilter {
            source: Some("zoom".into()),
            ..filter()
        };
        assert_eq!(normalize_filter(bad_source).unwrap_err(), "filter_invalid");
        let reversed = MeetingFilter {
            from: Some(20),
            to: Some(10),
            ..filter()
        };
        assert_eq!(normalize_filter(reversed).unwrap_err(), "filter_invalid");
        let ok = MeetingFilter {
            source: Some("import".into()),
            from: Some(10),
            to: Some(20),
            ..filter()
        };
        assert_eq!(
            normalize_filter(ok).unwrap().source.as_deref(),
            Some("import")
        );
    }

    #[test]
    fn search_returns_marked_snippet_for_indexed_meeting() {
        let (_dir, s) = tmp_store();
        let hit = ready_meeting(&s, "Kundentermin", 1_000);
        let _other = ready_meeting(&s, "Teamrunde", 2_000);
        s.replace_meeting_chunks(
            &hit.id,
            &[ChunkSource::Transcript],
            &[draft(ChunkSource::Transcript, "Das Budget für 2027 steht")],
            &state(STATUS_LEXICAL),
        )
        .unwrap();
        let page = search(&s, "  budget ", filter(), 0, 25).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].meeting.id, hit.id);
        let snippet = page.items[0].snippet.as_deref().unwrap();
        assert!(snippet.contains("<mark>"), "{snippet}");
        assert_eq!(page.items[0].hit_source, Some(ChunkSource::Transcript));
    }

    #[test]
    fn empty_query_lists_meetings_and_respects_folder_filter() {
        let (_dir, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let _b = ready_meeting(&s, "B", 2_000);
        assert_eq!(search(&s, "   ", filter(), 0, 25).unwrap().total, 2);
        let folder = save_folder(&s, Some(String::new()), "Vertrieb", None).unwrap();
        set_folders(&s, &a.id, &[folder.id.clone()]).unwrap();
        let page = search(
            &s,
            "",
            MeetingFilter {
                folder_id: Some(folder.id.clone()),
                ..filter()
            },
            0,
            25,
        )
        .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].meeting.id, a.id);
        assert!(page.items[0].snippet.is_none());
    }

    /// Stand P4d: der Titel steht erst mit dem Indexer (P4b) im Such-Index;
    /// eine noch nicht indexierte Besprechung findet die Suche nicht.
    #[test]
    fn unindexed_meeting_is_not_found_by_title_yet() {
        let (_dir, s) = tmp_store();
        ready_meeting(&s, "Kundentermin Meyer", 1_000);
        assert_eq!(search(&s, "Meyer", filter(), 0, 25).unwrap().total, 0);
    }

    #[test]
    fn folders_are_assigned_many_to_many_and_deleting_keeps_meetings() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "A", 1_000);
        let f1 = save_folder(&s, None, "Vertrieb", None).unwrap();
        let f2 = save_folder(&s, None, "Projekte", None).unwrap();
        set_folders(&s, &m.id, &[f1.id.clone(), f2.id.clone()]).unwrap();
        assert_eq!(s.meeting_folder_ids(&m.id).unwrap().len(), 2);

        let renamed = save_folder(&s, Some(f1.id.clone()), "Kunden", None).unwrap();
        assert_eq!(renamed.name, "Kunden");
        assert_eq!(renamed.meeting_count, 1);

        s.folder_delete(&f1.id).unwrap();
        assert_eq!(s.meeting_folder_ids(&m.id).unwrap(), vec![f2.id.clone()]);
        assert_eq!(search(&s, "", filter(), 0, 25).unwrap().total, 1);
    }

    /// P4b: der Status zaehlt fertige Besprechungen, lexikalisch indexierte
    /// (inkl. eingebetteter) und eingebettete getrennt.
    #[test]
    fn index_status_counts_meetings_chunks_and_vectors() {
        let (_dir, s) = tmp_store();
        let a = ready_meeting(&s, "A", 1_000);
        let _b = ready_meeting(&s, "B", 2_000);
        let ids = s
            .replace_meeting_chunks(
                &a.id,
                &[ChunkSource::Transcript],
                &[draft(ChunkSource::Transcript, "Budget")],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        let st = index_status_from(&s, "m").unwrap();
        assert_eq!((st.total, st.pending, st.lexical_done, st.embedded), (2, 1, 1, 0));
        assert_eq!((st.chunks, st.vectors), (1, 0));
        s.put_vectors("m", &[(ids[0], vec![1.0, 0.0])]).unwrap();
        s.set_index_status(&a.id, "ready", None, Some("m")).unwrap();
        let st = index_status_from(&s, "m").unwrap();
        assert_eq!((st.lexical_done, st.embedded, st.vectors), (1, 1, 1));
        assert!(!st.model_ready && !st.running && st.last_error.is_none());
    }

    #[test]
    fn unknown_folder_is_reported_and_nothing_changes() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "A", 1_000);
        let f = save_folder(&s, None, "Vertrieb", None).unwrap();
        set_folders(&s, &m.id, &[f.id.clone()]).unwrap();
        let err = set_folders(&s, &m.id, &["gibt-es-nicht".to_string()]).unwrap_err();
        assert_eq!(err, "folder_not_found");
        assert_eq!(s.meeting_folder_ids(&m.id).unwrap(), vec![f.id]);
        assert_eq!(
            save_folder(&s, None, "vertrieb", None).unwrap_err(),
            "folder_name_taken"
        );
    }
}
