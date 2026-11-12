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
//!
//! # Sperrordnung (verbindlich, Fixrunde 3 zu lva#544)
//!
//! Alles, was das Ergebnis des Einwilligungs-Tors beeinflusst oder von ihm abhaengt, laeuft ueber
//! EINE prozessweite Sperre ([`tor_sperre`]). Damit sie nie zum Deadlock fuehrt, gilt:
//!
//! 1. Reihenfolge der Sperren, aussen nach innen, nie umdrehen:
//!    `Recorder::start_guard` -> `FinalSlot::job` (nur kurz, ohne Warten; nimmt der Start NICHT
//!    mehr, wenn er die Tor-Sperre haelt) -> **Tor-Sperre** -> Datenbank-Verbindung
//!    (Schreibsperre `BEGIN IMMEDIATE`). Weitere Sperren kommen nicht hinzu: Zustands-/
//!    Pipeline-Sperren des Recorders und Sperren des Modell-Managers werden NIE gehalten,
//!    waehrend die Tor-Sperre genommen wird, und NIE unter ihr genommen.
//! 2. Die Tor-Sperre ist die innerste vor der Datenbank. Wer sie haelt, nimmt nur noch die
//!    Datenbank-Verbindung und wartet NIE auf einen anderen Thread: kein `join`, kein Kanal, keine
//!    Recorder-Sperre. Einzige, gewollte Ausnahme: der wirksame Beginn einer Aufnahme
//!    (`Recorder::start_row` im `start` von [`mit_tor`]) wartet hoechstens `LOOPBACK_START_TIMEOUT`
//!    auf den Start des Systemton-Threads, der keine Planungs-Sperre kennt; dafuer gilt die
//!    Atomaritaet von Tor und Beginn.
//! 3. Unter der Sperre laeuft nie ein Modell (Spracherkennung, Sprecher-Modell). Ein Modell
//!    laeuft zwischen zwei kurzen Pruefungen: [`modell_lauf`] prueft vorher und nachher je unter
//!    der Sperre, der Aufruf selbst laeuft ohne sie; ein inzwischen geaendertes Tor verwirft das
//!    Ergebnis.
//! 4. Wer eine Aufnahme (neu) startet, beendet zuerst (OHNE Tor-Sperre) laufende nachtraegliche
//!    Laeufe und wartet auf sie; erst dann folgt [`mit_tor`] mit der frischen Pruefung und dem
//!    wirksamen Beginn. Beginnt zwischen Warten und Tor ein neuer Lauf, merkt der Recorder es
//!    (`final_gen`) und wiederholt Warten und Tor.
//! 5. Die Sperre ist nicht wiedereintrittsfaehig; ein zweiter Zugriff im selben Thread meldet
//!    `debug_assert!`.
//!
//! Durchgesetzt wird das, wo es der Compiler kann, durch Typen ([`TorNachweis`]: Start ohne Tor
//! nicht aufrufbar), sonst durch [`ohne_tor_sperre`]: jeder `join` und jeder Modellaufruf ruft den
//! Pruefpunkt, der in Debug-Laeufen (Tests) bei gehaltener Sperre mit `panic!` abbricht.

pub mod adapter_c;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod testhilfe;

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

/// Eine Planung, die zu einem Start gehoert, und die Anwesenden, gegen die sie geprueft wird.
struct StartBezug {
    /// `(plan_id, anwesende)`; eine Planung kann mehrfach vorkommen (mehrere Quellen), dann muss
    /// sie gegen JEDE Anwesendenliste bestehen.
    plaene: Vec<(String, Vec<String>)>,
}

/// Ein Kalendertermin, der zum Start gehoert: (UID, Instanz: `None` = unbekannt, `Some(None)` =
/// Einzeltermin) und seine Anwesenden (leer, wenn der Termin nicht mehr im Cache ist).
struct StartTermin {
    uid: String,
    instanz: Option<Option<i64>>,
    anwesende: Vec<String>,
}

