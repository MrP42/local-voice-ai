//! Tauri-Commands fuer Notizblock, Vorlagen, Aufgaben und Aufnahmeposition
//! (M1, P1c). Duenne Huelle ueber `MeetingStore` und `MeetingRecorderManager`
//! (Muster `commands/meetings.rs`); die Fehlercodes des Stores
//! (`revision_conflict`, `template_readonly`, `template_not_found`,
//! `template_invalid:<grund>`, `template_import:<grund>`) gehen unveraendert
//! an die UI, die sie uebersetzt. KI-Notizen (Enhance) liegen in P1b.
//!
//! Datenschutz: Notiz-, Vorlagen- und Dateitexte stehen in keiner Fehlermeldung
//! und in keinem Log.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::managers::meetings::notes::model::{
    ActionItem, MeetingNotes, NoteBlock, TemplateInfo, TemplateSpec,
};
use crate::managers::meetings::notes::templates::{self, MAX_FILE_BYTES, MAX_TITLE_CHARS};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::MeetingStore;

/// Laufende Aufnahme und ihre Audioposition (Mikrofon-Zeitachse, ms).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecordingPosition {
    pub meeting_id: String,
    pub position_ms: u64,
}

// ---------------------------------------------------------------------------
// Logik ohne Tauri (testbar mit einem Store im Tempdir)
// ---------------------------------------------------------------------------

const COPY_SUFFIX: &str = " (Kopie)";

/// Titel einer Kopie: `<Titel> (Kopie)`, der Titel wird so gekuerzt, dass das
/// Ergebnis in die 60 Zeichen der Titelpruefung passt.
fn duplicate_title(title: &str) -> String {
    let keep = MAX_TITLE_CHARS - COPY_SUFFIX.chars().count();
    let base: String = title.trim().chars().take(keep).collect();
    format!("{}{COPY_SUFFIX}", base.trim_end())
}

fn duplicate_template(store: &MeetingStore, id: &str) -> Result<TemplateInfo, String> {
    let source = store
        .get_template_info(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "template_not_found".to_string())?;
    store
        .save_template(None, &duplicate_title(&source.title), &source.spec)
        .map_err(|e| e.to_string())
}

fn export_template(store: &MeetingStore, id: &str, path: &Path) -> Result<(), String> {
    let info = store
        .get_template_info(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "template_not_found".to_string())?;
    std::fs::write(path, templates::export_file(&info).as_bytes())
        .map_err(|_| "template_export:write_failed".to_string())
}

/// Liest hoechstens `MAX_FILE_BYTES + 1` Bytes: eine riesige Datei wird nie
/// ganz in den Speicher geholt, nur um dann abgelehnt zu werden.
fn read_import_file(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "template_import:unreadable".to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "template_import:unreadable".to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("template_import:too_large".to_string());
    }
    String::from_utf8(bytes).map_err(|_| "template_import:invalid_json".to_string())
}

