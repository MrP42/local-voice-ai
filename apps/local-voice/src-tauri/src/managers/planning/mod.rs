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
    let _sperre = tor_sperre();
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
    let _sperre = tor_sperre();
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
/// Marke. Die Planung faellt auf `uebergeben` zurueck, solange sie an keinen Termin gebunden ist.
/// Die Einwilligungen gelten je Termin: war die Planung gebunden, werden alle auf `offen`
/// zurueckgesetzt (siehe [`einwilligungen_zuruecksetzen`]).
pub fn identitaet_loesen(store: &MeetingStore, id: &str, jetzt: i64) -> Res<PlanLink> {
    let _sperre = tor_sperre();
    in_tx(store, |tx| {
        let plan = plan_holen(tx, id)?;
        if plan.status.ist_abgeschlossen() {
            return Err("plan_abgeschlossen".to_string());
        }
        let link = plan_store::link_laden(tx, id)?.ok_or("verknuepfung_fehlt")?;
        let neu = lose(&link);
        if hat_termin(&link) {
            einwilligungen_zuruecksetzen(tx, id, jetzt)?;
        }
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

/// Setzt alle aktuellen Einwilligungen der Planung auf `offen` (ohne Nachweis), je Person mit einer
/// Verlaufszeile; die bisherigen Zeilen mit dem Nachweis bleiben im Verlauf. Aufrufer halten die
/// Transaktion des Termin-Wechsels. Eine Einwilligung gilt je Termin und Person (Konzept 3.5).
pub(crate) fn einwilligungen_zuruecksetzen(
    conn: &Connection,
    plan_id: &str,
    jetzt: i64,
) -> Res<()> {
    for c in plan_store::einwilligungen(conn, plan_id)? {
        if c.state == besprechungsplanung::model::ConsentState::Offen
            && c.channel.is_none()
            && c.recorded_at.is_none()
        {
            continue;
        }
        let offen = Consent {
            state: besprechungsplanung::model::ConsentState::Offen,
            channel: None,
            recorded_at: None,
            ..c
        };
        plan_store::einwilligung_setzen(conn, &offen, jetzt)?;
    }
    Ok(())
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
    let _sperre = tor_sperre();
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

// ---------------------------------------------------------------------------
// Einwilligungs-Tor am Start der Besprechung
// ---------------------------------------------------------------------------

/// Die Planungen, die zu einem Start gehoeren, und die Anwesenden dazu.
struct StartBezug {
    plan_ids: Vec<String>,
    anwesende: Vec<String>,
}

/// Findet die Planungen der Besprechung, die gleich startet.
///
/// Verknuepfung (der kleinste saubere Weg, ohne neue Tabelle): Besprechung und Planung teilen den
/// KALENDERTERMIN. Der Start kennt den Termin (`event`, bei `meetings_start_from_event` und der
/// Workflow-Aktion) oder die Besprechung, in die aufgenommen wird, ueber `meeting_calendar_links`
/// (`target_meeting_id`); die Planung gehoert dazu, wenn ihre Verknuepfung dieselbe UID und
/// Instanz traegt (Adapter C, `meeting_plan_links`) oder sie die Besprechung ausdruecklich
/// fuehrt (`meeting_plans.meeting_id`). Eine ausdrueckliche `plan_id` beim Start waere dagegen
/// freiwillig: wer sie weglaesst, umginge das Tor.
///
/// Ist der Termin nicht mehr im Cache (nur noch der Schnappschuss der Verknuepfung), ist die
/// Instanz unbekannt: dann zaehlen alle Planungen der UID, und es gibt keine Anwesenden (sperrt).
fn start_bezug(
    conn: &Connection,
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<StartBezug> {
    let sql = |e: anyhow::Error| format!("sql: {e}");
    // (uid, Instanz: `None` = unbekannt, `Some(None)` = Einzeltermin), Anwesende
    let mut termin: Option<(String, Option<Option<i64>>)> = None;
    let mut anwesende: Vec<String> = Vec::new();
    // Der Termin kommt IMMER frisch aus dem Cache: Teilnehmende koennen sich seit dem Laden des
    // Aufrufers geaendert haben (Kalenderabruf). Ein uebergebener Schnappschuss ist nur der
    // Schluessel (Key bzw. UID + Instanz); fehlt der Termin im Cache, gibt es keine Anwesenden
    // (das sperrt, sobald eine Planung dazugehoert).
    let mut ereignis = None;
    if let Some(e) = event {
        ereignis = store.calendar_event(&e.key).map_err(sql)?;
        if ereignis.is_none() {
            termin = Some((e.uid.clone(), Some(e.instance_start)));
        }
    } else if let Some(target) = target_meeting_id {
        if let Some(link) = store.meeting_calendar_link(target).map_err(sql)? {
            if let Some(key) = link.event_key.as_deref() {
                ereignis = store.calendar_event(key).map_err(sql)?;
            }
            if ereignis.is_none() {
                termin = Some((link.uid, None));
            }
        }
    }
    if let Some(e) = &ereignis {
        termin = Some((e.uid.clone(), Some(e.instance_start)));
        anwesende = e
            .attendees
            .iter()
            .map(|a| {
                a.email
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map_or_else(|| consent::UNBEKANNT.to_string(), str::to_string)
            })
            .collect();
    }
    let mut plan_ids: Vec<String> = Vec::new();
    if let Some((uid, instanz)) = &termin {
        let mut st = conn
            .prepare(
                "SELECT l.plan_id FROM meeting_plan_links l
                 JOIN meeting_plans p ON p.id = l.plan_id
                 WHERE p.deleted_at IS NULL
                   AND (l.remote_id = ?1 OR l.ical_uid = ?1)
                   AND (?2 = 1 OR l.instance_start IS ?3)
                 ORDER BY l.plan_id",
            )
            .map_err(|e| format!("sql: {e}"))?;
        let flach: Option<i64> = instanz.and_then(|i| i);
        let rows = st
            .query_map(params![uid, instanz.is_none(), flach], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| format!("sql: {e}"))?;
        for id in rows {
            plan_ids.push(id.map_err(|e| format!("sql: {e}"))?);
        }
    }
    if let Some(target) = target_meeting_id {
        let mut st = conn
            .prepare("SELECT id FROM meeting_plans WHERE meeting_id = ?1 AND deleted_at IS NULL")
            .map_err(|e| format!("sql: {e}"))?;
        let rows = st
            .query_map([target], |r| r.get::<_, String>(0))
            .map_err(|e| format!("sql: {e}"))?;
        for id in rows {
            let id = id.map_err(|e| format!("sql: {e}"))?;
            if !plan_ids.contains(&id) {
                plan_ids.push(id);
            }
        }
    }
    Ok(StartBezug {
        plan_ids,
        anwesende,
    })
}

/// Die prozessweite Sperre des Einwilligungs-Tors. Wer das Tor-Ergebnis beeinflussen kann
/// (Einwilligung setzen, Planung aendern, Termin wechseln bzw. Identitaet loesen, Abgleich mit dem
/// Kalender, Schreiben des Kalender-Caches), nimmt sie VOR dem Schreiben; wer eine Transkription
/// startet, haelt sie vom Tor bis zum wirksamen Beginn ([`mit_tor`]). Ein Widerruf, der waehrend
/// eines Starts gespeichert wird, wartet damit bis nach dem Start und gilt fuer den naechsten.
///
/// Reihenfolge der Sperren (nie umdrehen): Recorder-Start (`start_guard`) -> diese Sperre ->
/// Datenbank-Schreibsperre. Eine Datenbankverbindung wird nicht ueber sie hinaus gehalten. Die
/// Sperre gilt je Prozess; die App ist die einzige Schreiberin dieser Tabellen.
static TOR_SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn tor_sperre() -> std::sync::MutexGuard<'static, ()> {
    // `Mutex<()>` traegt keine Daten: eine Vergiftung darf nie alle spaeteren Starts blockieren.
    TOR_SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Nachweis, dass das Einwilligungs-Tor fuer diesen Start bestanden wurde und die Sperre noch
/// gehalten wird. Nur [`mit_tor`] erzeugt ihn; Startfunktionen verlangen ihn als Parameter und
/// sind damit ohne Tor nicht aufrufbar (Compiler statt Zeichenkettenpruefung).
pub struct TorNachweis {
    _nur_hier: (),
}

/// Fuehrt `start` unter dem Einwilligungs-Tor aus: Sperre nehmen, Tor pruefen
/// ([`start_tor`], alles frisch aus der Datenbank), dann `start` AUSFUEHREN, waehrend die Sperre
/// noch gehalten wird. `start` ist die Funktion, die Aufnahme bzw. Transkription wirksam beginnt;
/// sie darf nicht selbst Planungsfunktionen mit Sperre rufen (die Sperre ist nicht
/// wiedereintrittsfaehig). Bei `NurNotizen` wird `start` nie aufgerufen.
pub fn mit_tor<T>(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
    start: impl FnOnce(&TorNachweis) -> Res<T>,
) -> Res<T> {
    let _sperre = tor_sperre();
    start_tor(store, event, target_meeting_id)?;
    start(&TorNachweis { _nur_hier: () })
}

/// [`mit_tor`] fuer eine vorhandene Besprechung (nachtraegliche Transkription, Import in einen
/// verknuepften Eintrag, Fortsetzen): die Planung ergibt sich aus der Besprechung.
pub fn mit_tor_besprechung<T>(
    store: &MeetingStore,
    meeting_id: &str,
    start: impl FnOnce(&TorNachweis) -> Res<T>,
) -> Res<T> {
    mit_tor(store, None, Some(meeting_id), start)
}

/// Das Einwilligungs-Tor: laeuft UNMITTELBAR vor dem Start einer Aufnahme mit
/// Live-Transkription (und vor jeder nachtraeglichen Transkription) und liest dabei alles frisch
/// aus der Datenbank: Einwilligungen, Termin und Teilnehmende (ein Widerruf zwischen einer Abfrage per [`start_modus`] und dem Start wirkt damit). Allein ohne Sperre
/// ist das nur eine Momentaufnahme; atomar mit dem Start ist es nur ueber [`mit_tor`]. Gehoert die Besprechung
/// zu keiner Planung, passiert nichts. Sonst muss JEDE zugehoerige Planung
/// [`StartModus::MitTranskription`] ergeben; bei `NurNotizen` startet nichts (kein Ton, keine
/// Transkription) und der Fehler ist `nur_notizen: <grund>` (`nicht_geplant`, `abgelehnt` oder
/// `offen`; nie Namen). Personen ohne Identitaet und fehlende Anwesende sperren.
pub fn start_tor(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<()> {
    let mut conn = verbindung(store)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(sql)?;
    let bezug = start_bezug(&tx, store, event, target_meeting_id)?;
    for plan_id in &bezug.plan_ids {
        let plan = plan_holen(&tx, plan_id)?;
        let einwilligungen = plan_store::einwilligungen(&tx, plan_id)?;
        match consent::start_modus(
            plan_id,
            plan.transcription_planned,
            &bezug.anwesende,
            &einwilligungen,
        ) {
            StartModus::MitTranskription => {}
            StartModus::NurNotizen {
                nicht_geplant,
                abgelehnt,
                ..
            } => {
                let grund = if nicht_geplant {
                    "nicht_geplant"
                } else if !abgelehnt.is_empty() {
                    "abgelehnt"
                } else {
                    "offen"
                };
                return Err(format!("nur_notizen: {grund}"));
            }
        }
    }
    Ok(())
}
