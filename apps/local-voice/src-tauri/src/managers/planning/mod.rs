//! Besprechungsplanung (Epic lva#534, Konzept `docs/plans/2026-11-12_besprechungsplanung.md`),
//! Anbindung des Crates `besprechungsplanung` an die Besprechungs-Datenbank (P3).
//!
//! Das Crate liefert Modell, Regeln, Einladungstext, Abgleich und die SQL-Zugriffe auf einer
//! geliehenen Verbindung. Hier steht, was die App dazu beitraegt:
//!
//! - Verbindung und Transaktion: jede Planungs-Verbindung hat `PRAGMA foreign_keys = ON`
//!   (geprueft, sonst `foreign_keys_nicht_aktiv`); jede Aenderung laeuft in EINER Transaktion
//!   `BEGIN IMMEDIATE`. Die Schreibsperre liegt damit VOR dem Lesen des Stands: zwischen Pruefen
//!   (Status, Verknuepfung) und Schreiben kann keine andere Verbindung etwas aendern.
//! - die Befehle der Oberflaeche als Funktionen auf `&MeetingStore` (`commands::besprechungsplanung`
//!   ist nur die duenne Huelle);
//! - [`adapter_c`]: Adapter C, der Abgleich der Planungen mit dem Kalender-Cache nach jedem Abruf.
//!
//! Fehler sind die String-Codes des Crates (`plan_nicht_gefunden`, `plan_abgeschlossen`,
//! `status_uebergang_ungueltig`, ...) plus die der App (siehe Funktionen).

pub mod adapter_c;

#[cfg(test)]
mod tests;

use besprechungsplanung::consent::{self, StartModus};
use besprechungsplanung::invitation::{self, Einladung, Gestaltung};
use besprechungsplanung::model::{
    AdapterKind, Consent, Plan, PlanInput, PlanLink, PlanStatus, SyncState,
};
use besprechungsplanung::rules::Regeln;
use besprechungsplanung::{store as plan_store, sync};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::managers::meetings::store::MeetingStore;

pub(crate) type Res<T> = Result<T, String>;

/// Eine Zeile im Verlauf der Einwilligungen: der damalige Stand und wann er festgehalten wurde.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EinwilligungsEintrag {
    pub einwilligung: Consent,
    /// Zeitpunkt des Eintrags in ms (Nachweis; `recorded_at` der Einwilligung ist der der Zusage).
    pub festgehalten_am: i64,
}

/// Oeffnet eine Verbindung fuer die Planungstabellen: Fremdschluessel an und geprueft.
///
/// `MeetingStore::get_connection` setzt `foreign_keys` schon selbst; die Pruefung hier faengt eine
/// SQLite-Fassung ab, die den Schalter ignoriert (dann wuerden Einwilligungen beim endgueltigen
/// Loeschen einer Planung liegen bleiben).
pub(crate) fn verbindung(store: &MeetingStore) -> Res<Connection> {
    let conn = store.get_connection().map_err(|e| format!("sql: {e}"))?;
    fremdschluessel_pruefen(&conn)?;
    Ok(conn)
}

fn fremdschluessel_pruefen(conn: &Connection) -> Res<()> {
    let an: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .map_err(|e| format!("sql: {e}"))?;
    if an == 1 {
        Ok(())
    } else {
        Err("foreign_keys_nicht_aktiv".to_string())
    }
}

/// Beginnt eine Schreib-Transaktion mit sofortiger Schreibsperre.
pub(crate) fn schreib_tx(conn: &mut Connection) -> Res<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("sql: {e}"))
}

fn sql(e: rusqlite::Error) -> String {
    format!("sql: {e}")
}

