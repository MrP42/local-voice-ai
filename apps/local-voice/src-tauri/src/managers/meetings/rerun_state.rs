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
//! Fortsetzen und das Oeffnen der Fassungsliste rufen dieselbe Funktion. Neue
//! Abschnitte werden nur behalten, wenn Phase `running` UND die Epoche der Arbeitskopie
//! passt; sonst wird zurueckgenommen. Scheitert ein Schritt, bleibt der Zustand
//! unveraendert und laesst sich erneut abschliessen (nie halb, nie verworfen).

use serde_json::{json, Value};

use super::store::{MeetingStatus, MeetingStore};
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
        })
    }

    pub fn to_value(&self) -> Value {
        json!({
            "phase": self.phase,
            "model": self.model,
            "kind": self.kind,
            "epoch": self.epoch,
            "variant_id": self.variant_id,
            "prev_status": self.prev_status,
            "prev": self.prev,
        })
    }
}

pub fn read(store: &MeetingStore, meeting_id: &str) -> Result<Option<Marker>, String> {
    let meta = store
        .metadata_json(meeting_id)
        .map_err(|e| format!("partial_marker_read_failed: {e}"))?;
    Ok(meta
        .as_ref()
        .and_then(|m| m.get(KEY))
        .and_then(Marker::from_value))
}

/// Die rohe Marke (fuer `prev`).
pub fn read_raw(store: &MeetingStore, meeting_id: &str) -> Result<Value, String> {
    let meta = store
        .metadata_json(meeting_id)
        .map_err(|e| format!("partial_marker_read_failed: {e}"))?;
    Ok(meta
        .and_then(|m| m.get(KEY).cloned())
        .unwrap_or(Value::Null))
}

pub fn write(store: &MeetingStore, meeting_id: &str, value: Value) -> Result<(), String> {
    store
        .set_metadata_key(meeting_id, KEY, value)
        .map_err(|e| format!("partial_marker_failed: {e}"))
}

pub fn write_marker(store: &MeetingStore, meeting_id: &str, m: &Marker) -> Result<(), String> {
    write(store, meeting_id, m.to_value())
}

/// Laufbeginn (unter dem Auftrag, VOR `begin_rerun`): offenen Vorlauf abschliessen,
/// dann Marke `start` mit dem Stand davor.
pub fn mark_start(store: &MeetingStore, meeting_id: &str, prev_status: &str) -> Result<(), String> {
    settle(store, meeting_id, false)?;
    let prev = read_raw(store, meeting_id)?;
    write_marker(
        store,
        meeting_id,
        &Marker {
            phase: PHASE_START.to_string(),
            prev_status: Some(prev_status.to_string()),
            prev,
            ..Default::default()
        },
    )
}

/// Nach `clear_segments`: die Arbeitskopie (diese Epoche) gehoert jetzt dem Lauf.
pub fn mark_running(
    store: &MeetingStore,
    meeting_id: &str,
    model: &str,
    kind: &'static str,
) -> Result<(), String> {
    let epoch = store
        .segment_epoch(meeting_id)
        .map_err(|e| format!("segment_epoch_failed: {e}"))?;
    let mut marker = read(store, meeting_id)?.unwrap_or_default();
    marker.phase = PHASE_RUNNING.to_string();
    marker.epoch = Some(epoch);
    marker.model = Some(model.to_string());
    marker.kind = kind;
    write_marker(store, meeting_id, &marker)
}

/// Ein offenes Teilergebnis, das „Fortsetzen“ ergaenzen kann.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resumable {
    pub model: String,
    pub variant_id: String,
}

/// Wie [`settle`] ausging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// Keine Marke und kein offener Lauf.
    Nothing,
    /// Das Teilergebnis ist (jetzt oder schon) Fassung; mit Modell fortsetzbar.
    Kept(Option<Resumable>),
    /// Zurueckgenommen: alte Fassung aktiv, Marke auf dem Stand vor dem Lauf;
    /// der Status vor dem Lauf (falls bekannt).
    RolledBack(Option<MeetingStatus>),
    /// Ein abgeschlossener Lauf hatte nur die Marke noch nicht geloescht.
    Finished,
}

pub fn status_from_name(name: &str) -> Option<MeetingStatus> {
    match name {
        "ready" => Some(MeetingStatus::Ready),
        "failed" => Some(MeetingStatus::Failed),
        "cancelled" => Some(MeetingStatus::Cancelled),
        _ => None,
    }
}

fn rerun_pending(store: &MeetingStore, meeting_id: &str) -> Result<bool, String> {
    let conn = store.get_connection().map_err(|e| e.to_string())?;
    variants::rerun_pending(&conn, meeting_id).map_err(|e| e.code().to_string())
}

