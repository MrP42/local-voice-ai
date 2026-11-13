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
//! # Klasse und Durchsetzungspunkte (Fixrunde 5 zu lva#544)
//!
//! Regel (Konzept 3.5): Ton einer Besprechung, die einer Planung zugeordnet ist, erreicht ein
//! Modell nur, wenn ALLE Anwesenden nachweislich zugestimmt haben; ein Modellergebnis aendert das
//! Transkript nur, wenn das beim Schreiben noch gilt. Durchgesetzt wird das an DREI Stellen, nicht
//! je Ablauf:
//!
//! 1. **Zuordnung** ([`BINDUNGEN_MIGRATION`]): jede Planung, unter der eine Aufnahme wirksam
//!    begann oder ein Modell freigegeben wurde, wird samt der dabei geprueften Anwesenden an die
//!    Besprechung gebunden (`meeting_plan_bindings`), und ebenso der TERMIN, unter dem eine
//!    Aufnahme begann (`meeting_start_events`: eine Planung, die ihm erst spaeter zugeordnet
//!    wird, gilt damit auch ohne Kalender-Verknuepfung). Beides entsteht unter der Tor-Sperre,
//!    VOR dem wirksamen Beginn ([`TorNachweis::binden`], der Beginn verlangt [`Gebunden`]), je
//!    Vorgang des Tors in EINER Schreib-Transaktion ([`bindung_schreiben`]: alle Planungen und der
//!    Termin oder nichts; ein Fehler mittendrin laesst keine halbe Zuordnung zurueck), und
//!    geht nie mehr verloren: eine ersetzte Kalender-Verknuepfung (`INSERT OR REPLACE` in
//!    `link_meeting_event`, erst nach dem Start geschrieben, ein Fehler dort nur eine Warnung),
//!    ein entfernter Kalender oder ein Zusammenfuehren aendern es nicht. Das Tor prueft immer
//!    ALLE Quellen zusammen ([`start_bezug`], [`alle_paare`]): Bindungen, den uebergebenen
//!    Termin, die Termine frueherer Starts, die Kalender-Verknuepfung und
//!    `meeting_plans.meeting_id`, und fuer einen zum Anhaengen vorgemerkten Teil
//!    (`meeting_append_pending`) zusaetzlich alle Quellen seiner Zielbesprechung. Sperrt eine
//!    davon, sperrt das Tor; widerspruechliche Quellen werden nie gegeneinander ausgetauscht.
//! 2. **Modell** ([`Freigabe`]): jede einzelne Inferenz ueber Ton (Spracherkennung, Sprachprobe,
//!    Sprechertrennung, Stimmvergleich) verlangt eine `&Freigabe` als Parameter
//!    (`TranscriptionManager::transcribe_segments*`, `ChannelDiarizer::diarize`,
//!    `voiceprint::Embedder::embed`). Eine Freigabe gibt es nur aus [`modell_lauf`] (Tor davor UND
//!    danach, auch nach Fehler und Panik) und sie gilt fuer genau EINEN Aufruf: ein zweiter
//!    Versuch oder ein weiteres Fenster mit derselben Freigabe scheitert (`freigabe_verbraucht`).
//!    Fuer Ton, der keiner Besprechung gehoert, gibt es nur den benannten Ausweg
//!    [`Freigabe::ohne_besprechung`]. Das Tor sperrt auch, wenn es sich NICHT PRUEFEN laesst
//!    (Datenbankfehler): [`TorFehler`] kennt nur "abgelehnt" und "nicht pruefbar", beides ist
//!    nach aussen der Code `nur_notizen: ...`. Wer einen Modellaufruf wiederholt, bekommt die
//!    Ursache als Typ ([`modell_versuch`], [`LaufFehler`]): wiederholt und als Luecke vermerkt
//!    wird nur ein Fehler des Aufrufs, nie einer des Tors.
//! 3. **Schreiben** ([`Modellarbeit`], [`schreibschutz`]): `modell_lauf` verlangt eine laufende
//!    `Modellarbeit` des Threads fuer die Besprechung. Solange sie lebt, prueft der Speicher JEDE
//!    Aenderung am Transkript dieser Besprechung (Segmente, Sprecher, Hinweise:
//!    `append_delta`, `replace_segments*`, `update_segments_with_speakers`, `clear_segments`) im
//!    selben Schritt unter der Tor-Sperre. Ist das Tor zu, wird nichts geschrieben (auch keine
//!    Luecke). Aktive Fassung, Lauf-Marke und der Status eines Laufs wechseln nur in Schritten,
//!    die unter [`mit_tor_besprechung`] stehen (`final_pass::run_job`: Vorbereitung;
//!    `retranscribe`: Beginn, Leeren, Abschluss). Was das Tor beendet, endet FORTSETZBAR, nie als
//!    erledigt: Nachholen und Recovery `cancelled` ohne Luecke (`final_pass::finish_stopped`), ein
//!    Import mit Audiodatei ebenso (`import::mark_refused`), eine Neu-Transkription wird
//!    zurueckgenommen (das vorherige Transkript steht wieder).
//!
//! # Sperrordnung (verbindlich)
//!
//! Alles, was das Ergebnis des Einwilligungs-Tors beeinflusst oder von ihm abhaengt, laeuft ueber
//! EINE prozessweite Sperre ([`tor_sperre`]). Damit sie nie zum Deadlock fuehrt, gilt:
//!
//! 1. Reihenfolge der Sperren, aussen nach innen, nie umdrehen:
//!    `Recorder::start_guard` bzw. `parts::MERGE_LOCK` -> **Tor-Sperre** -> Datenbank-Verbindung
//!    (Schreibsperre `BEGIN IMMEDIATE`).
//! 2. Die Tor-Sperre ist die innerste vor der Datenbank. Wer sie haelt, nimmt nur noch die
//!    Datenbank-Verbindung (und kurze Dateiarbeit ohne fremden Thread) und wartet NIE auf einen
//!    anderen Thread: kein `join`, kein Kanal, keine Sperre des Recorders (`RunCore`), keine des
//!    Modell-Managers, kein Start von Mikrofon oder Systemton. Der Aufbau einer Aufnahme
//!    (`Recorder::start_row`) laeuft deshalb NICHT unter ihr: unter der Sperre stehen nur Tor,
//!    Zeile und Bindung, spaeter der wirksame Beginn ([`Gebunden::wirksam`]: frische Pruefung und
//!    ein Merker); dazwischen und danach schuetzen Bindung und die Punkte 2 und 3 oben jeden
//!    Block.
//!    Erlaubt unter ihr sind sonst nur BLATT-Sperren: kurz, ohne Warten, und nie gehalten,
//!    waehrend eine andere Sperre genommen wird (also auch nie ueber die Tor-Sperre hinweg). Das
//!    sind das Verzeichnis der Auftraege (`meetings::job::global()`, in `retranscribe::begin_run`),
//!    der Weckruf der Import-Warteschlange (`ImportQueue::enqueue`, in
//!    `parts::enqueue_appended`) und die Sperre des `FinalSlot` selbst (`spawn`, `active`,
//!    `live_aktiv`); das WARTEN im `FinalSlot` (`live_beginnen`, `wait`) ist dagegen unter der
//!    Tor-Sperre verboten und geprueft.
//! 3. Unter der Sperre laeuft nie ein Modell. Ein Modell laeuft zwischen zwei kurzen Pruefungen
//!    ([`modell_lauf`]); ein inzwischen geaendertes Tor verwirft das Ergebnis.
//! 4. Wer eine Aufnahme (neu) startet, meldet sie zuerst im `FinalSlot` an (`live_beginnen`): ab
//!    da beginnt kein nachtraeglicher Lauf mehr mit Modell (er wird abgebrochen geboren), laufende
//!    werden abgebrochen und mit Frist abgewartet (OHNE Tor-Sperre). Erst dann folgt [`mit_tor`].
//! 5. Die Sperre ist nicht wiedereintrittsfaehig; ein zweiter Zugriff im selben Thread meldet
//!    `debug_assert!`.
//!
//! Durchgesetzt wird das, wo es der Compiler kann, durch Typen ([`TorNachweis`], [`Gebunden`],
//! [`Freigabe`]), sonst durch [`ohne_tor_sperre`]: jedes Warten auf den Enddurchlauf
//! (`FinalSlot::live_beginnen`, `FinalSlot::wait`), jeder Modellaufruf ([`modell_lauf_mit`],
//! `TranscriptionManager::transcribe_segments_detecting`), das Laden eines Modells
//! (`initiate_model_load_inner`), jede Sperre des Recorders (`RunCore::publish_started`,
//! `begin_stop`, `set_paused`), der Aufbau der Aufnahme (`start_row`) und der Start von Mikrofon
//! und Systemton rufen den Pruefpunkt, der in Debug-Laeufen (Tests) bei gehaltener Sperre mit
//! `panic!` abbricht.
//!
//! Wartezeiten: unter `start_guard` wartet ein Start hoechstens `recorder::CANCEL_WAIT` auf das
//! Ende eines abgebrochenen Enddurchlaufs (ein laufender Aufruf der Spracherkennung und ein
//! laufendes Laden lassen sich nicht unterbrechen); danach scheitert der START (`job_busy`),
//! statt zu haengen. Unter der Tor-Sperre gibt es kein Warten ausser auf die Datenbank
//! (Busy-Timeout der Verbindung).

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
/// 3. die Termine, unter denen eine Aufnahme der Zielbesprechung begann
///    (`meeting_start_events`, [`StartTerminZeile`]);
/// 4. Planungen, die die Zielbesprechung ausdruecklich fuehren (`meeting_plans.meeting_id`).
///
/// Zu einem Termin gehoert eine Planung, wenn ihre Verknuepfung (Adapter C, `meeting_plan_links`)
/// dieselbe UID und Instanz traegt. Eine ausdrueckliche `plan_id` beim Start waere dagegen
/// freiwillig: wer sie weglaesst, umginge das Tor.
///
/// Ist ein Termin nicht mehr im Cache (nur noch der Schnappschuss), ist die Instanz unbekannt:
/// dann zaehlen alle Planungen der UID, und es gibt keine Anwesenden (sperrt). Planungen aus
/// Quelle 4 werden gegen die Anwesenden ALLER bekannten Termine geprueft (keine: sperrt).
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
    if let Some(target) = target_meeting_id {
        // Termine, unter denen eine Aufnahme dieser Besprechung begann (gebunden beim Start).
        for t in start_termine(conn, target)? {
            termin_aus_schluessel(store, &mut termine, &t.key, Some((t.uid, Some(t.instanz))))?;
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

/// Eine Planung und die Anwesenden, gegen die sie geprueft wird.
type Paar = (String, Vec<String>);

/// Nachweis, dass das Einwilligungs-Tor fuer diesen Start bestanden wurde und die Sperre noch
/// gehalten wird. Nur [`mit_tor`] erzeugt ihn. Er traegt die Planungen, gegen die geprueft wurde:
/// [`TorNachweis::binden`] schreibt sie an die Besprechung, und erst der so entstandene
/// [`Gebunden`] oeffnet den wirksamen Beginn (Compiler statt Zeichenkettenpruefung).
pub struct TorNachweis {
    paare: Vec<Paar>,
    /// Die Zielbesprechung des Tors und was fuer sie in dieser Pruefung NEU gebunden wurde
    /// (Planungen und Termin, schon geschrieben: [`tor_und_binden`]).
    ziel: Option<(String, NeuGebunden)>,
    /// Der uebergebene Termin des Starts: er wird mit gebunden ([`START_TERMINE`]), damit auch
    /// eine Planung gilt, die ihm erst NACH dem Start zugeordnet wird.
    termin: Option<StartTerminZeile>,
    _nur_hier: (),
}

impl TorNachweis {
    /// Bindet die geprueften Planungen und den Termin des Starts an die Besprechung dieses Starts
    /// (unter der noch gehaltenen Sperre, VOR dem wirksamen Beginn). Ohne Planung entsteht keine
    /// Planungs-Zeile; den Nachweis gibt es trotzdem. Geschrieben wird in EINER Transaktion
    /// ([`bindung_schreiben`]): scheitert etwas davon, steht NICHTS davon in der Datenbank, es
    /// gibt keinen Nachweis und damit keinen Beginn (`nur_notizen: tor_nicht_pruefbar`). Fuer die
    /// Zielbesprechung des Tors ist schon alles geschrieben ([`tor_und_binden`]): hier entsteht
    /// nur noch der Nachweis.
    pub fn binden(&self, store: &MeetingStore, meeting_id: &str) -> Res<Gebunden> {
        let neu = match &self.ziel {
            Some((ziel, neu)) if ziel == meeting_id => neu.clone(),
            // Ein Fehler beim Binden ist ein Fehler des Tors (wie in [`tor_und_binden`]).
            _ => bindung_schreiben(store, meeting_id, &self.paare, self.termin.as_ref())
                .map_err(|e| TorFehler::nicht_pruefbar(e).code())?,
        };
        Ok(Gebunden {
            meeting_id: meeting_id.to_string(),
            neu: neu.plaene,
            termin_neu: neu.termin,
            _nur_hier: (),
        })
    }
}

/// Nachweis, dass die Planungen des Starts an die Besprechung gebunden sind
/// (`meeting_plan_bindings`). Nur [`TorNachweis::binden`] erzeugt ihn; der wirksame Beginn einer
/// Aufnahme (`Recorder::start_row`) verlangt ihn. Er gilt ueber die Sperre hinaus: ab der Bindung
/// prueft jeder Modellaufruf und jedes Schreiben das Tor mit diesen Planungen.
pub struct Gebunden {
    meeting_id: String,
    /// Planungen, die dieser Start neu gebunden hat (fuer [`bindung_verwerfen`]).
    neu: Vec<String>,
    /// Der Termin, den dieser Start neu gebunden hat (Schluessel; fuer [`bindung_verwerfen`]).
    termin_neu: Option<String>,
    _nur_hier: (),
}

impl Gebunden {
    pub fn meeting_id(&self) -> &str {
        &self.meeting_id
    }

    /// Der WIRKSAME Beginn der Aufnahme: nimmt die Tor-Sperre, prueft das Tor fuer die gebundene
    /// Besprechung noch einmal frisch und ruft, wenn es besteht, `schalten` unter der Sperre.
    /// Zwischen der Bindung (unter der ersten Sperre) und hier baut der Start Dateien, Pipeline
    /// und Captures OHNE Sperre auf; ein Widerruf in dieser Zeit gewinnt hier (`Err`, `schalten`
    /// laeuft nie), ein spaeterer wartet bis nach `schalten` und trifft den naechsten
    /// Modellaufruf. `schalten` legt nur einen Merker um: keine Sperre, kein Warten, kein Modell.
    pub fn wirksam(&self, store: &MeetingStore, schalten: impl FnOnce()) -> Res<()> {
        let _sperre = tor_sperre();
        tor_paare(store, None, Some(&self.meeting_id)).map_err(|e| e.code())?;
        schalten();
        Ok(())
    }
}

/// Der Start ist gescheitert, bevor Ton aufgenommen wurde: die Bindungen, die ER angelegt hat,
/// fallen wieder weg (aeltere bleiben), alle in EINER Transaktion. Ein Fehler hier laesst ALLE
/// stehen (sperrend), nie einen Teil davon.
pub fn bindung_verwerfen(store: &MeetingStore, gebunden: Gebunden) {
    if gebunden.neu.is_empty() && gebunden.termin_neu.is_none() {
        return;
    }
    let _sperre = tor_sperre();
    let verworfen = in_tx(store, |tx| {
        for plan_id in &gebunden.neu {
            tx.execute(
                "DELETE FROM meeting_plan_bindings WHERE meeting_id = ?1 AND plan_id = ?2",
                params![gebunden.meeting_id, plan_id],
            )
            .map_err(sql)?;
        }
        if let Some(key) = &gebunden.termin_neu {
            tx.execute(
                "DELETE FROM meeting_start_events WHERE meeting_id = ?1 AND event_key = ?2",
                params![gebunden.meeting_id, key],
            )
            .map_err(sql)?;
        }
        Ok(())
    });
    if let Err(e) = verworfen {
        // Nur der Fehlertext der Datenbank, nie Inhalte.
        log::warn!("planning: Bindung eines gescheiterten Starts bleibt stehen und sperrt ({e})");
    }
}

// ---------------------------------------------------------------------------
// Zuordnung: Bindung von Planungen an eine Besprechung
// ---------------------------------------------------------------------------

/// Migration Index 18 der Besprechungs-Datenbank: welche Planungen fuer eine Besprechung gelten,
/// seit unter ihnen eine Aufnahme begann oder ein Modell freigegeben wurde, und gegen welche
/// Anwesenden dabei geprueft wurde. Mehrere Planungen je Besprechung sind moeglich (uebergebener
/// Termin UND Termin des Zieleintrags, zusammengefuehrte Teile). `meeting_id` hat bewusst keinen
/// Fremdschluessel: die Bindung ueberlebt den Papierkorb der Besprechung. Nur CREATE.
pub(crate) const BINDUNGEN_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS meeting_plan_bindings (
        meeting_id TEXT NOT NULL,
        plan_id TEXT NOT NULL REFERENCES meeting_plans(id) ON DELETE CASCADE,
        attendees_json TEXT NOT NULL DEFAULT '[]',
        created_at INTEGER NOT NULL,
        PRIMARY KEY (meeting_id, plan_id)
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_plan_bindings_plan
        ON meeting_plan_bindings(plan_id);
    CREATE TABLE IF NOT EXISTS meeting_start_events (
        meeting_id TEXT NOT NULL,
        event_key TEXT NOT NULL,
        uid TEXT NOT NULL,
        instance_start INTEGER,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (meeting_id, event_key)
    );";

/// Ein Termin, unter dem eine Aufnahme der Besprechung begann (`meeting_start_events`, Teil von
/// [`BINDUNGEN_MIGRATION`]): Schluessel im Kalender-Cache, UID und Instanz (`None` =
/// Einzeltermin). Anders als die Verknuepfung `meeting_calendar_links` (eine je Besprechung,
/// `INSERT OR REPLACE`, erst NACH dem Start geschrieben, ein Fehler dort ist nur eine Warnung)
/// entsteht die Zeile unter der Tor-Sperre VOR dem wirksamen Beginn und wird nie ersetzt. Das Tor
/// liest sie als Quelle wie die Verknuepfung: eine Planung, die dem Termin erst nach dem Start
/// zugeordnet wird, gilt damit ab ihrer Zuordnung fuer die Besprechung.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StartTerminZeile {
    key: String,
    uid: String,
    instanz: Option<i64>,
}

impl StartTerminZeile {
    fn von(event: &crate::managers::calendar::model::CalEvent) -> Self {
        Self {
            key: event.key.clone(),
            uid: event.uid.clone(),
            instanz: event.instance_start,
        }
    }
}

/// Bindet den Termin des Starts an die Besprechung. `true`: die Zeile ist neu. Nur unter der
/// Tor-Sperre und in der Transaktion des Aufrufers ([`bindung_schreiben`]).
fn termin_binden(conn: &Connection, meeting_id: &str, t: &StartTerminZeile) -> Res<bool> {
    debug_assert!(!conn.is_autocommit(), "Bindung nur in einer Transaktion");
    let neu = conn
        .execute(
            "INSERT OR IGNORE INTO meeting_start_events
               (meeting_id, event_key, uid, instance_start, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                meeting_id,
                t.key,
                t.uid,
                t.instanz,
                chrono::Utc::now().timestamp_millis()
            ],
        )
        .map_err(sql)?;
    Ok(neu > 0)
}

/// Die Termine, unter denen Aufnahmen der Besprechung begannen.
fn start_termine(conn: &Connection, meeting_id: &str) -> Res<Vec<StartTerminZeile>> {
    let mut st = conn
        .prepare(
            "SELECT event_key, uid, instance_start FROM meeting_start_events
             WHERE meeting_id = ?1 ORDER BY event_key",
        )
        .map_err(sql)?;
    let rows = st
        .query_map([meeting_id], |r| {
            Ok(StartTerminZeile {
                key: r.get(0)?,
                uid: r.get(1)?,
                instanz: r.get(2)?,
            })
        })
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

/// Die an die Besprechung gebundenen, nicht geloeschten Planungen mit ihren Anwesenden.
fn bindungen(conn: &Connection, meeting_id: &str) -> Res<Vec<Paar>> {
    let mut st = conn
        .prepare(
            "SELECT b.plan_id, b.attendees_json FROM meeting_plan_bindings b
             JOIN meeting_plans p ON p.id = b.plan_id
             WHERE b.meeting_id = ?1 AND p.deleted_at IS NULL
             ORDER BY b.plan_id",
        )
        .map_err(sql)?;
    let rows = st
        .query_map([meeting_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql)?;
    let mut out = Vec::new();
    for row in rows {
        let (plan_id, json) = row.map_err(sql)?;
        // Eine unlesbare Liste gilt als leer: ohne Anwesende sperrt das Tor.
        let anwesende: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        out.push((plan_id, anwesende));
    }
    Ok(out)
}

/// Was EIN Vorgang des Tors an einer Besprechung NEU gebunden hat (fuer [`bindung_verwerfen`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NeuGebunden {
    plaene: Vec<String>,
    /// Schluessel des neu gebundenen Termins.
    termin: Option<String>,
}

/// Die Zeile, die [`binden`] fuer EINE Planung schreiben muesste: die Anwesenden als JSON und ob
/// die Bindung neu ist. `None`: die Bindung steht schon so da. Die Anwesenden einer schon
/// gebundenen Planung werden VEREINIGT (wer einmal als anwesend geprueft wurde, bleibt es), als
/// Identitaeten; eine Person OHNE Identitaet bleibt als `unbekannt` stehen und sperrt weiter (nach
/// bestandenem Tor gibt es keine; beim Zusammenfuehren darf sie nicht still wegfallen).
fn bindungs_zeile(
    conn: &Connection,
    meeting_id: &str,
    plan_id: &str,
    anwesende: &[String],
) -> Res<Option<(String, bool)>> {
    let alt: Option<String> = {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT attendees_json FROM meeting_plan_bindings
             WHERE meeting_id = ?1 AND plan_id = ?2",
            params![meeting_id, plan_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
    };
    let mut liste: Vec<String> = alt
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    let vorher = liste.len();
    for person in anwesende
        .iter()
        .map(|a| consent::identitaet(a).unwrap_or_else(|| consent::UNBEKANNT.to_string()))
    {
        if !liste.contains(&person) {
            liste.push(person);
        }
    }
    if alt.is_some() && liste.len() == vorher {
        return Ok(None);
    }
    let json = serde_json::to_string(&liste).map_err(|e| format!("sql: {e}"))?;
    Ok(Some((json, alt.is_none())))
}

/// Schreibt die Bindung jeder Planung aus `paare` an die Besprechung ([`bindungs_zeile`]). Gibt
/// die NEU gebundenen Planungen zurueck. Nur unter der Tor-Sperre und NUR in der Transaktion des
/// Aufrufers ([`bindung_schreiben`], Zusammenfuehren): jede Planung ist ein eigener
/// Schreibzugriff, und scheitert einer, darf keiner der frueheren stehen bleiben.
fn binden(conn: &Connection, meeting_id: &str, paare: &[Paar]) -> Res<Vec<String>> {
    debug_assert!(!conn.is_autocommit(), "Bindung nur in einer Transaktion");
    let jetzt = chrono::Utc::now().timestamp_millis();
    let mut neu = Vec::new();
    for (plan_id, anwesende) in paare {
        let Some((json, ist_neu)) = bindungs_zeile(conn, meeting_id, plan_id, anwesende)? else {
            continue;
        };
        conn.execute(
            "INSERT INTO meeting_plan_bindings (meeting_id, plan_id, attendees_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(meeting_id, plan_id) DO UPDATE SET attendees_json = excluded.attendees_json",
            params![meeting_id, plan_id, json, jetzt],
        )
        .map_err(sql)?;
        if ist_neu && !neu.contains(plan_id) {
            neu.push(plan_id.clone());
        }
    }
    Ok(neu)
}

/// Gibt es fuer diesen Vorgang ueberhaupt etwas zu schreiben? Nur Lesen: der Normalfall (alles
/// schon gebunden, vor und nach JEDEM Modellaufruf) nimmt so keine Schreibsperre der Datenbank.
fn bindung_offen(
    conn: &Connection,
    meeting_id: &str,
    paare: &[Paar],
    termin: Option<&StartTerminZeile>,
) -> Res<bool> {
    for (plan_id, anwesende) in paare {
        if bindungs_zeile(conn, meeting_id, plan_id, anwesende)?.is_some() {
            return Ok(true);
        }
    }
    let Some(t) = termin else { return Ok(false) };
    let gebunden: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meeting_start_events
                           WHERE meeting_id = ?1 AND event_key = ?2)",
            params![meeting_id, t.key],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(!gebunden)
}

/// ALLE Bindungsaenderungen EINES Vorgangs des Tors an einer Besprechung (die geprueften Planungen
/// und der Termin des Starts) in EINER Schreib-Transaktion: entweder steht danach alles oder
/// nichts davon in der Datenbank. Ein Fehler mittendrin (zweite Planung, Termin, Abschluss)
/// nimmt auch die schon geschriebenen Zeilen zurueck: wer es danach wiederholt, findet den Stand
/// von vorher. Nur unter der Tor-Sperre (Sperrordnung: Tor-Sperre -> Datenbank); die Transaktion
/// ist eine eigene auf einer eigenen Verbindung, kein Aufrufer haelt dabei schon eine.
fn bindung_schreiben(
    store: &MeetingStore,
    meeting_id: &str,
    paare: &[Paar],
    termin: Option<&StartTerminZeile>,
) -> Res<NeuGebunden> {
    debug_assert!(unter_tor_sperre(), "Bindung nur unter der Tor-Sperre");
    if paare.is_empty() && termin.is_none() {
        return Ok(NeuGebunden::default());
    }
    let mut conn = verbindung(store)?;
    if !bindung_offen(&conn, meeting_id, paare, termin)? {
        return Ok(NeuGebunden::default());
    }
    let tx = schreib_tx(&mut conn)?;
    let plaene = binden(&tx, meeting_id, paare)?;
    let termin = match termin {
        Some(t) if termin_binden(&tx, meeting_id, t)? => Some(t.key.clone()),
        _ => None,
    };
    tx.commit().map_err(sql)?;
    Ok(NeuGebunden { plaene, termin })
}

/// Zusammenfuehren mehrerer Besprechungen (`parts::apply_merge`): was fuer einen Teil gilt, gilt
/// danach fuer das Ziel. `quellen` sind die Planungen jedes Teils, VOR der Transaktion gelesen
/// ([`bezug_der_besprechung`]); geschrieben wird in der Transaktion der Zusammenfuehrung (hier
/// entsteht keine eigene und keine verschachtelte: scheitert etwas, nimmt der Aufrufer ALLES
/// zurueck). Der Aufrufer haelt die Tor-Sperre.
pub(crate) fn bindungen_uebertragen(
    conn: &Connection,
    ziel: &str,
    quellen: &[Paar],
    teile: &[String],
) -> Res<()> {
    binden(conn, ziel, quellen)?;
    // Auch die Termine der Starts gehen mit: eine Planung, die einem davon spaeter zugeordnet
    // wird, gilt dann fuer das Ziel.
    for teil in teile {
        conn.execute(
            "INSERT OR IGNORE INTO meeting_start_events
               (meeting_id, event_key, uid, instance_start, created_at)
             SELECT ?1, event_key, uid, instance_start, created_at
             FROM meeting_start_events WHERE meeting_id = ?2",
            params![ziel, teil],
        )
        .map_err(sql)?;
    }
    Ok(())
}

/// Alle Planungen, die JETZT fuer die Besprechung selbst gelten (Bindungen und laufende Quellen),
/// mit ihren Anwesenden, ohne Pruefung der Einwilligungen. Der Aufrufer haelt die Tor-Sperre.
/// Ohne die Planungen eines Anhaenge-Ziels: die hat das Ziel schon selbst, und was unter ihnen
/// fuer den Teil freigegeben wurde, steht in dessen Bindungen.
pub(crate) fn bezug_der_besprechung(store: &MeetingStore, meeting_id: &str) -> Res<Vec<Paar>> {
    let conn = verbindung(store)?;
    eigene_paare(&conn, store, None, Some(meeting_id))
}

fn alle_paare(
    conn: &Connection,
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<Vec<Paar>> {
    let mut paare = eigene_paare(conn, store, event, target_meeting_id)?;
    // Ein zum Anhaengen vorgemerkter Teil wird Teil seiner Zielbesprechung: bis dahin gelten fuer
    // ihn AUCH deren Planungen (sonst liefe sein Ton ungeprueft durch das Modell und landete beim
    // Zusammenfuehren im Transkript der geplanten Besprechung). Eine Ebene genuegt: ein wartender
    // Teil nimmt selbst keine Teile an (`meetings_append_files`).
    if let Some(ziel) = target_meeting_id
        .map(|teil| anhaenge_ziel(conn, teil))
        .transpose()?
        .flatten()
    {
        for paar in eigene_paare(conn, store, None, Some(&ziel))? {
            if !paare.contains(&paar) {
                paare.push(paar);
            }
        }
    }
    Ok(paare)
}

/// Die Planungen EINER Besprechung: laufende Quellen ([`start_bezug`]) und Bindungen.
fn eigene_paare(
    conn: &Connection,
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<Vec<Paar>> {
    let mut paare = start_bezug(conn, store, event, target_meeting_id)?.plaene;
    if let Some(target) = target_meeting_id {
        for paar in bindungen(conn, target)? {
            if !paare.contains(&paar) {
                paare.push(paar);
            }
        }
    }
    Ok(paare)
}

/// Die Besprechung, an die `meeting_id` angehaengt werden soll (`meeting_append_pending`).
fn anhaenge_ziel(conn: &Connection, meeting_id: &str) -> Res<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT target_id FROM meeting_append_pending WHERE meeting_id = ?1",
        [meeting_id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sql)
}

/// Fuehrt `start` unter dem Einwilligungs-Tor aus: Sperre nehmen, Tor pruefen (alles frisch aus
/// der Datenbank), bestandene Planungen und den uebergebenen Termin an die Zielbesprechung binden
/// (eine Transaktion: [`bindung_schreiben`]), dann `start` AUSFUEHREN,
/// waehrend die Sperre noch gehalten wird. `start` schreibt nur in die Datenbank (Zeile anlegen,
/// [`TorNachweis::binden`], Marke, Status): es darf nicht selbst Planungsfunktionen mit Sperre
/// rufen (die Sperre ist nicht wiedereintrittsfaehig), nicht auf andere Threads warten, keine
/// Sperre von Recorder oder Modell-Manager nehmen und kein Modell laufen lassen (Sperrordnung).
/// Bei `NurNotizen` wird `start` nie aufgerufen.
pub fn mit_tor<T>(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
    start: impl FnOnce(&TorNachweis) -> Res<T>,
) -> Res<T> {
    let _sperre = tor_sperre();
    let (paare, neu) = tor_und_binden(store, event, target_meeting_id).map_err(|e| e.code())?;
    start(&TorNachweis {
        paare,
        ziel: target_meeting_id.map(|t| (t.to_string(), neu)),
        termin: event.map(StartTerminZeile::von),
        _nur_hier: (),
    })
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

/// Prueft das Tor fuer eine vorhandene Besprechung unter der Sperre (kurz, ohne etwas zu starten)
/// und bindet bestandene Planungen an sie.
pub fn tor_pruefen_besprechung(store: &MeetingStore, meeting_id: &str) -> Res<()> {
    tor_pruefen_typ(store, meeting_id).map_err(|e| e.code())
}

/// [`tor_pruefen_besprechung`] mit dem Fehler als Typ ([`TorFehler`]).
fn tor_pruefen_typ(store: &MeetingStore, meeting_id: &str) -> Result<(), TorFehler> {
    let _sperre = tor_sperre();
    tor_und_binden(store, None, Some(meeting_id)).map(|_| ())
}

/// Tor pruefen und, wenn es besteht, die geprueften Planungen und den uebergebenen Termin an die
/// Zielbesprechung binden. Gibt die geprueften Paare und das NEU Gebundene zurueck. Nur unter der
/// Tor-Sperre. Gebunden wird nur nach bestandenem Tor: eine Planung, unter der nie eine Aufnahme
/// begann und nie ein Modell lief, haengt der Besprechung nicht dauerhaft an.
///
/// Alle Bindungsaenderungen dieses Vorgangs stehen in EINER Transaktion ([`bindung_schreiben`]).
/// Laesst sich nicht ALLES binden, gilt das Tor als nicht bestanden (die Zuordnung waere sonst
/// nicht dauerhaft), und von dem Versuch bleibt keine Zeile zurueck: die Ablehnung
/// `nur_notizen: tor_nicht_pruefbar` hinterlaesst denselben Stand wie vor ihr.
fn tor_und_binden(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Result<(Vec<Paar>, NeuGebunden), TorFehler> {
    debug_assert!(unter_tor_sperre(), "Bindung nur unter der Tor-Sperre");
    let paare = tor_paare(store, event, target_meeting_id)?;
    let neu = match target_meeting_id {
        Some(target) => {
            let termin = event.map(StartTerminZeile::von);
            bindung_schreiben(store, target, &paare, termin.as_ref())
                .map_err(TorFehler::nicht_pruefbar)?
        }
        None => NeuGebunden::default(),
    };
    Ok((paare, neu))
}

/// Warum das Einwilligungs-Tor nicht bestanden ist. Es gibt genau zwei Gruende, und BEIDE sperren
/// (fail-closed): die Regel lehnt ab, oder das Tor liess sich nicht pruefen (Datenbankfehler).
/// Nach aussen ist beides derselbe Code `nur_notizen: <grund>` ([`TorFehler::code`]): ein Fehler
/// des Tors ist damit nie mit einem Fehler des Modells zu verwechseln (kein erneuter Versuch,
/// kein Luecken-Platzhalter, der Rest bleibt fortsetzbar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorFehler {
    /// Die Regel lehnt ab: `nicht_geplant`, `abgelehnt`, `offen` oder `ohne_modellarbeit`.
    Abgelehnt(&'static str),
    /// Das Tor liess sich nicht pruefen. Die Ursache steht nur im Log, nie im Code.
    NichtPruefbar,
}

impl TorFehler {
    /// Ein Fehler beim Lesen oder Binden: die Ursache ins Log (nie Inhalte: es ist der Fehlertext
    /// der Datenbank), nach aussen nur `NichtPruefbar`.
    fn nicht_pruefbar(ursache: String) -> Self {
        log::warn!("planning: Einwilligungs-Tor nicht pruefbar - gesperrt ({ursache})");
        Self::NichtPruefbar
    }

    /// Der Code fuer Befehle und Ereignisse: immer `nur_notizen: <grund>` ([`ist_ablehnung`]).
    pub fn code(&self) -> String {
        match self {
            Self::Abgelehnt(grund) => format!("nur_notizen: {grund}"),
            Self::NichtPruefbar => "nur_notizen: tor_nicht_pruefbar".to_string(),
        }
    }
}

/// Das Ende EINES Modellversuchs hinter dem Tor ([`modell_versuch`]), nach der Ursache getrennt:
/// nur ein Fehler des Aufrufs darf wiederholt oder als Luecke vermerkt werden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaufFehler {
    /// Das Tor hat nicht freigegeben (Ablehnung ODER nicht pruefbar): Schluss, nichts schreiben.
    Tor(TorFehler),
    /// Der Aufruf selbst ist gescheitert (Modell, Engine).
    Aufruf(String),
}

#[cfg(test)]
std::thread_local! {
    /// Testhaken: so viele naechste Lesevorgaenge des Tors scheitern in DIESEM Thread wie bei
    /// einem Datenbankfehler; danach ist "die Datenbank wieder da".
    pub(crate) static TOR_STOERUNG: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// ---------------------------------------------------------------------------
// Modellebene: Freigabe je Inferenz
// ---------------------------------------------------------------------------

/// Ton, der KEINER Besprechung gehoert und deshalb ohne Einwilligungs-Tor an ein Modell darf. Jede
/// Stelle ist hier benannt; eine neue braucht einen neuen Eintrag (und damit eine Begruendung).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OhneBesprechung {
    /// Eine Sekunde Stille nach dem Laden eines Modells (GPU-Pipelines anlegen, Ergebnis verworfen).
    Aufwaermen,
    /// Abgleich eines Hoerbuchs mit seinem Text (`hoersync_live`): eine Mediendatei, kein Gespraech.
    Hoerbuch,
}

/// Die Erlaubnis fuer GENAU EINE Inferenz ueber Ton. Jede Funktion, die ein Modell ueber Ton
/// rechnen laesst, verlangt sie als Parameter und loest sie ein ([`Freigabe::einloesen`]). Fuer
/// Ton einer Besprechung entsteht sie nur in [`modell_lauf`] bzw. [`modell_lauf_mit`] (Tor davor
/// und danach); ein zweiter Aufruf mit derselben Freigabe (Wiederholung, weiteres Fenster)
/// scheitert. Nicht kopierbar, nicht zwischen Threads teilbar.
pub struct Freigabe {
    /// `true`: Freigabe aus dem Tor, gilt fuer einen Aufruf.
    einmalig: bool,
    verbraucht: std::cell::Cell<bool>,
}

/// Fehlercode einer zum zweiten Mal eingeloesten Freigabe.
pub const FREIGABE_VERBRAUCHT: &str = "freigabe_verbraucht";

impl Freigabe {
    /// Freigabe fuer Ton, der keiner Besprechung gehoert (benannter Ausweg, beliebig oft gueltig).
    pub fn ohne_besprechung(_zweck: OhneBesprechung) -> Self {
        Self {
            einmalig: false,
            verbraucht: std::cell::Cell::new(false),
        }
    }

    fn fuer_einen_aufruf() -> Self {
        Self {
            einmalig: true,
            verbraucht: std::cell::Cell::new(false),
        }
    }

    /// Ruft die Inferenz-Funktion als Erstes: die Freigabe aus dem Tor gilt genau einmal.
    pub fn einloesen(&self) -> Res<()> {
        if self.einmalig && self.verbraucht.replace(true) {
            return Err(FREIGABE_VERBRAUCHT.to_string());
        }
        Ok(())
    }

    /// Eine Freigabe wie aus dem Tor, fuer Tests von Inferenz-Funktionen ohne Datenbank.
    #[cfg(test)]
    pub(crate) fn fuer_test() -> Self {
        Self::fuer_einen_aufruf()
    }
}

std::thread_local! {
    /// Besprechungen, fuer die DIESER Thread gerade Modellarbeit leistet (Stapel).
    static MODELLARBEIT: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Ein Abschnitt, in dem dieser Thread Ton einer Besprechung durch ein Modell schickt und die
/// Ergebnisse speichert (Live-Worker, Enddurchlauf, Nachholen, Import, Neu-Transkription,
/// Sprechertrennung, Stimmvergleich). Solange er lebt:
///
/// - [`modell_lauf`] ist fuer diese Besprechung erlaubt (ohne ihn: `nur_notizen: ohne_modellarbeit`);
/// - der Speicher prueft JEDE Aenderung am Transkript dieser Besprechung aus diesem Thread unter
///   der Tor-Sperre ([`schreibschutz`]).
///
/// Er gehoert dem Thread, der ihn begonnen hat (nicht `Send`), und darf verschachtelt werden.
pub struct Modellarbeit {
    meeting_id: String,
    _nicht_send: std::marker::PhantomData<*const ()>,
}

impl Modellarbeit {
    pub fn beginnen(meeting_id: &str) -> Self {
        MODELLARBEIT.with(|m| m.borrow_mut().push(meeting_id.to_string()));
        Self {
            meeting_id: meeting_id.to_string(),
            _nicht_send: std::marker::PhantomData,
        }
    }
}

impl Drop for Modellarbeit {
    fn drop(&mut self) {
        MODELLARBEIT.with(|m| {
            let mut m = m.borrow_mut();
            if let Some(i) = m.iter().rposition(|id| *id == self.meeting_id) {
                m.remove(i);
            }
        });
    }
}

/// Leistet dieser Thread gerade Modellarbeit fuer die Besprechung?
pub fn in_modellarbeit(meeting_id: &str) -> bool {
    MODELLARBEIT.with(|m| m.borrow().iter().any(|id| id == meeting_id))
}

/// Schreibschutz des Speichers (Schreibebene): ruft JEDER Schreiber des Transkripts
/// (`MeetingStore::append_delta`, `write_transcript`, `clear_segments`) als Erstes. Leistet der
/// Thread Modellarbeit fuer die Besprechung, wird das Tor JETZT unter der Sperre geprueft und die
/// Sperre bis zum Ende des Schreibens gehalten (Rueckgabe): zwischen Pruefung und Schreiben kann
/// kein Widerruf liegen. Ist das Tor zu: `Err("nur_notizen: ...")`, nichts wird geschrieben.
/// Ausserhalb von Modellarbeit (Korrektur von Hand, Untertitel-Import, Zuschnitt) geschieht nichts.
pub(crate) fn schreibschutz(store: &MeetingStore, meeting_id: &str) -> Res<Option<TorSperre>> {
    if !in_modellarbeit(meeting_id) {
        return Ok(None);
    }
    if unter_tor_sperre() {
        // Der Thread haelt die Sperre schon (Schritt unter `mit_tor_besprechung`): frisch pruefen.
        tor_paare(store, None, Some(meeting_id)).map_err(|e| e.code())?;
        return Ok(None);
    }
    let sperre = tor_sperre();
    tor_und_binden(store, None, Some(meeting_id)).map_err(|e| e.code())?;
    Ok(Some(sperre))
}

/// EIN Modellaufruf ueber Ton dieser Besprechung (Spracherkennung je Versuch und Block,
/// Sprachprobe je Fenster, Sprecher-Modell je Kanal, Stimmvergleich je Ausschnitt). Das Tor wird
/// VOR dem Aufruf unter der Sperre geprueft; die Sperre ist beim Aufruf selbst frei (ein Modell
/// darf nie unter ihr laufen, sonst stuende jeder Widerruf bis zum Ende der Erkennung). `call`
/// bekommt die [`Freigabe`] fuer genau eine Inferenz. Danach wird das Tor unter der Sperre erneut
/// geprueft, IMMER: auch wenn `call` mit einem Fehler oder einer Panik endete. Hat sich die Lage
/// inzwischen geaendert (Widerruf, Planungs-/Terminwechsel, neuer Kalenderstand), gewinnt die
/// Ablehnung: Ergebnis, Fehler oder Panik werden verworfen (`Err`, der Aufrufer speichert nichts,
/// wiederholt nicht und schreibt keine Luecke). Fehler: `nur_notizen: <grund>`; sonst gehen Fehler
/// und Panik des Aufrufs unveraendert durch.
///
/// Voraussetzung ist eine laufende [`Modellarbeit`] des Threads fuer die Besprechung, sonst
/// `nur_notizen: ohne_modellarbeit` ohne Aufruf.
///
/// Warum kein Zaehler je Planung: das erneute Lesen nach dem Aufruf ist die staerkere Pruefung
/// (sie sieht auch Aenderungen anderer Verbindungen und Prozesse).
pub fn modell_lauf<T>(
    store: &MeetingStore,
    meeting_id: &str,
    call: impl FnOnce(&Freigabe) -> Res<T>,
) -> Res<T> {
    modell_lauf_mit(store, meeting_id, call, |tor| tor.code())
}

/// [`modell_lauf`] fuer Schleifen mit Wiederholung (Spracherkennung je Block): der Fehler sagt als
/// TYP, ob das Tor nicht freigab ([`LaufFehler::Tor`]: Ablehnung oder nicht pruefbar, also
/// Schluss) oder der Aufruf scheiterte ([`LaufFehler::Aufruf`]: darf wiederholt werden). Wer
/// wiederholt, entscheidet damit nie am Text eines Fehlers.
pub fn modell_versuch<T>(
    store: &MeetingStore,
    meeting_id: &str,
    call: impl FnOnce(&Freigabe) -> Res<T>,
) -> Result<T, LaufFehler> {
    modell_lauf_mit(
        store,
        meeting_id,
        |frei| call(frei).map_err(LaufFehler::Aufruf),
        LaufFehler::Tor,
    )
}

/// [`modell_lauf`] fuer Aufrufe mit eigenem Fehlertyp: `gesperrt` macht aus dem Fehler des Tors
/// ([`TorFehler`]: Ablehnung oder nicht pruefbar) den Fehler des Aufrufers.
pub fn modell_lauf_mit<T, E>(
    store: &MeetingStore,
    meeting_id: &str,
    call: impl FnOnce(&Freigabe) -> Result<T, E>,
    gesperrt: impl Fn(TorFehler) -> E,
) -> Result<T, E> {
    ohne_tor_sperre("Modellaufruf");
    if !in_modellarbeit(meeting_id) {
        return Err(gesperrt(TorFehler::Abgelehnt("ohne_modellarbeit")));
    }
    tor_pruefen_typ(store, meeting_id).map_err(&gesperrt)?;
    let frei = Freigabe::fuer_einen_aufruf();
    let ergebnis = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&frei)));
    // Die Nachpruefung laeuft IMMER, auch nach Fehler und Panik des Aufrufs.
    let danach = tor_pruefen_typ(store, meeting_id);
    match (ergebnis, danach) {
        (_, Err(tor)) => Err(gesperrt(tor)),
        (Ok(ergebnis), Ok(())) => ergebnis,
        (Err(panik), Ok(())) => std::panic::resume_unwind(panik),
    }
}

/// Ist `fehler` der Code des Tors (`nur_notizen: ...`)? Jeder Fehler des Tors traegt ihn
/// ([`TorFehler::code`]), auch ein Datenbankfehler beim Pruefen: an den Grenzen, an denen Fehler
/// nur noch Codes sind (Befehle, Ereignisse, Fehler des Speichers), ist das die Erkennung.
pub fn ist_ablehnung(fehler: &str) -> bool {
    fehler.starts_with("nur_notizen")
}

/// Das Einwilligungs-Tor: laeuft UNMITTELBAR vor dem Start einer Aufnahme mit
/// Live-Transkription (und vor jeder nachtraeglichen Transkription) und liest dabei alles frisch
/// aus der Datenbank: Einwilligungen, Termin und Teilnehmende (ein Widerruf zwischen einer
/// Abfrage per [`start_modus`] und dem Start wirkt damit). Allein ohne Sperre ist das nur eine
/// Momentaufnahme (und bindet nichts); atomar mit dem Start ist es nur ueber [`mit_tor`]. Gehoert
/// die Besprechung zu keiner Planung, passiert nichts. Sonst muss JEDE zugehoerige Planung gegen
/// JEDE Anwesendenliste [`StartModus::MitTranskription`] ergeben (Bindungen und [`start_bezug`]);
/// bei `NurNotizen` startet nichts (kein Ton, keine Transkription) und der Fehler ist
/// `nur_notizen: <grund>` (`nicht_geplant`, `abgelehnt` oder `offen`; nie Namen). Personen ohne
/// Identitaet und fehlende Anwesende sperren.
pub fn start_tor(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<()> {
    tor_paare(store, event, target_meeting_id)
        .map(|_| ())
        .map_err(|e| e.code())
}

/// Die Pruefung von [`start_tor`]; gibt die bestandenen Paare (Planung, Anwesende) zurueck.
/// Jeder Fehler beim Lesen (Datenbank, fehlende Planung) sperrt: [`TorFehler::NichtPruefbar`].
fn tor_paare(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Result<Vec<Paar>, TorFehler> {
    #[cfg(test)]
    {
        let gestoert = TOR_STOERUNG.with(|c| {
            let n = c.get();
            c.set(n.saturating_sub(1));
            n > 0
        });
        if gestoert {
            return Err(TorFehler::nicht_pruefbar("sql: injiziert (Test)".into()));
        }
    }
    let gelesen = tor_lesen(store, event, target_meeting_id).map_err(TorFehler::nicht_pruefbar)?;
    match gelesen.grund {
        Some(grund) => Err(TorFehler::Abgelehnt(grund)),
        None => Ok(gelesen.paare),
    }
}

/// Was [`tor_lesen`] in EINER Lesetransaktion gefunden hat.
struct TorLesung {
    paare: Vec<Paar>,
    /// Der Grund der ersten Planung, die nicht `MitTranskription` ergibt.
    grund: Option<&'static str>,
}

/// Liest alles, was das Tor braucht, frisch aus der Datenbank. `Err` ist hier IMMER ein Fehler
/// beim Lesen, nie die Entscheidung der Regel (die steht in [`TorLesung::grund`]).
fn tor_lesen(
    store: &MeetingStore,
    event: Option<&crate::managers::calendar::model::CalEvent>,
    target_meeting_id: Option<&str>,
) -> Res<TorLesung> {
    let mut conn = verbindung(store)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(sql)?;
    let paare = alle_paare(&tx, store, event, target_meeting_id)?;
    let mut grund = None;
    for (plan_id, anwesende) in &paare {
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
                grund = Some(if nicht_geplant {
                    "nicht_geplant"
                } else if !abgelehnt.is_empty() {
                    "abgelehnt"
                } else {
                    "offen"
                });
                break;
            }
        }
    }
    drop(tx);
    Ok(TorLesung { paare, grund })
}
