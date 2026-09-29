//! Headless-Werkzeug `--calendar-dump <datei|url> [--from D] [--to D] [--json]`
//! (M5 §5 „Headless“): liest einen Kalender wie der Sync-Dienst es tut (Abruf,
//! Parser, Serienexpansion) und gibt die Termine im Fenster aus, ohne die
//! Datenbank zu beruehren. Damit laesst sich eine echte Outlook- oder
//! Google-Adresse pruefen (auch: liefert die veroeffentlichte Adresse
//! Teilnehmende?), und die Fixture-Tabellen sind ohne Oberflaeche abnehmbar.
//!
//! - `--from`/`--to`: `YYYY-MM-DD` (0 Uhr UTC) oder RFC 3339. Vorgabe: 30 Tage
//!   vor bis 30 Tage nach jetzt (das Fenster des Sync-Dienstes).
//! - Standardzone: `LVA_CALENDAR_TZ`, sonst die Windows-Zone, sonst UTC.
//! - Die Ausgabe enthaelt NIE die Adresse: bei einer URL nur den Host, bei einer
//!   Datei den Pfad. Beschreibungstexte fehlen bewusst (koennen Zugangsdaten
//!   und Einwahlcodes tragen).
//! - Exit: 0 gelesen (auch mit Hinweisen), 1 Fehler beim Lesen/Abruf/Parsen,
//!   2 falscher Aufruf.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};

use super::fetch::{self, FetchOutcome};
use super::ics;

const DAY_MS: i64 = 86_400_000;

fn is_url(target: &str) -> bool {
    let l = target.trim().to_ascii_lowercase();
    ["http://", "https://", "webcal://", "webcals://"]
        .iter()
        .any(|p| l.starts_with(p))
}

fn parse_bound(text: &str) -> Result<i64, String> {
    let t = text.trim();
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        let midnight = d.and_hms_opt(0, 0, 0).ok_or("ungueltiges Datum")?;
        return Ok(Utc.from_utc_datetime(&midnight).timestamp_millis());
    }
    DateTime::parse_from_rfc3339(t)
        .map(|d| d.timestamp_millis())
        .map_err(|_| format!("Datum '{t}' ist weder JJJJ-MM-TT noch RFC 3339"))
}

