//! Kommandos der Transkript-Fassungen (A3): auflisten, waehlen, Segmente fuer den
//! Vergleich, KI-Zusammenfuehren. Die Logik steht in `meetings::variants` und
//! `meetings::merge`; hier nur Argumente, Tor fuer laufende Auftraege und Fehler.
//!
//! Fehler gehen als Code an die Oberflaeche (`variant_*`, `merge_*`, `no_provider`).

use std::sync::Arc;

use tauri::{AppHandle, State};

use super::big_stack;
use crate::managers::meetings::job::{self, Gate, JobPhase};
use crate::managers::meetings::merge;
use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{MeetingStore, StoredSegment};
use crate::managers::meetings::translate::{self, Control, TranslationReport};
use crate::managers::meetings::variants::{self, TranscriptVariant};
use crate::settings::AppSettings;

fn variant_error(e: variants::VariantError) -> String {
    e.to_string()
}

/// Die Fassungen einer Besprechung. Eine unterbrochene Neu-Transkription (Absturz)
/// wird vorher zurueckgenommen, sofern kein Auftrag mehr laeuft.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variants(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<TranscriptVariant>, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    if !job::global().is_running(&meeting_id) {
        // Nur wenn der Status nicht mehr `processing` ist: dann gehoert die Arbeitskopie
        // in `transcripts` keinem Lauf mehr.
        let _ = variants::recover_interrupted(&mut conn, &meeting_id);
    }
    variants::list(&mut conn, &meeting_id).map_err(variant_error)
}

/// „Fassung waehlen“: macht die Fassung zum aktiven Transkript.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_activate(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<TranscriptVariant, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let (variant, _) = variants::get_segments(&conn, &variant_id).map_err(variant_error)?;
    if job::global().is_running(&variant.meeting_id) {
        return Err(variants::VariantError::Busy.to_string());
    }
    let chosen = variants::activate(&mut conn, &variant_id).map_err(variant_error)?;
    indexer::submit(&app, IndexJob::Meeting(chosen.meeting_id.clone()));
    Ok(chosen)
}

/// Die Segmente einer Fassung (fuer die Vergleichsansicht). Die aktive Fassung liefert
/// den Stand von `transcripts`, also auch Korrekturen von Hand.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_segments(
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<Vec<StoredSegment>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    variants::get_segments(&conn, &variant_id)
        .map(|(_, segments)| segments)
        .map_err(variant_error)
}

/// „Zusammenfuehren“: das lokale Modell verbessert den Text von Fassung `base_id`
/// mit Hilfe von `other_id` und legt eine dritte Fassung an (nicht aktiv). Ausgaben,
/// die das Schema verletzen oder zu viel erfinden, werden verworfen (`merge`).
#[tauri::command]
#[specta::specta]
pub async fn transcript_variants_merge(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    base_id: String,
    other_id: String,
) -> Result<TranscriptVariant, String> {
    if job::global().is_running(&meeting_id) {
        return Err(variants::VariantError::Busy.to_string());
    }
    let store: Arc<MeetingStore> = Arc::clone(&store);
    let settings = crate::settings::get_settings(&app);
    merge::merge_variants(&settings, &store, &meeting_id, &base_id, &other_id).await
}

/// G5: „Übersetzen nach …“: das lokale Modell übersetzt die Fassung `source_variant_id`
/// Satz für Satz nach `target_language` und legt eine NEUE Fassung `translation` an (nicht
/// aktiv). Die Quelle bleibt unverändert und jederzeit wählbar. Läuft als Auftrag der
/// Besprechung (Phase Übersetzung): Fortschritt in Blöcken, Pause, Stopp; bei Stopp, Fehler
/// oder Absturz entsteht keine Fassung. Fehler: `translate_*`, `variant_*`, `no_provider`,
/// `no_model`, `job_busy`.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_translate(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    source_variant_id: String,
    target_language: String,
) -> Result<TranscriptVariant, String> {
    // Hotfix 0.21.1: nur Argumente einsammeln; der Auftrag laeuft auf einem eigenen Thread
    // (siehe `big_stack`). Das Future dieses Commands bleibt klein.
    translate_job(
        job::app_emit(&app),
        Arc::new(crate::settings::get_settings(&app)),
        Arc::clone(&store),
        meeting_id,
        source_variant_id,
        target_language,
    )
    .await
}

