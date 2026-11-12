//! lva#523: Zustand einer Neu-Transkription ueber Stopp, Fehler und Absturz hinweg.
//!
//! Der Neu-Lauf schreibt live in `transcripts` (Arbeitskopie, `variants::begin_rerun`).
//! Damit ein Stopp das neu Transkribierte NICHT verwirft und ein Absturz nie Altbestand
//! fuer neue Abschnitte haelt, fuehrt der Lauf eine Marke in `metadata_json`
//! (`partial_rerun`) mit einer Phase:
//!
//! | Phase      | gesetzt                         | bedeutet                                   |
//! |------------|---------------------------------|--------------------------------------------|
//! | `start`    | vor `begin_rerun`               | Arbeitskopie gehoert noch keinem Lauf      |
//! | `running`  | nach `clear_segments` (+Epoche) | Arbeitskopie = neue Abschnitte DIESES Laufs|
//! | `rollback` | vor dem Zuruecknehmen (Fehler)  | alte Fassung wiederherstellen              |
//! | `kept`     | nach `keep_partial_rerun`       | Teilergebnis ist Fassung `variant_id`      |
//!
//! `prev` und `prev_status` halten den Stand vor dem Lauf. [`settle`] fuehrt jeden
//! Zwischenstand in einen Endzustand: Stopp, Fehler, Wiederherstellung nach Absturz,
//! Fortsetzen und das Oeffnen der Fassungsliste rufen dieselbe Funktion, immer unter
//! dem Auftrag der Besprechung (`job::try_start`). Neue Abschnitte werden nur
//! behalten, wenn Phase `running`, die Epoche der Arbeitskopie passt und die
//! Besprechung nicht `failed` ist; sonst wird zurueckgenommen.
//!
//! **Atomar:** Fassungsaenderung (behalten, zuruecknehmen, abschliessen), Marke und
//! Besprechungsstatus stehen in EINER SQLite-Transaktion (`IMMEDIATE`). Es gibt keinen
//! sichtbaren Zwischenstand zwischen „Fassung gesichert“ und „Marke/Status
//! geschrieben“; scheitert die Transaktion, bleibt alles wie vorher und laesst sich
//! erneut abschliessen. `running` ohne offenen Lauf kann es daher nicht geben und wird
//! als Fehler gemeldet, nie als „fertig“ gedeutet.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

use super::store::{MeetingStatus, MeetingStore};
use super::trim::TimeRange;
use super::variants;

/// Schluessel in `metadata_json`.
pub const KEY: &str = "partial_rerun";

pub const PHASE_START: &str = "start";
pub const PHASE_RUNNING: &str = "running";
pub const PHASE_ROLLBACK: &str = "rollback";
pub const PHASE_KEPT: &str = "kept";

/// Die Marke, wie sie gespeichert ist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marker {
    pub phase: String,
    pub model: Option<String>,
    pub kind: &'static str,
    pub epoch: Option<u32>,
    pub variant_id: Option<String>,
    pub prev_status: Option<String>,
    /// Die Marke vor diesem Lauf (`Null`: keine).
    pub prev: Value,
    /// lva#522: Bereich der Aufnahme (ms, absolut), wenn der Lauf nur einen Bereich
    /// transkribiert. Fehlt in Marken ohne Bereich und aus aelterem Bestand.
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

impl Marker {
    fn from_value(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let s = |k: &str| {
            obj.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
        };
        // Ohne Phase (Stand 02c1f87): eine behaltene Marke OHNE Fassung, also nicht
        // zuordenbar (`prepare_resume` weist sie ab).
        let phase = s("phase").unwrap_or_else(|| PHASE_KEPT.to_string());
        let kind = match s("kind").as_deref() {
            Some(k) if k == variants::KIND_OWN => variants::KIND_OWN,
            _ => variants::KIND_RETRANSCRIBED,
        };
        Some(Self {
            phase,
            model: s("model"),
            kind,
            epoch: obj
                .get("epoch")
                .and_then(Value::as_u64)
                .and_then(|e| u32::try_from(e).ok()),
            variant_id: s("variant_id"),
            prev_status: s("prev_status"),
            prev: obj.get("prev").cloned().unwrap_or(Value::Null),
            start_ms: obj.get("start_ms").and_then(Value::as_u64),
            end_ms: obj.get("end_ms").and_then(Value::as_u64),
        })
    }

