//! Herkunft erzeugter Inhalte (A1, Goal „Integrationen“): Kommando fuer das
//! Kontextmenue „Herkunft“ (Paket A3).

use std::sync::Arc;

use tauri::State;

use crate::managers::meetings::store::MeetingStore;
use crate::managers::provenance::{self, ProvenanceEntry, SubjectKind};

/// Herkunft eines Inhalts: Modell, Anbieter (lokal/entfernt), Token, Dauer,
/// Zeitpunkt, Quellen, Konfidenz und Ausloeser. `content_type` ist die Art
/// (`transcript`, `document`, ...), `id` die Kennung des Inhalts: bei
/// `transcript` die Besprechung, bei `document` die Dokument-ID.
///
/// Gibt es keinen gespeicherten Eintrag (Inhalt aus der Zeit vor der
/// Provenienz), wird die Herkunft aus den alten Daten abgeleitet
/// (`origin: "derived"`); ist auch das nicht moeglich, ist die Liste leer.
#[tauri::command]
#[specta::specta]
pub async fn provenance_get(
    store: State<'_, Arc<MeetingStore>>,
    content_type: SubjectKind,
    id: String,
) -> Result<Vec<ProvenanceEntry>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    provenance::get(&conn, content_type, &id).map_err(|e| e.to_string())
}