fn iso(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn error_payload(code: i32, message: String) -> (i32, Value) {
    (code, json!({"mode": "calendar_dump", "error": message}))
}

/// Fuehrt den Aufruf aus. Liefert (Exit-Code, JSON); bei einem Fehler steht
/// unter `error` der Klartext. `now_ms` bestimmt nur die Vorgabe des Fensters.
pub fn run_cli(target: &str, from: Option<&str>, to: Option<&str>, now_ms: i64) -> (i32, Value) {
    let from_ms = match from.map(parse_bound).transpose() {
        Ok(v) => v.unwrap_or(now_ms - 30 * DAY_MS),
        Err(e) => return error_payload(2, format!("--from: {e}")),
    };
    let to_ms = match to.map(parse_bound).transpose() {
        Ok(v) => v.unwrap_or(now_ms + 30 * DAY_MS),
        Err(e) => return error_payload(2, format!("--to: {e}")),
    };
    if from_ms >= to_ms {
        return error_payload(2, "--from muss vor --to liegen".to_string());
    }
    if target.trim().is_empty() {
        return error_payload(
            2,
            "--calendar-dump braucht eine Datei oder Adresse".to_string(),
        );
    }

    let (raw, source) = if is_url(target) {
        let host = fetch::host_hint(target).unwrap_or_else(|| "?".to_string());
        match tauri::async_runtime::block_on(fetch::fetch_ics(target, None, None)) {
            Ok(FetchOutcome::Body { text, .. }) => (text, format!("url:{host}")),
            Ok(FetchOutcome::NotModified) => {
                return error_payload(1, "Server antwortet mit 304 ohne Anfrage".to_string())
            }
            Err(e) => return error_payload(1, e.to_string()),
        }
    } else {
        match std::fs::read(target.trim()) {
            Ok(bytes) => (
                String::from_utf8(bytes)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                format!("file:{}", target.trim()),
            ),
            Err(e) => return error_payload(1, format!("Datei nicht lesbar: {e}")),
        }
    };

    let tz = std::env::var("LVA_CALENDAR_TZ")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(ics::system_tz_name)
        .unwrap_or_else(|| "UTC".to_string());
    let result = match ics::parse_and_expand(&raw, "dump", from_ms, to_ms, &tz) {
        Ok(r) => r,
        Err(e) => return error_payload(1, e.to_string()),
    };

    let events: Vec<Value> = result
        .events
        .iter()
        .map(|e| {
            json!({
                "uid": e.uid,
                "title": e.title,
                "start": iso(e.starts_at),
                "end": iso(e.ends_at),
                "starts_at": e.starts_at,
                "ends_at": e.ends_at,
                "all_day": e.all_day,
                "cancelled": e.cancelled,
                "location": e.location,
                "join_url": e.join_url,
                "attendees": e.attendees.iter().map(|a| json!({
                    "email": a.email,
                    "name": a.name,
                    "organizer": a.organizer,
                    "partstat": a.partstat,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    (
        0,
        json!({
            "mode": "calendar_dump",
            "source": source,
            "from": iso(from_ms),
            "to": iso(to_ms),
            "default_tz": tz,
            "has_attendee_data": result.has_attendee_data,
            "count": events.len(),
            "warnings": result.warnings,
            "events": events,
        }),
    )
}

/// Lesbare Tabelle fuer den Aufruf ohne `--json`.
pub fn format_table(payload: &Value) -> String {
    let mut out = String::new();
    if let Some(err) = payload.get("error").and_then(Value::as_str) {
        return format!("Fehler: {err}");
    }
    out.push_str(&format!(
        "{} Termine, {} bis {} (Standardzone {}), Teilnehmerdaten: {}\n",
        payload["count"],
        payload["from"].as_str().unwrap_or(""),
        payload["to"].as_str().unwrap_or(""),
        payload["default_tz"].as_str().unwrap_or(""),
        if payload["has_attendee_data"].as_bool().unwrap_or(false) {
            "ja"
        } else {
            "nein"
        }
    ));
    for e in payload["events"].as_array().into_iter().flatten() {
        let att = e["attendees"].as_array().map_or(0, Vec::len);
        out.push_str(&format!(
            "{}  {}{}{}  ({} Teilnehmende){}\n",
            e["start"].as_str().unwrap_or(""),
            e["title"].as_str().unwrap_or(""),
            if e["cancelled"].as_bool().unwrap_or(false) {
                " [abgesagt]"
            } else {
                ""
            },
            if e["all_day"].as_bool().unwrap_or(false) {
                " [ganztägig]"
            } else {
                ""
            },
            att,
            e["join_url"]
                .as_str()
                .map(|u| format!("  {u}"))
                .unwrap_or_default()
        ));
    }
    for w in payload["warnings"].as_array().into_iter().flatten() {
        out.push_str(&format!("Hinweis: {}\n", w.as_str().unwrap_or("")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/calendar")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    const NOW: i64 = 1_790_000_000_000;

    #[test]
    fn dump_of_the_outlook_fixture_lists_the_table() {
        let (code, p) = run_cli(
            &fixture("outlook_series.ics"),
            Some("2026-09-28"),
            Some("2026-11-08"),
            NOW,
        );
        assert_eq!(code, 0, "{p}");
        assert_eq!(p["count"], 8);
        assert_eq!(p["from"], "2026-09-28T00:00:00Z");
        assert_eq!(p["has_attendee_data"], true);
        let starts: Vec<&str> = p["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["start"].as_str().unwrap())
            .collect();
        assert_eq!(
            starts,
            [
                "2026-09-28T08:00:00Z",
                "2026-10-05T12:00:00Z",
                "2026-10-15T13:00:00Z",
                "2026-10-19T08:00:00Z",
                "2026-10-26T09:00:00Z",
                "2026-10-27T10:00:00Z",
                "2026-10-29T14:00:00Z",
                "2026-11-02T09:00:00Z"
            ]
        );
        let first = &p["events"][0];
        assert_eq!(first["title"], "Jour fixe Vertrieb");
        assert_eq!(first["attendees"].as_array().unwrap().len(), 3);
        assert_eq!(first["attendees"][0]["organizer"], true);
        assert_eq!(p["events"][3]["cancelled"], true);
        // Keine Beschreibung in der Ausgabe.
        assert!(!p.to_string().contains("Agenda"));
        // Die Tabelle nennt Titel, Zeit und den Absage-Merker.
        let table = format_table(&p);
        assert!(
            table.contains("2026-10-19T08:00:00Z  Abgesagt: Jour fixe Vertrieb [abgesagt]"),
            "{table}"
        );
    }

    #[test]
    fn bad_usage_is_exit_2_and_read_errors_exit_1() {
        let f = fixture("outlook_series.ics");
        assert_eq!(run_cli(&f, Some("gestern"), None, NOW).0, 2);
        assert_eq!(run_cli(&f, None, Some("2026-13-40"), NOW).0, 2);
        assert_eq!(
            run_cli(&f, Some("2026-11-08"), Some("2026-09-28"), NOW).0,
            2
        );
        assert_eq!(run_cli("", None, None, NOW).0, 2);
        assert_eq!(run_cli("C:/gibt/es/nicht.ics", None, None, NOW).0, 1);
        // Eine HTML-Datei ist kein Kalender.
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("login.html");
        std::fs::write(&html, "<html>Anmelden</html>").unwrap();
        let (code, p) = run_cli(html.to_str().unwrap(), None, None, NOW);
        assert_eq!(code, 1);
        assert!(p["error"].as_str().unwrap().contains("keinen Kalender"));
    }

    #[test]
    fn rfc3339_bounds_and_default_window_work() {
        let f = fixture("outlook_series.ics");
        let (code, p) = run_cli(
            &f,
            Some("2026-10-05T11:00:00+02:00"),
            Some("2026-10-05T23:00:00Z"),
            NOW,
        );
        assert_eq!(code, 0);
        assert_eq!(p["from"], "2026-10-05T09:00:00Z");
        assert_eq!(p["count"], 1, "nur der verschobene Jour fixe um 12:00Z");
        // Ohne Grenzen: 30 Tage um `now`.
        let (_, p) = run_cli(&f, None, None, NOW);
        let span = Utc
            .timestamp_millis_opt(NOW + 30 * DAY_MS)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        assert_eq!(p["to"], span.as_str());
    }

    #[test]
    fn the_source_never_names_more_than_the_host() {
        assert!(is_url("webcal://cal.example/private/SECRET.ics"));
        assert!(is_url("HTTPS://x.example/a"));
        assert!(!is_url("C:/kalender/a.ics"));
        assert!(!is_url("kalender.ics"));
        assert_eq!(
            fetch::host_hint("https://cal.example/private/SECRET.ics?k=1").as_deref(),
            Some("cal.example")
        );
    }
}