fn anwesende_von(e: &crate::managers::calendar::model::CalEvent) -> Vec<String> {
    e.attendees
        .iter()
        .map(|a| {
            a.email
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map_or_else(|| consent::UNBEKANNT.to_string(), str::to_string)
        })
        .collect()
}

/// Liest den Termin frisch aus dem Cache; fehlt er, gilt `ersatz` (UID und Instanz des
/// Schnappschusses) ohne Anwesende.
fn termin_aus_schluessel(
    store: &MeetingStore,
    termine: &mut Vec<StartTermin>,
    key: &str,
    ersatz: Option<(String, Option<Option<i64>>)>,
) -> Res<()> {
    match store.calendar_event(key).map_err(|e| format!("sql: {e}"))? {
        Some(e) => termine.push(StartTermin {
            uid: e.uid.clone(),
            instanz: Some(e.instance_start),
            anwesende: anwesende_von(&e),
        }),
        None => {
            if let Some((uid, instanz)) = ersatz {
                termine.push(StartTermin {
                    uid,
                    instanz,
                    anwesende: Vec::new(),
                });
            }
        }
    }
    Ok(())
}

/// Findet die Planungen der Besprechung, die gleich startet.
///
/// Verknuepfung (der kleinste saubere Weg, ohne neue Tabelle): Besprechung und Planung teilen den
/// KALENDERTERMIN. Quellen, die ALLE zusammen gelten (nie eine statt der anderen, sonst verdeckte
/// ein uebergebener Termin die Verknuepfung des Ziels):
///
/// 1. der uebergebene Termin (`event`, bei `meetings_start_from_event` und der Workflow-Aktion),
///    nur als Schluessel (Key bzw. UID + Instanz); Termin und Teilnehmende kommen frisch aus dem
///    Cache;
/// 2. der Termin, mit dem die Zielbesprechung (`target_meeting_id`) ueber `meeting_calendar_links`
///    verknuepft ist;
/// 3. Planungen, die die Zielbesprechung ausdruecklich fuehren (`meeting_plans.meeting_id`).
///
/// Zu einem Termin gehoert eine Planung, wenn ihre Verknuepfung (Adapter C, `meeting_plan_links`)
/// dieselbe UID und Instanz traegt. Eine ausdrueckliche `plan_id` beim Start waere dagegen
/// freiwillig: wer sie weglaesst, umginge das Tor.
///
/// Ist ein Termin nicht mehr im Cache (nur noch der Schnappschuss), ist die Instanz unbekannt:
/// dann zaehlen alle Planungen der UID, und es gibt keine Anwesenden (sperrt). Planungen aus
/// Quelle 3 werden gegen die Anwesenden ALLER bekannten Termine geprueft (keine: sperrt).
fn start_bezug(
    conn: &Connection,
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<StartBezug> {
    let mut termine: Vec<StartTermin> = Vec::new();
    if let Some(e) = event {
        termin_aus_schluessel(
            store,
            &mut termine,
            &e.key,
            Some((e.uid.clone(), Some(e.instance_start))),
        )?;
    }
    if let Some(target) = target_meeting_id {
        if let Some(link) = store
            .meeting_calendar_link(target)
            .map_err(|e| format!("sql: {e}"))?
        {
            let ersatz = Some((link.uid.clone(), None));
            match link.event_key.as_deref() {
                Some(key) => termin_aus_schluessel(store, &mut termine, key, ersatz)?,
                None => termine.extend(ersatz.map(|(uid, instanz)| StartTermin {
                    uid,
                    instanz,
                    anwesende: Vec::new(),
                })),
            }
        }
    }
    let mut plaene: Vec<(String, Vec<String>)> = Vec::new();
    for t in &termine {
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
        let flach: Option<i64> = t.instanz.and_then(|i| i);
        let rows = st
            .query_map(params![t.uid, t.instanz.is_none(), flach], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| format!("sql: {e}"))?;
        for id in rows {
            let id = id.map_err(|e| format!("sql: {e}"))?;
            let paar = (id, t.anwesende.clone());
            if !plaene.contains(&paar) {
                plaene.push(paar);
            }
        }
    }
    if let Some(target) = target_meeting_id {
        let alle: Vec<String> = termine
            .iter()
            .flat_map(|t| t.anwesende.iter().cloned())
            .collect();
        let mut st = conn
            .prepare("SELECT id FROM meeting_plans WHERE meeting_id = ?1 AND deleted_at IS NULL")
            .map_err(|e| format!("sql: {e}"))?;
        let rows = st
            .query_map([target], |r| r.get::<_, String>(0))
            .map_err(|e| format!("sql: {e}"))?;
        for id in rows {
            let paar = (id.map_err(|e| format!("sql: {e}"))?, alle.clone());
            if !plaene.contains(&paar) {
                plaene.push(paar);
            }
        }
    }
    Ok(StartBezug { plaene })
}

// ---------------------------------------------------------------------------
// Die Tor-Sperre und ihre Ordnung
// ---------------------------------------------------------------------------

std::thread_local! {
    /// Haelt DIESER Thread gerade die Tor-Sperre? (Nur fuer die Pruefung der Sperrordnung.)
    static UNTER_TOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
std::thread_local! {
    /// Testhaken: wird in `tor_sperre()` aufgerufen, BEVOR auf die Sperre gewartet wird. So
    /// beweist ein Test, dass der Schreiber die Sperre wirklich erreicht hat (statt zu raten).
    pub(crate) static VERSUCH: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Die prozessweite Sperre des Einwilligungs-Tors. Wer das Tor-Ergebnis beeinflussen kann
/// (Einwilligung setzen, Planung aendern, Termin wechseln bzw. Identitaet loesen, Abgleich mit dem
/// Kalender, Schreiben des Kalender-Caches), nimmt sie VOR dem Schreiben; wer eine Transkription
/// startet, haelt sie vom Tor bis zum wirksamen Beginn ([`mit_tor`]); wer ein Modell ueber Ton der
/// Besprechung laufen laesst, prueft das Tor davor und danach unter ihr ([`modell_lauf`]). Ein
/// Widerruf, der waehrend eines Starts gespeichert wird, wartet damit bis nach dem Start und gilt
/// fuer den naechsten Aufruf.
static TOR_SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Die gehaltene Tor-Sperre. Beim Freigeben faellt die Merkung des Threads (Sperrordnung, Punkt 3).
pub struct TorSperre {
    _sperre: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TorSperre {
    fn drop(&mut self) {
        UNTER_TOR.with(|c| c.set(false));
    }
}

/// Nimmt die Tor-Sperre (nicht wiedereintrittsfaehig; ein zweiter Aufruf im selben Thread waere
/// ein Selbst-Deadlock und wird in Debug-Laeufen sofort gemeldet).
pub fn tor_sperre() -> TorSperre {
    #[cfg(test)]
    VERSUCH.with(|v| {
        if let Some(f) = v.borrow().as_ref() {
            f();
        }
    });
    debug_assert!(
        !UNTER_TOR.with(|c| c.get()),
        "Sperrordnung verletzt: die Tor-Sperre wird im selben Thread zum zweiten Mal genommen"
    );
    // `Mutex<()>` traegt keine Daten: eine Vergiftung darf nie alle spaeteren Starts blockieren.
    let g = TOR_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
    UNTER_TOR.with(|c| c.set(true));
    TorSperre { _sperre: g }
}

/// Haelt dieser Thread gerade die Tor-Sperre?
pub fn unter_tor_sperre() -> bool {
    UNTER_TOR.with(|c| c.get())
}

/// Pruefpunkt der Sperrordnung: ruft jeder Code VOR einer Handlung, die unter der Tor-Sperre
/// verboten ist (auf einen anderen Thread warten, ein Modell laufen lassen). In Debug-Laeufen
/// (`cfg(debug_assertions)`, also Tests) ein `panic!`, im Release ohne Wirkung.
#[track_caller]
pub fn ohne_tor_sperre(was: &str) {
    if cfg!(debug_assertions) && unter_tor_sperre() {
        panic!("Sperrordnung verletzt: {was} unter der Tor-Sperre");
    }
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
/// wiedereintrittsfaehig), nicht auf andere Threads warten und kein Modell laufen lassen
/// (Sperrordnung). Bei `NurNotizen` wird `start` nie aufgerufen.
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

/// Prueft das Tor fuer eine vorhandene Besprechung unter der Sperre (kurz, ohne etwas zu starten).
pub fn tor_pruefen_besprechung(store: &MeetingStore, meeting_id: &str) -> Res<()> {
    let _sperre = tor_sperre();
    start_tor(store, None, Some(meeting_id))
}

/// EIN Modellaufruf ueber Ton dieser Besprechung (Spracherkennung je Block bzw. Lauf,
/// Sprecher-Modell). Das Tor wird VOR dem Aufruf unter der Sperre geprueft; die Sperre ist beim
/// Aufruf selbst frei (ein Modell darf nie unter ihr laufen, sonst stuende jeder Widerruf bis zum
/// Ende der Erkennung). Danach wird das Tor unter der Sperre erneut geprueft: hat sich die Lage
/// inzwischen geaendert (Widerruf, Planungs-/Terminwechsel, neuer Kalenderstand), wird das Ergebnis
/// verworfen (`Err`, der Aufrufer speichert nichts und ruft nicht weiter). Fehler:
/// `nur_notizen: <grund>`; Fehler des Aufrufs selbst gehen unveraendert durch.
///
/// Warum kein Zaehler je Planung: das erneute Lesen nach dem Aufruf ist die staerkere Pruefung
/// (sie sieht auch Aenderungen anderer Verbindungen und Prozesse) und braucht keine Migration.
pub fn modell_lauf<T>(
    store: &MeetingStore,
    meeting_id: &str,
    call: impl FnOnce() -> Res<T>,
) -> Res<T> {
    modell_lauf_mit(store, meeting_id, call, |code| code)
}

/// [`modell_lauf`] fuer Aufrufe mit eigenem Fehlertyp: `abgelehnt` macht aus dem Code des Tors
/// (`nur_notizen: <grund>`) den Fehler des Aufrufers.
pub fn modell_lauf_mit<T, E>(
    store: &MeetingStore,
    meeting_id: &str,
    call: impl FnOnce() -> Result<T, E>,
    abgelehnt: impl Fn(String) -> E,
) -> Result<T, E> {
    ohne_tor_sperre("Modellaufruf");
    tor_pruefen_besprechung(store, meeting_id).map_err(&abgelehnt)?;
    let out = call()?;
    tor_pruefen_besprechung(store, meeting_id).map_err(&abgelehnt)?;
    Ok(out)
}

/// Ist `fehler` die Ablehnung des Tors (`nur_notizen: ...`)?
pub fn ist_ablehnung(fehler: &str) -> bool {
    fehler.starts_with("nur_notizen")
}

/// Das Einwilligungs-Tor: laeuft UNMITTELBAR vor dem Start einer Aufnahme mit
/// Live-Transkription (und vor jeder nachtraeglichen Transkription) und liest dabei alles frisch
/// aus der Datenbank: Einwilligungen, Termin und Teilnehmende (ein Widerruf zwischen einer
/// Abfrage per [`start_modus`] und dem Start wirkt damit). Allein ohne Sperre ist das nur eine
/// Momentaufnahme; atomar mit dem Start ist es nur ueber [`mit_tor`]. Gehoert die Besprechung zu
/// keiner Planung, passiert nichts. Sonst muss JEDE zugehoerige Planung gegen JEDE Anwesendenliste
/// [`StartModus::MitTranskription`] ergeben (siehe [`start_bezug`]); bei `NurNotizen` startet
/// nichts (kein Ton, keine Transkription) und der Fehler ist `nur_notizen: <grund>`
/// (`nicht_geplant`, `abgelehnt` oder `offen`; nie Namen). Personen ohne Identitaet und fehlende
/// Anwesende sperren.
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
    for (plan_id, anwesende) in &bezug.plaene {
        let plan = plan_holen(&tx, plan_id)?;
        let einwilligungen = plan_store::einwilligungen(&tx, plan_id)?;
        match consent::start_modus(
            plan_id,
            plan.transcription_planned,
            anwesende,
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