fn import_template(store: &MeetingStore, path: &Path) -> Result<TemplateInfo, String> {
    let (title, spec) = templates::import_file(&read_import_file(path)?)?;
    store
        .save_template(None, &title, &spec)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_get(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<MeetingNotes, String> {
    store.get_notes(&meeting_id).map_err(|e| e.to_string())
}

/// Speichert den gesamten Notizblock; Rueckgabe = neue Revision. Bei
/// abweichender `base_revision` Fehler `revision_conflict` ohne Schreiben.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notes_save(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    blocks: Vec<NoteBlock>,
    base_revision: u64,
) -> Result<u64, String> {
    let revision = store
        .save_notes(&meeting_id, &blocks, base_revision)
        .map_err(|e| e.to_string())?;
    crate::managers::meetings::search::indexer::submit_debounced(&app, &meeting_id); // M4-P4b
    Ok(revision)
}

/// Audioposition der laufenden Aufnahme; `None`, wenn keine laeuft.
#[tauri::command]
#[specta::specta]
pub async fn meetings_recording_position(
    recorder: State<'_, Arc<MeetingRecorderManager>>,
) -> Result<Option<RecordingPosition>, String> {
    Ok(recorder
        .position_ms()
        .map(|(meeting_id, position_ms)| RecordingPosition {
            meeting_id,
            position_ms,
        }))
}

#[tauri::command]
#[specta::specta]
pub async fn meetings_set_template(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    template_id: Option<String>,
) -> Result<(), String> {
    store
        .set_meeting_template(&meeting_id, template_id.as_deref())
        .map_err(|e| e.to_string())
}

/// Vorlage, die fuer die Besprechung gewaehlt wurde (`None` = Standard).
#[tauri::command]
#[specta::specta]
pub async fn meetings_get_template(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    store
        .meeting_template_id(&meeting_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_list(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<TemplateInfo>, String> {
    store.list_template_infos().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_save(
    store: State<'_, Arc<MeetingStore>>,
    id: Option<String>,
    title: String,
    spec: TemplateSpec,
) -> Result<TemplateInfo, String> {
    store
        .save_template(id.as_deref(), &title, &spec)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_duplicate(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<TemplateInfo, String> {
    duplicate_template(&store, &id)
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_delete(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    store.delete_template(&id).map_err(|e| e.to_string())
}

/// Schreibt die Vorlage als `.lvtemplate.json`. Der Pfad stammt aus dem
/// Speichern-Dialog; geschrieben wird im Backend (das fs-Plugin laesst nur
/// `$APPDATA` zu, siehe `export.rs`).
#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_export(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    path: String,
) -> Result<(), String> {
    export_template(&store, &id, Path::new(&path))
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_templates_import(
    store: State<'_, Arc<MeetingStore>>,
    path: String,
) -> Result<TemplateInfo, String> {
    import_template(&store, Path::new(&path))
}

#[tauri::command]
#[specta::specta]
pub async fn action_items_list(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<ActionItem>, String> {
    store
        .list_action_items(&meeting_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn action_items_set_status(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    done: bool,
) -> Result<(), String> {
    store
        .set_action_item_status(&id, done)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::notes::templates::{builtin_id, FILE_EXTENSION};

    fn store_in(dir: &Path) -> MeetingStore {
        MeetingStore::open_at(&dir.join("meetings.db")).unwrap()
    }

    #[test]
    fn duplicate_title_appends_the_suffix_and_stays_within_the_limit() {
        assert_eq!(duplicate_title("Allgemein"), "Allgemein (Kopie)");
        let long = "ä".repeat(60);
        let dup = duplicate_title(&long);
        assert!(dup.chars().count() <= MAX_TITLE_CHARS);
        assert!(dup.ends_with(" (Kopie)"));
        assert!(templates::validate_title(&dup).is_ok());
    }

    #[test]
    fn duplicating_a_builtin_yields_an_editable_user_template() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let copy = duplicate_template(&store, &builtin_id("allgemein")).unwrap();
        assert!(!copy.builtin);
        assert!(copy.title.ends_with("(Kopie)"));
        assert!(!copy.spec.sections.is_empty());
        // Die Kopie ist bearbeitbar, das Original nicht.
        assert!(store
            .save_template(Some(&copy.id), "Meine Vorlage", &copy.spec)
            .is_ok());
        let err = store
            .save_template(Some(&builtin_id("allgemein")), "x", &copy.spec)
            .unwrap_err();
        assert_eq!(err.to_string(), "template_readonly");
    }

    #[test]
    fn duplicating_an_unknown_template_reports_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        assert_eq!(
            duplicate_template(&store, "gibt-es-nicht").unwrap_err(),
            "template_not_found"
        );
    }

    #[test]
    fn export_then_import_round_trips_a_template() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let file = dir.path().join(format!("vorlage{FILE_EXTENSION}"));
        export_template(&store, &builtin_id("vertrieb"), &file).unwrap();
        let imported = import_template(&store, &file).unwrap();
        let original = store
            .get_template_info(&builtin_id("vertrieb"))
            .unwrap()
            .unwrap();
        assert!(!imported.builtin);
        assert_eq!(imported.title, original.title);
        assert_eq!(imported.spec, original.spec);
    }

    #[test]
    fn import_rejects_bad_files_with_stable_codes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let write = |name: &str, body: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        assert_eq!(
            import_template(&store, &write("a.json", b"das ist kein json")).unwrap_err(),
            "template_import:invalid_json"
        );
        assert_eq!(
            import_template(
                &store,
                &write("b.json", br#"{"format":"anderes@1","title":"x","spec":{}}"#)
            )
            .unwrap_err(),
            // Falsche Kennung mit kaputter spec: die Struktur scheitert zuerst.
            "template_import:invalid_json"
        );
        let big = vec![b' '; MAX_FILE_BYTES + 10];
        assert_eq!(
            import_template(&store, &write("c.json", &big)).unwrap_err(),
            "template_import:too_large"
        );
        assert_eq!(
            import_template(&store, &dir.path().join("fehlt.json")).unwrap_err(),
            "template_import:unreadable"
        );
        // Nichts davon hat eine Vorlage angelegt: nur die acht mitgelieferten.
        assert_eq!(store.list_template_infos().unwrap().len(), 8);
    }

    #[test]
    fn a_wrong_format_marker_is_reported_as_format() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let good = store
            .get_template_info(&builtin_id("allgemein"))
            .unwrap()
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(&templates::export_file(&good)).unwrap();
        value["format"] = serde_json::Value::String("lva-meeting-template@9".into());
        let p = dir.path().join("neu.json");
        std::fs::write(&p, serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(
            import_template(&store, &p).unwrap_err(),
            "template_import:format"
        );
    }

    #[test]
    fn recording_position_type_exports_with_a_numeric_position() {
        let mut types = specta::TypeCollection::default();
        types.register::<RecordingPosition>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .unwrap();
        assert!(ts.contains("position_ms: number"), "{ts}");
    }
}
