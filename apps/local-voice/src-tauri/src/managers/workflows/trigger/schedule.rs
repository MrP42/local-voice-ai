//! Ausloeser „Zeitplan“ (B2): taeglich oder an bestimmten Wochentagen zu einer Uhrzeit,
//! Ortszeit des Rechners. Ein bewusst kleiner, Cron-artiger Plan, kein Cron-Ausdruck:
//!
//! ```json
//! {"type": "schedule", "every": "daily",  "at": "08:00"}
//! {"type": "schedule", "every": "weekly", "at": "17:30", "weekdays": ["mo", "mi", "fr"]}
//! ```
//!
//! `at` ist `HH:MM` (24 h). `weekdays` nur bei `weekly`: `mo di mi do fr sa so`, englisch
//! `mon tue wed thu fri sat sun` oder die Zahlen 1 (Montag) bis 7 (Sonntag).
//!
//! **Reine Entscheidung mit fester Uhr.** `latest_slot` bekommt Jetzt und die Zeitzone als
//! Parameter (Tests: feste Uhr und `FixedOffset`, die Anwendung: `Local`) und liefert den
//! juengsten Zeitpunkt des Plans, der hoechstens `GRACE_MS` zurueckliegt.
//!
//! **Genau einmal je Zeitpunkt.** Der Schluessel ist `schedule:<zeitpunkt-utc>`; mehrere
//! Takte innerhalb der Karenzzeit, ein Neustart und zwei Threads ergeben einen Lauf (Engine).
//! **Karenzzeit** `GRACE_MS` (15 min): wer die App erst kurz nach der Uhrzeit startet oder
//! aus dem Ruhezustand weckt, bekommt den Lauf nachgeholt; was laenger zurueckliegt,
//! verfaellt (kein Nachholen von Tagen). **Neue Ablaeufe** feuern nie rueckwirkend: ein
//! Zeitpunkt vor dem letzten Aendern, Einschalten oder Scharfschalten des Ablaufs
//! (`updated_at`) zaehlt nicht. **Sommerzeit**: eine Uhrzeit, die es am Umstellungstag nicht
//! gibt (02:30 beim Vorstellen), entfaellt an diesem Tag; eine doppelte (beim Zurueckstellen)
//! gilt beim ersten Mal.

use std::collections::BTreeSet;

use chrono::{Datelike, TimeZone};
use serde_json::{json, Map, Value};

use super::{enabled_with, fire, iso, RunSink, TickReport};

pub const KIND: &str = "schedule";
/// So lange nach dem Zeitpunkt wird ein Lauf noch nachgeholt.
pub const GRACE_MS: i64 = 15 * 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Daily,
    Weekly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub every: Every,
    pub hour: u32,
    pub minute: u32,
    /// 1 = Montag ... 7 = Sonntag; bei `Daily` leer.
    pub weekdays: BTreeSet<u32>,
}

fn parse_at(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

fn parse_weekday(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().filter(|n| (1..=7).contains(n)).map(|n| n as u32),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "mo" | "mon" | "montag" | "monday" | "1" => Some(1),
            "di" | "tue" | "dienstag" | "tuesday" | "2" => Some(2),
            "mi" | "wed" | "mittwoch" | "wednesday" | "3" => Some(3),
            "do" | "thu" | "donnerstag" | "thursday" | "4" => Some(4),
            "fr" | "fri" | "freitag" | "friday" | "5" => Some(5),
            "sa" | "sat" | "samstag" | "saturday" | "6" => Some(6),
            "so" | "sun" | "sonntag" | "sunday" | "7" => Some(7),
            _ => None,
        },
        _ => None,
    }
}

