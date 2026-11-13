//! Duenne Befehlshuelle ueber `managers::planning` (Besprechungsplanung, lva#534, P3; Muster
//! `commands/calendar.rs`). Die Logik und die Transaktionen stehen in `managers/planning`; Fehler
//! sind die String-Codes des Crates `besprechungsplanung` (`plan_nicht_gefunden`,
//! `status_uebergang_ungueltig`, `einwilligung_nachweis_fehlt`, ...) plus die der App
//! (`verknuepfung_fehlt`, `marke_fehlt`, `termin_belegt`, `status_geaendert`, ...).

use std::sync::Arc;

use besprechungsplanung::consent::StartModus;
use besprechungsplanung::invitation::{Einladung, Gestaltung};
use besprechungsplanung::model::{Consent, Plan, PlanInput, PlanLink, PlanStatus};
use besprechungsplanung::sync::Ereignis;
use tauri::State;

use crate::managers::meetings::store::MeetingStore;
use crate::managers::planning::adapter_c::{self, AbgleichBericht};
use crate::managers::planning::{self, EinwilligungsEintrag};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Die Datenbankarbeit gehoert nicht auf den Async-Worker: `BEGIN IMMEDIATE` kann auf eine
/// Schreibsperre warten (laufender Kalenderabruf, Aufnahme), deshalb `spawn_blocking`.
async fn blockierend<T, F>(store: State<'_, Arc<MeetingStore>>, arbeit: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&MeetingStore) -> Result<T, String> + Send + 'static,
{
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || arbeit(&store))
        .await
        .map_err(|e| format!("Planung abgebrochen: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_anlegen(
    store: State<'_, Arc<MeetingStore>>,
    input: PlanInput,
) -> Result<Plan, String> {
    blockierend(store, move |store| {
        planning::plan_anlegen(store, &input, now_ms())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_aendern(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    input: PlanInput,
) -> Result<Plan, String> {
    blockierend(store, move |store| {
        planning::plan_aendern(store, &id, &input, now_ms())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_laden(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<Option<Plan>, String> {
    blockierend(store, move |store| planning::plan_laden(store, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_liste(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<Plan>, String> {
    blockierend(store, planning::plan_liste).await
}

/// Weiches Loeschen (Papierkorb).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_loeschen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    blockierend(store, move |store| {
        planning::plan_loeschen(store, &id, now_ms())
    })
    .await
}

/// Status von Hand setzen. `verknuepft`, `verschoben` und `abgesagt` setzt nur der Abgleich mit
/// dem Kalender (`status_nur_durch_abgleich`).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_status_setzen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    status: PlanStatus,
) -> Result<(), String> {
    blockierend(store, move |store| {
        planning::status_setzen(store, &id, status, now_ms())
    })
    .await
}

/// Einladungstext (HTML und Text, Marke `lva-plan:<id>`) der gespeicherten Planung und Vermerk der
/// Uebergabe an Outlook: die Planung wird `uebergeben`, der Abgleich sucht danach die Marke im
/// Kalender. Ohne `gestaltung` gilt die neutrale.
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_einladung(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    gestaltung: Option<Gestaltung>,
) -> Result<Einladung, String> {
    blockierend(store, move |store| {
        planning::einladung_erzeugen(store, &id, &gestaltung.unwrap_or_default(), now_ms())
    })
    .await
}

/// Die Verknuepfung der Planung mit ihrem Termin (`None`, solange nichts uebergeben wurde).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_verknuepfung(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<Option<PlanLink>, String> {
    blockierend(store, move |store| planning::verknuepfung_laden(store, &id)).await
}

/// Einwilligung einer Person setzen (Zustimmung und Ablehnung nur mit Zeitpunkt und Kanal).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_einwilligung_setzen(
    store: State<'_, Arc<MeetingStore>>,
    einwilligung: Consent,
) -> Result<(), String> {
    blockierend(store, move |store| {
        planning::einwilligung_setzen(store, &einwilligung, now_ms())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_einwilligungen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<Vec<Consent>, String> {
    blockierend(store, move |store| planning::einwilligungen(store, &id)).await
}

/// Verlauf der Einwilligungen (aelteste zuerst), als Nachweis.
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_einwilligungs_verlauf(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<Vec<EinwilligungsEintrag>, String> {
    blockierend(store, move |store| {
        planning::einwilligungs_verlauf(store, &id)
    })
    .await
}

/// Wie die Besprechung startet: mit Transkription nur, wenn sie geplant ist und alle Anwesenden
/// nachweislich zugestimmt haben; sonst nur Notizen (mit den offenen und abgelehnten Personen).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_start_modus(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    anwesende: Vec<String>,
) -> Result<StartModus, String> {
    blockierend(store, move |store| {
        planning::start_modus(store, &id, &anwesende)
    })
    .await
}

/// Die Verschiebung zur Kenntnis genommen: der Merker faellt, `verschoben` wird `verknuepft`.
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_verschiebung_bestaetigen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<PlanLink, String> {
    blockierend(store, move |store| {
        planning::verschiebung_bestaetigen(store, &id, now_ms())
    })
    .await
}

/// Loest die Bindung an den bisherigen Termin (nur auf ausdrueckliche Auswahl der Nutzerin bzw. des
/// Nutzers): der naechste Abgleich verknuepft ueber die Marke neu.
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_identitaet_loesen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<PlanLink, String> {
    blockierend(store, move |store| {
        planning::identitaet_loesen(store, &id, now_ms())
    })
    .await
}

/// Bestaetigte Auswahl eines Termins aus dem Kalender-Cache (`event_key`): fuer mehrdeutige
/// Marken (Serien) und den Wechsel des Termins. Der Termin muss die Marke der Planung tragen.
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_termin_waehlen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    event_key: String,
) -> Result<Vec<Ereignis>, String> {
    blockierend(store, move |store| {
        adapter_c::termin_waehlen(store, &id, &event_key, now_ms())
    })
    .await
}

/// Gleicht alle offenen Planungen jetzt mit dem Kalender-Cache ab (sonst nach jedem Abruf).
#[tauri::command]
#[specta::specta]
pub async fn besprechungsplanung_abgleichen(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<AbgleichBericht, String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || adapter_c::abgleich_lauf(&store, now_ms()))
        .await
        .map_err(|e| format!("Abgleich abgebrochen: {e}"))?
}