    pub fn to_value(&self) -> Value {
        let mut value = json!({
            "phase": self.phase,
            "model": self.model,
            "kind": self.kind,
            "epoch": self.epoch,
            "variant_id": self.variant_id,
            "prev_status": self.prev_status,
            "prev": self.prev,
        });
        // Nur mit Bereich (Marken ohne Bereich bleiben, wie sie waren).
        if let (Some(map), Some(start), Some(end)) =
            (value.as_object_mut(), self.start_ms, self.end_ms)
        {
            map.insert("start_ms".into(), json!(start));
            map.insert("end_ms".into(), json!(end));
        }
        value
    }

    /// Der Bereich dieses Laufs; `None` ohne Bereich oder bei einem unstimmigen.
    pub fn range(&self) -> Option<TimeRange> {
        let range = TimeRange {
            start_ms: self.start_ms?,
            end_ms: self.end_ms?,
        };
        range.validate().ok()
    }

    fn set_range(&mut self, range: Option<TimeRange>) {
        self.start_ms = range.map(|r| r.start_ms);
        self.end_ms = range.map(|r| r.end_ms);
    }
}

// ---------------------------------------------------------------------------
// Lesen und Schreiben IN einer Transaktion
// ---------------------------------------------------------------------------

fn metadata_tx(
    conn: &Connection,
    meeting_id: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let current: Option<Option<String>> = conn
        .query_row(
            "SELECT metadata_json FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(current) = current else {
        return Err("meeting_not_found".to_string());
    };
    match current.as_deref().map(str::trim) {
        None | Some("") => Ok(serde_json::Map::new()),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err("metadata_json_invalid".to_string()),
        },
    }
}

fn read_raw_tx(conn: &Connection, meeting_id: &str) -> Result<Value, String> {
    Ok(metadata_tx(conn, meeting_id)?
        .get(KEY)
        .cloned()
        .unwrap_or(Value::Null))
}

fn read_tx(conn: &Connection, meeting_id: &str) -> Result<Option<Marker>, String> {
    Ok(Marker::from_value(&read_raw_tx(conn, meeting_id)?))
}