/// Fuehrt `f` in einer Schreib-Transaktion aus; bei einem Fehler wird nichts geschrieben.
fn in_tx<T>(store: &MeetingStore, f: impl FnOnce(&Transaction<'_>) -> Res<T>) -> Res<T> {
    let mut conn = verbindung(store)?;
    let tx = schreib_tx(&mut conn)?;
    let out = f(&tx)?;
    tx.commit().map_err(sql)?;
    Ok(out)
}

/// Setzt den Status nur, wenn er noch der ist, den der Aufrufer gelesen hat
/// (`status_geaendert`, wenn nicht). Eine abgeschlossene Planung (durchgefuehrt, protokolliert,
/// versendet) wird hier nie veraendert: dafuer gilt [`plan_store::plan_status_setzen`].
pub(crate) fn status_wechseln(
    conn: &Connection,
    plan_id: &str,
    erwartet: PlanStatus,
    neu: PlanStatus,
    jetzt: i64,
) -> Res<()> {
    if erwartet.ist_abgeschlossen() || !plan_store::uebergang_erlaubt(erwartet, neu) {
        return Err("status_uebergang_ungueltig".to_string());
    }
    let n = conn
        .execute(
            "UPDATE meeting_plans SET status = ?2, updated_at = ?3
             WHERE id = ?1 AND status = ?4 AND deleted_at IS NULL",
            params![plan_id, neu.as_str(), jetzt, erwartet.as_str()],
        )
        .map_err(sql)?;
    if n == 1 {
        Ok(())
    } else {
        Err("status_geaendert".to_string())
    }
}

fn plan_holen(conn: &Connection, id: &str) -> Res<Plan> {
    plan_store::plan_laden(conn, id)?.ok_or_else(|| "plan_nicht_gefunden".to_string())
}

// ---------------------------------------------------------------------------
// Planung: anlegen, aendern, laden, Liste, loeschen, Status
// ---------------------------------------------------------------------------

/// Legt eine Planung im Status `entwurf` an (Id: ULID, zugleich die Marke im Termin).
pub fn plan_anlegen(store: &MeetingStore, input: &PlanInput, jetzt: i64) -> Res<Plan> {
    let id = ulid::Ulid::new().to_string();
    in_tx(store, |tx| {
        plan_store::plan_anlegen(tx, &id, input, &Regeln::default(), jetzt)
    })
}

/// Aendert die Inhalte einer Planung (nicht mehr nach der Durchfuehrung).
pub fn plan_aendern(store: &MeetingStore, id: &str, input: &PlanInput, jetzt: i64) -> Res<Plan> {
    in_tx(store, |tx| {
        plan_store::plan_aendern(tx, id, input, &Regeln::default(), jetzt)
    })
}

pub fn plan_laden(store: &MeetingStore, id: &str) -> Res<Option<Plan>> {
    plan_store::plan_laden(&verbindung(store)?, id)
}

pub fn plan_liste(store: &MeetingStore) -> Res<Vec<Plan>> {
    plan_store::plan_liste(&verbindung(store)?)
}

/// Weiches Loeschen (Papierkorb); Verknuepfung und Einwilligungen bleiben bis zum endgueltigen
/// Entfernen der Zeile.
pub fn plan_loeschen(store: &MeetingStore, id: &str, jetzt: i64) -> Res<()> {
    in_tx(store, |tx| plan_store::plan_loeschen(tx, id, jetzt))
}

/// Setzt den Status von Hand. `verschoben` und `abgesagt` setzt nur der Abgleich mit dem
/// Kalender (Modell-Doku), `verknuepft` ebenfalls (Folge einer erkannten Verknuepfung): davon
/// abgesehen gelten die Uebergaenge des Crates.
pub fn status_setzen(store: &MeetingStore, id: &str, status: PlanStatus, jetzt: i64) -> Res<()> {
    if matches!(
        status,
        PlanStatus::Verschoben | PlanStatus::Abgesagt | PlanStatus::Verknuepft
    ) {
        return Err("status_nur_durch_abgleich".to_string());
    }
    in_tx(store, |tx| {
        let plan = plan_holen(tx, id)?;
        if plan.status == status {
            return Ok(());
        }
        if !plan_store::uebergang_erlaubt(plan.status, status) {
            return Err("status_uebergang_ungueltig".to_string());
        }
        // Ein abgeschlossener Status geht nur vorwaerts: den Wechsel fuehrt das Crate aus.
        if plan.status.ist_abgeschlossen() {
            return plan_store::plan_status_setzen(tx, id, status, jetzt);
        }
        status_wechseln(tx, id, plan.status, status, jetzt)
    })
}

// ---------------------------------------------------------------------------
// Einladung und Uebergabe (Adapter C)
// ---------------------------------------------------------------------------

/// Baut den Einladungstext der gespeicherten Planung und vermerkt die Uebergabe an Outlook
/// (Adapter C): die Verknuepfung entsteht ohne Termin, mit der Pruefsumme des Blocks, und ein
/// Entwurf wird `uebergeben`. Erst damit sucht der Abgleich nach der Marke im Kalender.
/// Wiederholt aufgerufen aendert es eine schon erkannte Verknuepfung nicht.
pub fn einladung_erzeugen(
    store: &MeetingStore,
    id: &str,
    gestaltung: &Gestaltung,
    jetzt: i64,
) -> Res<Einladung> {
    in_tx(store, |tx| {
        let plan = plan_holen(tx, id)?;
        if plan.status.ist_abgeschlossen() {
            return Err("plan_abgeschlossen".to_string());
        }
        let einladung = invitation::einladung(&plan, gestaltung);
        match plan_store::link_laden(tx, id)? {
            None => plan_store::link_speichern(
                tx,
                &PlanLink {
                    body_hash: Some(einladung.block_hash.clone()),
                    ..leere_verknuepfung(id)
                },
            )?,
            Some(link) if !hat_termin(&link) => plan_store::link_speichern(
                tx,
                &PlanLink {
                    body_hash: Some(einladung.block_hash.clone()),
                    ..link
                },
            )?,
            Some(_) => {}
        }
        if plan.status == PlanStatus::Entwurf {
            status_wechseln(tx, id, PlanStatus::Entwurf, PlanStatus::Uebergeben, jetzt)?;
        }
        Ok(einladung)
    })
}

/// Verknuepfung ohne Termin (Adapter C, noch nichts beobachtet).
pub(crate) fn leere_verknuepfung(plan_id: &str) -> PlanLink {
    PlanLink {
        plan_id: plan_id.to_string(),
        adapter: AdapterKind::IcsMarker,
        source_id: None,
        remote_id: None,
        ical_uid: None,
        change_key: None,
        starts_at: None,
        ends_at: None,
        location: None,
        join_url: None,
        body_hash: None,
        last_seen_at: None,
        last_change_at: None,
        sync_state: SyncState::Ok,
        verschiebung_offen: false,
        block_konflikt: false,
        abgesagt: false,
        instanz: None,
        fremd_beobachtet: false,
    }
}

/// Ist die Verknuepfung schon an einen Termin gebunden? (Erstverknuepfung: nein.)
pub(crate) fn hat_termin(link: &PlanLink) -> bool {
    link.remote_id.is_some()
        || link.ical_uid.is_some()
        || link.starts_at.is_some()
        || link.instanz.is_some()
}

pub fn verknuepfung_laden(store: &MeetingStore, id: &str) -> Res<Option<PlanLink>> {
    plan_store::link_laden(&verbindung(store)?, id)
}

// ---------------------------------------------------------------------------
// Verschiebung bestaetigen, Identitaet loesen
// ---------------------------------------------------------------------------

/// Die Nutzerin bzw. der Nutzer hat die Verschiebung zur Kenntnis genommen: der Merker faellt, der
/// Status `verschoben` wird wieder `verknuepft` (sofern der Termin nicht abgesagt ist).
pub fn verschiebung_bestaetigen(store: &MeetingStore, id: &str, jetzt: i64) -> Res<PlanLink> {
    in_tx(store, |tx| {
        let plan = plan_holen(tx, id)?;
        let link = plan_store::link_laden(tx, id)?.ok_or("verknuepfung_fehlt")?;
        let neu = sync::verschiebung_bestaetigen(&link);
        if neu != link {
            plan_store::link_speichern(tx, &neu)?;
        }
        if plan.status == PlanStatus::Verschoben {
            status_wechseln(
                tx,
                id,
                PlanStatus::Verschoben,
                PlanStatus::Verknuepft,
                jetzt,
            )?;
        }
        Ok(neu)
    })
}

/// Loest die Termin-Identitaet der Verknuepfung (ausdrueckliche Auswahl der Nutzerin bzw. des
/// Nutzers: "anderer Termin"). Der naechste Abgleich gilt dann als Erstverknuepfung ueber die
/// Marke. Die Planung faellt auf `uebergeben` zurueck, solange sie an keinen Termin gebunden ist;
/// Planung und Einwilligungen bleiben sonst unberuehrt.
pub fn identitaet_loesen(store: &MeetingStore, id: &str, jetzt: i64) -> Res<PlanLink> {
    in_tx(store, |tx| {
        let plan = plan_holen(tx, id)?;
        if plan.status.ist_abgeschlossen() {
            return Err("plan_abgeschlossen".to_string());
        }
        let link = plan_store::link_laden(tx, id)?.ok_or("verknuepfung_fehlt")?;
        let neu = lose(&link);
        plan_store::link_speichern(tx, &neu)?;
        zuruecksetzen_auf_uebergeben(tx, &plan, jetzt)?;
        Ok(neu)
    })
}

/// [`sync::identitaet_loesen`], dazu die Kalenderquelle: sie gehoerte zum bisherigen Termin.
pub(crate) fn lose(link: &PlanLink) -> PlanLink {
    PlanLink {
        source_id: None,
        ..sync::identitaet_loesen(link)
    }
}

pub(crate) fn zuruecksetzen_auf_uebergeben(conn: &Connection, plan: &Plan, jetzt: i64) -> Res<()> {
    if matches!(
        plan.status,
        PlanStatus::Verknuepft | PlanStatus::Verschoben | PlanStatus::Abgesagt
    ) {
        status_wechseln(conn, &plan.id, plan.status, PlanStatus::Uebergeben, jetzt)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Einwilligungen und Startmodus
// ---------------------------------------------------------------------------

/// Setzt die Einwilligung einer Person (Zustimmung und Ablehnung nur mit Zeitpunkt und Kanal);
/// Stand und Verlaufszeile entstehen in einer Transaktion.
pub fn einwilligung_setzen(store: &MeetingStore, c: &Consent, jetzt: i64) -> Res<()> {
    in_tx(store, |tx| plan_store::einwilligung_setzen(tx, c, jetzt))
}

pub fn einwilligungen(store: &MeetingStore, plan_id: &str) -> Res<Vec<Consent>> {
    plan_store::einwilligungen(&verbindung(store)?, plan_id)
}

pub fn einwilligungs_verlauf(
    store: &MeetingStore,
    plan_id: &str,
) -> Res<Vec<EinwilligungsEintrag>> {
    Ok(
        plan_store::einwilligungs_verlauf(&verbindung(store)?, plan_id)?
            .into_iter()
            .map(|(einwilligung, festgehalten_am)| EinwilligungsEintrag {
                einwilligung,
                festgehalten_am,
            })
            .collect(),
    )
}

/// Wie die Besprechung startet: mit Transkription nur, wenn sie geplant ist und alle Anwesenden
/// nachweislich zugestimmt haben (`consent::start_modus`). Plan und Einwilligungen werden aus
/// demselben Lesezugriff gelesen.
pub fn start_modus(store: &MeetingStore, plan_id: &str, anwesende: &[String]) -> Res<StartModus> {
    let mut conn = verbindung(store)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(sql)?;
    let plan = plan_holen(&tx, plan_id)?;
    let einwilligungen = plan_store::einwilligungen(&tx, plan_id)?;
    Ok(consent::start_modus(
        plan_id,
        plan.transcription_planned,
        anwesende,
        &einwilligungen,
    ))
}