/// Befunde der Felder `at` und `weekdays` als (Feld, deutscher Satz). Leer = in Ordnung.
pub fn check(params: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    match params.get("at").and_then(Value::as_str).map(parse_at) {
        Some(Some(_)) => {}
        _ => out.push((
            "at",
            "Uhrzeit im Format HH:MM erwartet (24 Stunden, z. B. 08:30).".to_string(),
        )),
    }
    let weekly = params.get("every").and_then(Value::as_str) == Some("weekly");
    match params.get("weekdays") {
        None | Some(Value::Null) => {
            if weekly {
                out.push((
                    "weekdays",
                    "Für „weekly“ sind Wochentage nötig (z. B. [\"mo\", \"mi\"]).".to_string(),
                ));
            }
        }
        Some(_) if !weekly => out.push((
            "weekdays",
            "Wochentage gibt es nur bei „weekly“; bei „daily“ weglassen.".to_string(),
        )),
        Some(Value::Array(days)) if !days.is_empty() && days.len() <= 14 => {
            if days.iter().any(|d| parse_weekday(d).is_none()) {
                out.push((
                    "weekdays",
                    "Unbekannter Wochentag (erlaubt: mo di mi do fr sa so, mon … sun oder 1 bis 7)."
                        .to_string(),
                ));
            }
        }
        Some(_) => out.push((
            "weekdays",
            "Liste mit mindestens einem Wochentag erwartet.".to_string(),
        )),
    }
    out
}

/// Der Plan aus den Feldern des Ausloesers; `None`, wenn `check` etwas beanstandet haette.
pub fn plan_from(params: &Map<String, Value>) -> Option<Plan> {
    if !check(params).is_empty() {
        return None;
    }
    let (hour, minute) = parse_at(params.get("at")?.as_str()?)?;
    let every = match params.get("every")?.as_str()? {
        "daily" => Every::Daily,
        "weekly" => Every::Weekly,
        _ => return None,
    };
    let weekdays = match (every, params.get("weekdays")) {
        (Every::Weekly, Some(Value::Array(days))) => {
            days.iter().filter_map(parse_weekday).collect()
        }
        _ => BTreeSet::new(),
    };
    Some(Plan {
        every,
        hour,
        minute,
        weekdays,
    })
}

/// Der juengste Zeitpunkt des Plans, der nicht nach `now_ms` liegt und hoechstens
/// `GRACE_MS` zurueck (ms UTC), in der Zeitzone `tz`.
pub fn latest_slot<Tz: TimeZone>(plan: &Plan, now_ms: i64, tz: &Tz) -> Option<i64> {
    let now_local = tz.timestamp_millis_opt(now_ms).single()?;
    let today = now_local.date_naive();
    let mut best: Option<i64> = None;
    // Die Karenzzeit ist kuerzer als ein Tag: heute und gestern genuegen.
    for back in 0..=1i64 {
        let day = today - chrono::Duration::days(back);
        let weekday = day.weekday().number_from_monday();
        if plan.every == Every::Weekly && !plan.weekdays.contains(&weekday) {
            continue;
        }
        let Some(naive) = day.and_hms_opt(plan.hour, plan.minute, 0) else {
            continue;
        };
        // Doppelte Ortszeit (Zurueckstellen): die erste; nicht vorhandene (Vorstellen): keine.
        let Some(slot) = tz.from_local_datetime(&naive).earliest() else {
            continue;
        };
        let ms = slot.timestamp_millis();
        if ms <= now_ms && now_ms - ms <= GRACE_MS && best.is_none_or(|b| ms > b) {
            best = Some(ms);
        }
    }
    best
}

pub fn key_for(slot_ms: i64) -> String {
    format!("schedule:{}", iso(slot_ms))
}

/// Ein Takt: reiht fuer jeden eingeschalteten Ablauf mit Zeitplan den faelligen Zeitpunkt
/// ein (die Engine macht daraus hoechstens einen Lauf).
pub fn on_tick<Tz: TimeZone>(sink: &dyn RunSink, now_ms: i64, tz: &Tz) -> TickReport {
    let mut report = TickReport::default();
    let armed = match enabled_with(sink, &[KIND]) {
        Ok(a) => a,
        Err(e) => {
            report.errors.push(format!("Ablaeufe: {e}"));
            return report;
        }
    };
    for a in armed {
        let Some(plan) = plan_from(&a.def.trigger.params) else {
            continue;
        };
        let Some(slot) = latest_slot(&plan, now_ms, tz) else {
            continue;
        };
        if slot < a.row.updated_at {
            // Der Ablauf wurde erst nach diesem Zeitpunkt angelegt, geaendert, ein- oder
            // scharfgeschaltet: nicht rueckwirkend.
            continue;
        }
        fire(
            sink,
            &mut report,
            &a.row.id,
            key_for(slot),
            json!({"scheduled_for": iso(slot)}),
        );
    }
    report
}

#[cfg(test)]
mod tests;