fn write_tx(conn: &Connection, meeting_id: &str, value: Value) -> Result<(), String> {
    let mut map = metadata_tx(conn, meeting_id)?;
    map.insert(KEY.to_string(), value);
    conn.execute(
        "UPDATE meetings SET metadata_json = ?1, updated_at = ?2 WHERE id = ?3",
        params![Value::Object(map).to_string(), now_s(), meeting_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn status_tx(conn: &Connection, meeting_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT status FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
        params![meeting_id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

fn set_status_tx(conn: &Connection, meeting_id: &str, status: MeetingStatus) -> Result<(), String> {
    conn.execute(
        "UPDATE meetings SET status = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        params![status.as_str(), now_s(), meeting_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn epoch_tx(conn: &Connection, meeting_id: &str) -> Result<u32, String> {
    let epoch: Option<i64> = conn
        .query_row(
            "SELECT segment_epoch FROM transcripts WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(u32::try_from(epoch.unwrap_or(0)).unwrap_or(0))
}

fn now_s() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
type CommitProbe = Box<dyn Fn(&Connection) -> bool>;

#[cfg(test)]
thread_local! {
    /// Tests: laesst den Commit jeder Transaktion scheitern, deren Endstand dieses
    /// Praedikat erfuellt (gelesen in der Transaktion, kurz vor `commit()`).
    pub(crate) static FAIL_COMMIT_IF: std::cell::RefCell<Option<CommitProbe>> =
        std::cell::RefCell::new(None);
}

fn in_tx<T>(
    store: &MeetingStore,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let out = f(&tx)?;
    #[cfg(test)]
    if FAIL_COMMIT_IF.with(|p| p.borrow().as_ref().is_some_and(|p| p(&tx))) {
        // Wie ein gescheitertes `commit()`: nichts gespeichert, Fehler zurueck.
        return Err("injected_commit_failure".to_string());
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Lesen ohne Aenderung (Anzeige, Tests).
#[cfg(test)]
pub fn read(store: &MeetingStore, meeting_id: &str) -> Result<Option<Marker>, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    read_tx(&conn, meeting_id)
}

/// Rohes Schreiben der Marke (nur Tests und Altbestand).
#[cfg(test)]
pub fn write(store: &MeetingStore, meeting_id: &str, value: Value) -> Result<(), String> {
    in_tx(store, |tx| write_tx(tx, meeting_id, value))
}

pub fn status_from_name(name: &str) -> Option<MeetingStatus> {
    match name {
        "ready" => Some(MeetingStatus::Ready),
        "failed" => Some(MeetingStatus::Failed),
        "cancelled" => Some(MeetingStatus::Cancelled),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Uebergaenge
// ---------------------------------------------------------------------------

/// Ein offenes Teilergebnis, das „Fortsetzen“ ergaenzen kann.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resumable {
    pub model: String,
    pub variant_id: String,
    /// lva#522: Bereich des Laufs (absolute ms), falls er nur einen transkribierte.
    pub range: Option<TimeRange>,
}

/// Wie [`settle`] ausging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// Keine Marke und kein offener Lauf.
    Nothing,
    /// Das Teilergebnis ist (jetzt oder schon) Fassung. `None`: nicht zuordenbar
    /// (Altbestand ohne Fassung) oder bewusst nicht fortsetzbar.
    Kept(Option<Resumable>),
    /// Zurueckgenommen: alte Fassung aktiv, Marke auf dem Stand vor dem Lauf, Status
    /// (falls bekannt) geschrieben, alles in einer Transaktion.
    RolledBack(Option<MeetingStatus>),
}

/// Was [`settle`] im Endzustand schreibt.
#[derive(Clone, Copy, Debug)]
pub struct Ends {
    /// Status, wenn das Teilergebnis JETZT behalten wird (`None`: unveraendert).
    pub kept: Option<MeetingStatus>,
    /// Behalten mit Fortsetzen-Marke (`false`: YouTube, nur die Fassung bleibt).
    pub resumable: bool,
    /// Status beim Zuruecknehmen, wenn der Vorstatus unbekannt ist.
    pub fallback: Option<MeetingStatus>,
}

impl Ends {
    /// Nichts am Status aendern (Fassungsdialog, Laufbeginn).
    pub const QUIET: Ends = Ends {
        kept: None,
        resumable: true,
        fallback: None,
    };
}

fn resumable_of(m: &Marker) -> Option<Resumable> {
    m.model
        .clone()
        .zip(m.variant_id.clone())
        .map(|(model, variant_id)| Resumable {
            model,
            variant_id,
            range: m.range(),
        })
}

fn roll_back_tx(
    tx: &Transaction<'_>,
    meeting_id: &str,
    marker: Option<&Marker>,
    pending: bool,
    fallback: Option<MeetingStatus>,
) -> Result<Settled, String> {
    if pending {
        variants::restore_in_tx(tx, meeting_id).map_err(|e| e.code().to_string())?;
    }
    let status = marker
        .and_then(|m| m.prev_status.as_deref())
        .and_then(status_from_name)
        .or(fallback);
    if let Some(m) = marker {
        write_tx(tx, meeting_id, m.prev.clone())?;
    }
    if let Some(status) = status {
        set_status_tx(tx, meeting_id, status)?;
    }
    Ok(Settled::RolledBack(status))
}

/// Fuehrt einen offenen Neu-Lauf in einen Endzustand, atomar (siehe Moduldoku).
/// Nur unter dem Auftrag der Besprechung rufen. `rollback`: der Lauf ist
/// gescheitert, nichts behalten. Scheitert es, ist nichts veraendert.
pub fn settle(
    store: &MeetingStore,
    meeting_id: &str,
    rollback: bool,
    ends: Ends,
) -> Result<Settled, String> {
    in_tx(store, |tx| {
        let marker = read_tx(tx, meeting_id)?;
        let pending = variants::rerun_pending(tx, meeting_id).map_err(|e| e.code().to_string())?;
        let Some(m) = marker else {
            return if pending {
                // Ein Lauf ohne Marke (Stand vor lva#523): nur Zuruecknehmen ist sicher.
                roll_back_tx(tx, meeting_id, None, true, ends.fallback)
            } else {
                Ok(Settled::Nothing)
            };
        };
        if !pending {
            return match m.phase.as_str() {
                PHASE_KEPT if ends.resumable => Ok(Settled::Kept(resumable_of(&m))),
                // Nicht fortsetzbar (keine Spur): Marke weg und Endstatus in DIESER
                // Transaktion, das Teilergebnis bleibt.
                PHASE_KEPT => {
                    write_tx(tx, meeting_id, Value::Null)?;
                    if let Some(status) = ends.kept {
                        set_status_tx(tx, meeting_id, status)?;
                    }
                    Ok(Settled::Kept(None))
                }
                PHASE_START | PHASE_ROLLBACK => {
                    roll_back_tx(tx, meeting_id, Some(&m), false, ends.fallback)
                }
                // `running` endet nur atomar (behalten, zuruecknehmen, abschliessen):
                // ohne offenen Lauf ist das ein Widerspruch, nie „fertig“.
                _ => Err("rerun_state_inconsistent".to_string()),
            };
        }
        let failed = status_tx(tx, meeting_id)? == "failed";
        let own_copy = m.phase == PHASE_RUNNING && m.epoch == Some(epoch_tx(tx, meeting_id)?);
        if rollback || failed || !own_copy {
            return roll_back_tx(tx, meeting_id, Some(&m), true, ends.fallback);
        }
        match variants::keep_partial_in_tx(tx, meeting_id, m.kind, m.model.as_deref())
            .map_err(|e| e.code().to_string())?
        {
            Some(variant_id) => {
                let mut k = m.clone();
                k.phase = PHASE_KEPT.to_string();
                k.variant_id = Some(variant_id);
                k.prev = Value::Null;
                write_tx(
                    tx,
                    meeting_id,
                    if ends.resumable {
                        k.to_value()
                    } else {
                        Value::Null
                    },
                )?;
                if let Some(status) = ends.kept {
                    set_status_tx(tx, meeting_id, status)?;
                }
                Ok(Settled::Kept(
                    ends.resumable.then(|| resumable_of(&k)).flatten(),
                ))
            }
            // Kein neuer Abschnitt: in derselben Transaktion schon zurueckgenommen.
            None => roll_back_tx(tx, meeting_id, Some(&m), false, ends.fallback),
        }
    })
}

/// Status, den ein zurueckgenommener Lauf wiederherstellt: `failed`/`cancelled`
/// bleiben, alles andere wird `ready`.
pub fn restorable_status(name: &str) -> MeetingStatus {
    match name {
        "failed" => MeetingStatus::Failed,
        "cancelled" => MeetingStatus::Cancelled,
        _ => MeetingStatus::Ready,
    }
}

/// Laufbeginn (unter dem Auftrag, VOR `begin_rerun`): offenen Vorlauf abschliessen,
/// dann Marke `start` mit dem Stand davor. Der Vorstatus kommt aus dem Zustand NACH
/// diesem Abschluss (nie aus einem vorher gelesenen Wert) und wird zurueckgegeben.
pub fn mark_start(
    store: &MeetingStore,
    meeting_id: &str,
    range: Option<TimeRange>,
) -> Result<MeetingStatus, String> {
    settle(store, meeting_id, false, Ends::QUIET)?;
    in_tx(store, |tx| {
        let prev_status = restorable_status(&status_tx(tx, meeting_id)?);
        let prev = read_raw_tx(tx, meeting_id)?;
        let mut marker = Marker {
            phase: PHASE_START.to_string(),
            prev_status: Some(prev_status.as_str().to_string()),
            prev,
            ..Default::default()
        };
        marker.set_range(range);
        write_tx(tx, meeting_id, marker.to_value())?;
        Ok(prev_status)
    })
}

/// Nach `clear_segments`: die Arbeitskopie (diese Epoche) gehoert jetzt dem Lauf.
pub fn mark_running(
    store: &MeetingStore,
    meeting_id: &str,
    model: &str,
    kind: &'static str,
    range: Option<TimeRange>,
) -> Result<(), String> {
    in_tx(store, |tx| {
        let mut marker = read_tx(tx, meeting_id)?.unwrap_or_default();
        marker.set_range(range);
        marker.phase = PHASE_RUNNING.to_string();
        marker.epoch = Some(epoch_tx(tx, meeting_id)?);
        marker.model = Some(model.to_string());
        marker.kind = kind;
        write_tx(tx, meeting_id, marker.to_value())
    })
}

/// Der Lauf ist fertig: Ergebnis wird aktive Fassung, Marke weg, Status `ready`, in
/// einer Transaktion. Gibt die Kennung der neuen Fassung zurueck.
pub fn complete(
    store: &MeetingStore,
    meeting_id: &str,
    kind: &'static str,
) -> Result<String, String> {
    in_tx(store, |tx| {
        let id = variants::finish_in_tx(tx, meeting_id, kind, None)
            .map_err(|e| format!("variants_finish_failed: {}", e.code()))?;
        write_tx(tx, meeting_id, Value::Null)?;
        set_status_tx(tx, meeting_id, MeetingStatus::Ready)?;
        Ok(id)
    })
}

/// Was vor dem Nachholen mit einem offenen Neu-Lauf geschieht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    /// Teilergebnis (wieder) aktiv: mit diesem Modell streng nachholen, bei einem
    /// Lauf mit Bereich nur in diesem (lva#522).
    Partial(String, Option<TimeRange>),
    /// Kein Neu-Lauf im Spiel: normal weiter.
    Normal,
    /// Zurueckgenommen, Status geschrieben: nichts weiter zu tun.
    Done(Option<MeetingStatus>),
}

/// „Fortsetzen“ und Wiederherstellung (unter dem Auftrag): offenen Lauf abschliessen
/// und die gestoppte Fassung wieder aktiv machen (der Nutzer kann umgeschaltet haben).
/// Eine nicht zuordenbare Marke wird abgewiesen, nie still die aktive Fassung ergaenzt.
pub fn prepare_resume(
    store: &MeetingStore,
    meeting_id: &str,
    has_track: bool,
) -> Result<Resume, String> {
    // Ohne gespeicherte Spur (YouTube) gibt es nichts nachzuholen: das Teilergebnis
    // endet ausdruecklich ohne Fortsetzbarkeit, nie als „nachgeholt“.
    let settled = settle(
        store,
        meeting_id,
        false,
        Ends {
            kept: (!has_track).then_some(MeetingStatus::Ready),
            resumable: has_track,
            fallback: Some(MeetingStatus::Ready),
        },
    )?;
    match settled {
        Settled::Nothing => Ok(Resume::Normal),
        Settled::RolledBack(status) => Ok(Resume::Done(status)),
        // Ohne Spur hat `settle` Marke und `ready` schon atomar geschrieben.
        Settled::Kept(None) if !has_track => Ok(Resume::Done(Some(MeetingStatus::Ready))),
        Settled::Kept(None) => Err("partial_rerun_unassigned".to_string()),
        Settled::Kept(Some(r)) => in_tx(store, |tx| {
            let active = variants::active_id(tx, meeting_id).map_err(|e| e.code().to_string())?;
            if active.as_deref() != Some(r.variant_id.as_str()) {
                variants::activate_for_resume_in_tx(tx, &r.variant_id)
                    .map_err(|e| format!("partial_variant_missing: {}", e.code()))?;
            }
            Ok(Resume::Partial(r.model, r.range))
        }),
    }
}

/// Fehlerpfad, wenn `settle(rollback)` selbst scheiterte: die Absicht festhalten.
pub fn mark_rollback(store: &MeetingStore, meeting_id: &str) -> Result<(), String> {
    in_tx(store, |tx| {
        let Some(mut m) = read_tx(tx, meeting_id)? else {
            return Ok(());
        };
        if m.phase == PHASE_KEPT {
            return Ok(());
        }
        m.phase = PHASE_ROLLBACK.to_string();
        write_tx(tx, meeting_id, m.to_value())
    })
}

/// Fehlerpfad eines Laufs (STT gescheitert): erst die Absicht dauerhaft speichern
/// (Marke `rollback`, sonst Status `failed`: beides laesst jeden spaeteren Abschluss
/// zuruecknehmen), DANN zuruecknehmen. Ist keins von beiden gespeichert, beginnt kein
/// Zuruecknehmen (`Err`): ein Absturz darin liesse sonst `running` + offen zurueck.
/// Gibt den Endstatus zurueck.
pub fn fail_run(
    store: &MeetingStore,
    meeting_id: &str,
    fallback: MeetingStatus,
) -> Result<MeetingStatus, String> {
    if let Err(m) = mark_rollback(store, meeting_id) {
        log::error!("meetings: rollback intent not stored ({meeting_id}): {m}");
        store
            .set_status(meeting_id, MeetingStatus::Failed)
            .map_err(|e| format!("rollback_intent_not_stored: {m}; {e}"))?;
    }
    let ends = Ends {
        kept: None,
        resumable: false,
        fallback: Some(fallback),
    };
    match settle(store, meeting_id, true, ends) {
        Ok(Settled::RolledBack(status)) => Ok(status.unwrap_or(fallback)),
        Ok(_) => {
            let _ = store.set_status(meeting_id, fallback);
            Ok(fallback)
        }
        // Absicht steht: der naechste Abschluss nimmt zurueck.
        Err(err) => {
            log::error!("meetings: rollback not finished ({meeting_id}): {err}");
            let _ = store.set_status(meeting_id, MeetingStatus::Failed);
            Ok(MeetingStatus::Failed)
        }
    }
}

/// Nach vollstaendigem strengen Nachholen: Marke weg und `ready`, in einer Transaktion.
pub fn finish_partial(store: &MeetingStore, meeting_id: &str) -> Result<(), String> {
    in_tx(store, |tx| {
        write_tx(tx, meeting_id, Value::Null)?;
        set_status_tx(tx, meeting_id, MeetingStatus::Ready)
    })
}

#[cfg(test)]
mod tests;