fn roll_back(
    store: &MeetingStore,
    meeting_id: &str,
    marker: Option<&Marker>,
    pending: bool,
) -> Result<Settled, String> {
    if pending {
        // Absicht dauerhaft festhalten, BEVOR zurueckgenommen wird: ein Absturz
        // mitten darin endet beim naechsten Abschluss wieder hier.
        if let Some(m) = marker {
            if m.phase != PHASE_ROLLBACK {
                let mut r = m.clone();
                r.phase = PHASE_ROLLBACK.to_string();
                write_marker(store, meeting_id, &r)?;
            }
        }
        let mut conn = store.get_connection().map_err(|e| e.to_string())?;
        variants::abort_rerun(&mut conn, meeting_id).map_err(|e| e.code().to_string())?;
    }
    let prev_status = marker
        .and_then(|m| m.prev_status.as_deref())
        .and_then(status_from_name);
    if let Some(m) = marker {
        write(store, meeting_id, m.prev.clone())?;
    }
    Ok(Settled::RolledBack(prev_status))
}

/// Fuehrt einen offenen Neu-Lauf in einen Endzustand. Nur rufen, wenn KEIN Auftrag
/// fuer die Besprechung laeuft (oder der eigene Auftrag ruft). `rollback`: der Lauf
/// ist gescheitert, nichts behalten.
pub fn settle(store: &MeetingStore, meeting_id: &str, rollback: bool) -> Result<Settled, String> {
    let marker = read(store, meeting_id)?;
    let pending = rerun_pending(store, meeting_id)?;
    let Some(m) = marker else {
        if pending {
            // Ein Lauf ohne Marke (Stand vor lva#523): nur Zuruecknehmen ist sicher.
            return roll_back(store, meeting_id, None, true);
        }
        return Ok(Settled::Nothing);
    };
    if !pending {
        return match m.phase.as_str() {
            PHASE_KEPT => Ok(Settled::Kept(
                m.model
                    .clone()
                    .zip(m.variant_id.clone())
                    .map(|(model, variant_id)| Resumable { model, variant_id }),
            )),
            // Abgeschlossen (finish_rerun lief), nur die Marke stand noch.
            PHASE_RUNNING if !rollback => {
                write(store, meeting_id, Value::Null)?;
                Ok(Settled::Finished)
            }
            _ => roll_back(store, meeting_id, Some(&m), false),
        };
    }
    let epoch = store
        .segment_epoch(meeting_id)
        .map_err(|e| format!("segment_epoch_failed: {e}"))?;
    let own_copy = m.phase == PHASE_RUNNING && m.epoch == Some(epoch);
    if rollback || !own_copy {
        return roll_back(store, meeting_id, Some(&m), true);
    }
    let kept = {
        let mut conn = store.get_connection().map_err(|e| e.to_string())?;
        variants::keep_partial_rerun(&mut conn, meeting_id, m.kind, m.model.as_deref())
            .map_err(|e| e.code().to_string())?
    };
    match kept {
        Some(variant_id) => {
            let mut k = m.clone();
            k.phase = PHASE_KEPT.to_string();
            k.variant_id = Some(variant_id);
            k.prev = Value::Null;
            write_marker(store, meeting_id, &k)?;
            Ok(Settled::Kept(k.model.zip(k.variant_id).map(
                |(model, variant_id)| Resumable { model, variant_id },
            )))
        }
        // Kein neuer Abschnitt: `keep_partial_rerun` hat schon zurueckgenommen.
        None => roll_back(store, meeting_id, Some(&m), false),
    }
}

/// „Fortsetzen“ und Wiederherstellung: das offene Teilergebnis, mit seiner Fassung
/// als aktiver (der Nutzer kann nach dem Stopp umgeschaltet haben). Ohne Marke `None`.
pub fn prepare_resume(store: &MeetingStore, meeting_id: &str) -> Result<Option<String>, String> {
    let Settled::Kept(Some(r)) = settle(store, meeting_id, false)? else {
        return Ok(None);
    };
    let mut conn = store.get_connection().map_err(|e| e.to_string())?;
    let active = variants::active_id(&conn, meeting_id).map_err(|e| e.code().to_string())?;
    if active.as_deref() != Some(r.variant_id.as_str()) {
        variants::activate(&mut conn, &r.variant_id)
            .map_err(|e| format!("partial_variant_missing: {}", e.code()))?;
    }
    Ok(Some(r.model))
}

/// Nach vollstaendigem Nachholen: das Teilergebnis ist fertig.
pub fn clear(store: &MeetingStore, meeting_id: &str) -> Result<(), String> {
    write(store, meeting_id, Value::Null)
}

#[cfg(test)]
mod tests;