/// Der Uebersetzungsauftrag ohne Tauri-Typen: baut und treibt [`run_translate_job`] auf einem
/// Thread mit grossem Stack. Das zurueckgegebene Future ist winzig (Test), nur das darf durch
/// den Haupt-Thread des Webview gereicht werden.
pub(crate) fn translate_job(
    emit: job::EmitFn,
    settings: Arc<AppSettings>,
    store: Arc<MeetingStore>,
    meeting_id: String,
    source_variant_id: String,
    target_language: String,
) -> impl std::future::Future<Output = Result<TranscriptVariant, String>> + Send {
    big_stack::run_result("translate-job", move || {
        run_translate_job(
            emit,
            settings,
            store,
            meeting_id,
            source_variant_id,
            target_language,
        )
    })
}

/// Der Auftrag selbst (vor 0.21.1 der Rumpf des Commands). Sein Future ist gross: nie direkt
/// als Future eines Commands verwenden, immer ueber [`translate_job`].
async fn run_translate_job(
    emit: job::EmitFn,
    settings: Arc<AppSettings>,
    store: Arc<MeetingStore>,
    meeting_id: String,
    source_variant_id: String,
    target_language: String,
) -> Result<TranscriptVariant, String> {
    let guard = job::global()
        .try_start(&meeting_id, emit)
        .map_err(|e| e.to_string())?;
    let handle = Arc::clone(guard.handle());
    handle.begin_phase_ex(JobPhase::Translation, 0, false);
    let progress_job = Arc::clone(&handle);
    let run = job::scope(
        Arc::clone(&handle),
        translate::translate_variant(
            &settings,
            &store,
            &meeting_id,
            &source_variant_id,
            &target_language,
            || async {
                match job::checkpoint().await {
                    Gate::Go { .. } => Control::Go,
                    Gate::Stopped | Gate::Cancelled => Control::Stop,
                }
            },
            move |done, total| {
                if done == 0 {
                    // Mehr als ein Block: dazwischen laesst sich anhalten.
                    progress_job.begin_phase_ex(JobPhase::Translation, total as u64, total >= 2);
                } else {
                    progress_job.advance(done as u64);
                }
            },
        ),
    );
    // Ein Stopp beendet die Anfrage an das Modell sofort; nichts wird gespeichert.
    let result = tokio::select! {
        result = run => result,
        _ = handle.stopped() => Err("translate_cancelled".to_string()),
    };
    drop(guard);
    if let Err(code) = &result {
        log::warn!(
            "Übersetzung beendet ohne Fassung: {}",
            code.split(':').next().unwrap_or(code)
        );
    }
    result
}

/// G5: der Prüfbericht einer Übersetzung (markierte Sätze mit Grund, Quelle, Sprachen).
/// `None` bei jeder anderen Fassung.
#[tauri::command]
#[specta::specta]
pub async fn transcript_variant_report(
    store: State<'_, Arc<MeetingStore>>,
    variant_id: String,
) -> Result<Option<TranslationReport>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    let meta = variants::get_meta(&conn, &variant_id).map_err(variant_error)?;
    Ok(meta.and_then(|json| serde_json::from_str(&json).ok()))
}

#[cfg(test)]
mod tests {
    //! Hotfix 0.21.1: der Uebersetzungsauftrag laeuft auf einem eigenen Thread mit grossem Stack.
    //! Vor dem Hotfix war sein Future (und die Huelle des Commands) auf dem 1-MiB-Haupt-Thread
    //! zu gross: `STATUS_STACK_OVERFLOW` beim Klick auf "Uebersetzen". Den Ueberlauf selbst (der
    //! den ganzen Prozess beendet) belegt `big_stack::tests` im Kindprozess mit einem Auftrag,
    //! dessen Future groesser ist als der Stack; hier steht, dass der echte Auftrag denselben
    //! Weg nimmt: der Command reicht nur ein winziges Future weiter.

    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::managers::integrations::test_support::Fx;
    use crate::managers::meetings::llm_call::test_support::{
        settings_with_mock_provider, spawn_llm_mock_with, MockReply,
    };
    use crate::managers::meetings::recorder::MeetingEvent;
    use crate::managers::meetings::store::{
        MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
    };
    use crate::managers::usage::{self, UsageLedger};
    use crate::settings::get_default_settings;

    const GOOD: &str = r#"{"lines":[{"n":1,"text":"Guten Morgen, hier ist Anna von Siemens."},{"n":2,"text":"Wir zahlen jeden Monat 1.200,50 Euro."}]}"#;

