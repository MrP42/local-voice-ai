//! Ausloeser „Besprechung fertig“ (B2): Aufnahme beendet, Transkript fertig, KI-Notizen
//! fertig, Protokoll fertig.
//!
//! Die Ausloeser hoeren auf die VORHANDENEN Ereignisse (R6: keine zweite Erkennung, keine
//! Doppelung der eingebauten Automatik `meeting_auto_enhance`):
//!
//! | Stufe        | Ereignis                                              |
//! |--------------|-------------------------------------------------------|
//! | `recording`  | `MeetingEvent::State { status: "processing" }`        |
//! | `transcript` | `MeetingEvent::TranscriptFinal`                       |
//! | `notes`      | `MeetingNotesEvent::Done`                             |
//! | `minutes`    | `MinutesEvent::Done`                                  |
//!
//! Das Anhaengen macht `hub` (es braucht den `AppHandle`); die Entscheidung steht hier und
//! ist rein. **Genau einmal je Besprechung und Stufe**: der Schluessel ist
//! `meeting:<id>:<stufe>`. Ein erneutes Erzeugen der Notizen oder des Protokolls, eine
//! Neu-Transkription oder ein doppelt gesendetes Ereignis ergibt keinen zweiten Lauf; wer
//! einen Ablauf fuer dieselbe Besprechung noch einmal will, startet ihn von Hand. Das
//! verhindert auch Schleifen: ein Ablauf „Protokoll fertig“, dessen Schritte das Protokoll neu
//! erzeugen, loest sich nicht selbst wieder aus.
//!
//! Bei `recording` zaehlt nur eine Live-Aufnahme (`source = live`): ein Import hat keine
//! „Aufnahme“, sein Transkript meldet die Stufe `transcript`.

use serde_json::json;

use super::{enabled_with, fire, RunSink, TickReport};

pub const KIND: &str = "meeting.finished";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Recording,
    Transcript,
    Notes,
    Minutes,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Recording => "recording",
            Stage::Transcript => "transcript",
            Stage::Notes => "notes",
            Stage::Minutes => "minutes",
        }
    }
}

/// Was die Ausloeserdaten ueber die Besprechung brauchen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingInfo {
    pub id: String,
    pub title: String,
}

/// Die Daten des Ausloesers (`trigger.*`).
pub fn trigger_data(stage: Stage, m: &MeetingInfo) -> serde_json::Value {
    let title: String = m.title.chars().take(500).collect();
    json!({
        "meeting_id": m.id,
        "title": title,
        "stage": stage.as_str(),
        "meeting": {"id": m.id, "title": title},
    })
}

pub fn key_for(stage: Stage, meeting_id: &str) -> String {
    format!("meeting:{meeting_id}:{}", stage.as_str())
}

/// Ein Ereignis: reiht fuer jeden eingeschalteten Ablauf mit passender Stufe einen Lauf ein.
pub fn on_event(sink: &dyn RunSink, stage: Stage, meeting: &MeetingInfo) -> TickReport {
    let mut report = TickReport::default();
    let armed = match enabled_with(sink, &[KIND]) {
        Ok(a) => a,
        Err(e) => {
            report.errors.push(format!("Ablaeufe: {e}"));
            return report;
        }
    };
    for a in armed {
        let wanted = a.def.trigger.params.get("stage").and_then(|v| v.as_str());
        if wanted != Some(stage.as_str()) {
            continue;
        }
        fire(
            sink,
            &mut report,
            &a.row.id,
            key_for(stage, &meeting.id),
            trigger_data(stage, meeting),
        );
    }
    report
}

#[cfg(test)]
mod tests;