    fn ensure_ledger() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            if usage::ledger().is_none() {
                let ledger = UsageLedger::open(Path::new(":memory:")).unwrap();
                usage::install_globals(Arc::new(ledger), Arc::new(get_default_settings));
            }
        });
    }

    fn reply() -> String {
        json!({
            "choices": [{ "message": { "role": "assistant", "content": GOOD } }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        })
        .to_string()
    }

    fn seg(i: u32, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: i,
            text: text.to_string(),
            start_ms: u64::from(i) * 4000 + 1000,
            end_ms: u64::from(i) * 4000 + 4500,
            channel: 2,
            speaker_index: Some(i % 2),
            words: None,
        }
    }

    /// Besprechung mit englischem Original (Fassung 1, aktiv, Sprache `en`).
    fn english_meeting() -> (Arc<MeetingStore>, String, String) {
        let fx = Fx::new();
        let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
        let meeting = store
            .create_meeting("Interview", MeetingSource::Import, None)
            .unwrap()
            .id;
        store.set_status(&meeting, MeetingStatus::Ready).unwrap();
        store
            .append_delta(
                &meeting,
                &TranscriptDelta {
                    new_segments: vec![
                        seg(0, "Good morning, this is Anna from Siemens."),
                        seg(1, "We pay 1,200.50 euros every month."),
                    ],
                },
            )
            .unwrap();
        let mut conn = store.get_connection().unwrap();
        variants::set_language(&mut conn, &meeting, "en").unwrap();
        let original = variants::list(&mut conn, &meeting).unwrap()[0].id.clone();
        drop(conn);
        (store, meeting, original)
    }

    fn mock_settings() -> Arc<AppSettings> {
        let port =
            tauri::async_runtime::block_on(spawn_llm_mock_with(|_| MockReply::Body(reply())));
        Arc::new(settings_with_mock_provider(port))
    }

    fn collector() -> (job::EmitFn, Arc<Mutex<Vec<MeetingEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: job::EmitFn = Arc::new(move |event| sink.lock().unwrap().push(event));
        (emit, events)
    }

    #[test]
    fn the_command_hands_tauri_a_tiny_future_and_the_job_still_translates() {
        ensure_ledger();
        let (store, meeting, original) = english_meeting();
        let (emit, events) = collector();
        let future = translate_job(
            emit,
            mock_settings(),
            Arc::clone(&store),
            meeting.clone(),
            original.clone(),
            "de".into(),
        );
        // Das, was durch den Haupt-Thread des Webview gereicht wird: ein Empfangskanal. Ohne den
        // Hotfix war es der ganze Auftrag (im Release 335 KiB, mit den Kopien der Huelle ueber
        // 1 MiB Stack): `inline` zeigt, was dort stand.
        let inline = {
            let (emit, _) = collector();
            let probe = run_translate_job(
                emit,
                Arc::new(get_default_settings()),
                Arc::clone(&store),
                meeting.clone(),
                original.clone(),
                "de".into(),
            );
            std::mem::size_of_val(&probe)
        };
        assert!(
            std::mem::size_of_val(&future) <= 256 && std::mem::size_of_val(&future) * 16 < inline,
            "Future des Commands: {} Byte (der Auftrag selbst: {inline} Byte)",
            std::mem::size_of_val(&future)
        );
        let translated = tauri::async_runtime::block_on(future).unwrap();
        assert_eq!(translated.kind, variants::KIND_TRANSLATION);
        assert_eq!(translated.language.as_deref(), Some("de"));
        assert!(
            !translated.active,
            "das Original bleibt das aktive Transkript"
        );
        // Der Auftrag meldete Fortschritt in der Phase Uebersetzung und gab seinen Platz frei.
        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                MeetingEvent::Progress {
                    phase: JobPhase::Translation,
                    ..
                }
            )),
            "kein Fortschritt der Phase Uebersetzung"
        );
        assert!(!job::global().is_running(&meeting));
        let mut conn = store.get_connection().unwrap();
        assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 2);
    }

    #[test]
    fn a_failed_job_keeps_its_own_code_frees_the_slot_and_leaves_no_variant() {
        ensure_ledger();
        let (store, meeting, original) = english_meeting();
        // Ohne eingerichtetes Modell: der Code kommt aus dem Auftrag, nicht als Thread-Fehler.
        let (emit, _) = collector();
        let err = tauri::async_runtime::block_on(translate_job(
            emit,
            Arc::new(get_default_settings()),
            Arc::clone(&store),
            meeting.clone(),
            original.clone(),
            "de".into(),
        ))
        .unwrap_err();
        assert!(err == "no_provider" || err == "no_model", "{err}");
        assert!(
            !job::global().is_running(&meeting),
            "auch nach einem Fehler ist der Platz frei"
        );
        let mut conn = store.get_connection().unwrap();
        assert_eq!(variants::list(&mut conn, &meeting).unwrap().len(), 1);
    }
}
