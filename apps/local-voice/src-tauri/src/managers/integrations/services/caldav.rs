//! iCloud-Kalender per CalDAV (Welle 3, Spec 2026-10-06-verbindungen-aktionen).
//!
//! Anmeldung: Apple-ID (Feld `email`) und ein app-spezifisches Passwort (Fach `token`), als
//! HTTP Basic. Jede Adresse, auch die vom Server genannte, muss auf `*.icloud.com` liegen,
//! bevor die Anmeldung mitgeht.
//!
//! - **Suchen** (`discover`): `current-user-principal` -> `calendar-home-set` -> die Kalender
//!   mit Terminen (`VEVENT`), je drei `PROPFIND`. Die Antworten (XML, Multistatus) liest ein
//!   kleiner, toleranter Leser: Namensraum-Praefixe beliebig, nur die wenigen Felder, die
//!   gebraucht werden.
//! - **Termin anlegen** (`create_event`): `PUT <kalender>/<uid>.ics` mit `If-None-Match: *`. Die
//!   UID bildet der Aufrufer aus Lauf und Schritt: ein zweiter Versuch trifft auf `412` (gibt es
//!   schon) und gilt als erledigt. Nie ueberschreibt die App einen vorhandenen Termin.
//! - Lesen der Termine geht ueber den vorhandenen ICS-Kalender (Freigabelink von iCloud).

use chrono::{DateTime, Duration, Utc};
use regex::Regex;

use super::http::{check_url, ApiRequest, Method, ServiceError};
use super::ops::{Account, Created, Exec};
use super::registry::def;

/// Einstieg fuer iCloud.
pub const BASE: &str = "https://caldav.icloud.com/";

/// Ein Kalender mit Terminen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarInfo {
    /// Volle Adresse (endet mit `/`).
    pub url: url::Url,
    pub name: String,
}

/// Ein neuer Termin.
#[derive(Clone, Debug)]
pub struct NewCalEvent<'a> {
    /// Stabile Kennung (aus Lauf und Schritt), nur `[A-Za-z0-9@._-]`.
    pub uid: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    pub start: DateTime<Utc>,
    pub minutes: u32,
    pub stamp: DateTime<Utc>,
}

const XML: &str = "application/xml; charset=utf-8";

const P_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const P_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;
const P_CALENDARS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:displayname/><d:resourcetype/><c:supported-calendar-component-set/></d:prop></d:propfind>"#;

/// Inhalt des ersten Elements `name` (beliebiges Namensraum-Praefix), ohne Tags.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let re = Regex::new(&format!(
        r"(?s)<(?:[A-Za-z0-9_-]+:)?{name}\b[^>]*>(.*?)</(?:[A-Za-z0-9_-]+:)?{name}>"
    ))
    .ok()?;
    re.captures(xml).and_then(|c| c.get(1)).map(|m| m.as_str())
}

fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let Ok(re) = Regex::new(&format!(
        r"(?s)<(?:[A-Za-z0-9_-]+:)?{name}\b[^>]*>(.*?)</(?:[A-Za-z0-9_-]+:)?{name}>"
    )) else {
        return Vec::new();
    };
    re.captures_iter(xml)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect()
}

fn unescape(s: &str) -> String {
    s.trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `href` innerhalb einer Eigenschaft (`current-user-principal`, `calendar-home-set`).
fn href_in(xml: &str, property: &str) -> Option<String> {
    element(xml, property)
        .and_then(|inner| element(inner, "href"))
        .map(unescape)
        .filter(|h| !h.is_empty())
}

/// Kalender mit Terminen aus einer Multistatus-Antwort.
pub fn parse_calendars(home: &url::Url, xml: &str) -> Vec<CalendarInfo> {
    let calendar_tag = Regex::new(r"<(?:[A-Za-z0-9_-]+:)?calendar\s*/?>").expect("Muster");
    elements(xml, "response")
        .into_iter()
        .filter_map(|resp| {
            let href = unescape(element(resp, "href")?);
            let is_calendar =
                element(resp, "resourcetype").is_some_and(|r| calendar_tag.is_match(r));
            if !is_calendar {
                return None;
            }
            // Ohne Angabe der Komponenten: Termine erlaubt (RFC 4791).
            let events = element(resp, "supported-calendar-component-set")
                .map(|c| c.contains("\"VEVENT\"") || c.contains("'VEVENT'"))
                .unwrap_or(true);
            if !events {
                return None;
            }
            let mut url = home.join(&href).ok()?;
            if !url.path().ends_with('/') {
                url.set_path(&format!("{}/", url.path()));
            }
            let name = element(resp, "displayname")
                .map(unescape)
                .unwrap_or_default();
            Some(CalendarInfo { url, name })
        })
        .collect()
}

impl Account<'_> {
    /// Adresse pruefen, bevor die Anmeldung mitgeht (auch vom Server genannte Adressen).
    fn checked(&self, url: url::Url) -> Result<url::Url, ServiceError> {
        check_url(&url, def(self.service).hosts, false)?;
        Ok(url)
    }

    fn propfind(
        &self,
        url: url::Url,
        depth: &str,
        body: &str,
        exec: &mut Exec,
    ) -> Result<String, ServiceError> {
        let req = self.auth(
            ApiRequest::new(Method::Propfind, self.checked(url)?)
                .header("Depth", depth)
                .raw(XML, body),
        )?;
        Ok(exec(req)?.text)
    }
}

fn join(base: &url::Url, href: &str) -> Result<url::Url, ServiceError> {
    base.join(href)
        .map_err(|_| ServiceError::BadResponse("ungültige Adresse in der Antwort"))
}

/// Alle Kalender mit Terminen (siehe Moduldoku).
pub fn discover(acc: &Account, exec: &mut Exec) -> Result<Vec<CalendarInfo>, ServiceError> {
    let base = url::Url::parse(BASE).expect("feste Adresse");
    let xml = acc.propfind(base.clone(), "0", P_PRINCIPAL, exec)?;
    let principal = href_in(&xml, "current-user-principal").ok_or(ServiceError::BadResponse(
        "kein Konto (current-user-principal)",
    ))?;
    let principal = join(&base, &principal)?;
    let xml = acc.propfind(principal.clone(), "0", P_HOME, exec)?;
    let home = href_in(&xml, "calendar-home-set").ok_or(ServiceError::BadResponse(
        "kein Kalenderordner (calendar-home-set)",
    ))?;
    let home = acc.checked(join(&principal, &home)?)?;
    let xml = acc.propfind(home.clone(), "1", P_CALENDARS, exec)?;
    Ok(parse_calendars(&home, &xml))
}

/// Der Kalender nach Namen (ohne Gross-/Kleinschreibung); ohne Namen der erste.
pub fn pick<'a>(
    cals: &'a [CalendarInfo],
    wanted: Option<&str>,
) -> Result<&'a CalendarInfo, ServiceError> {
    if cals.is_empty() {
        return Err(ServiceError::Config(
            "In diesem iCloud-Konto gibt es keinen Kalender für Termine.".into(),
        ));
    }
    match wanted.map(str::trim).filter(|w| !w.is_empty()) {
        None => Ok(&cals[0]),
        Some(w) => cals
            .iter()
            .find(|c| c.name.trim().eq_ignore_ascii_case(w))
            .ok_or_else(|| {
                let names: Vec<&str> = cals.iter().map(|c| c.name.as_str()).collect();
                ServiceError::Config(format!(
                    "Den Kalender „{w}“ gibt es nicht. Vorhanden: {}.",
                    names.join(", ")
                ))
            }),
    }
}

/// Text fuer eine iCalendar-Eigenschaft (RFC 5545 3.3.11).
fn ics_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace(['\n', '\r'], "\\n")
}

/// Zeilen hoechstens 75 Bytes, Fortsetzung mit Leerzeichen (an Zeichengrenzen).
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for ch in line.chars() {
        let n = ch.len_utf8();
        if len + n > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(ch);
        len += n;
    }
    out
}

fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Der Termin als iCalendar (ohne Teilnehmende: es geht keine Einladung hinaus).
pub fn ics(ev: &NewCalEvent<'_>) -> String {
    let end = ev.start + Duration::minutes(i64::from(ev.minutes));
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Local Voice AI//Folgetermin//DE".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", ev.uid),
        format!("DTSTAMP:{}", stamp(ev.stamp)),
        format!("DTSTART:{}", stamp(ev.start)),
        format!("DTEND:{}", stamp(end)),
        format!("SUMMARY:{}", ics_text(ev.summary)),
    ];
    if !ev.description.trim().is_empty() {
        lines.push(format!("DESCRIPTION:{}", ics_text(ev.description)));
    }
    lines.extend([
        "BEGIN:VALARM".to_string(),
        "ACTION:DISPLAY".to_string(),
        "TRIGGER:-PT15M".to_string(),
        format!("DESCRIPTION:{}", ics_text(ev.summary)),
        "END:VALARM".to_string(),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
    ]);
    lines
        .iter()
        .map(|l| fold(l))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// Legt den Termin im gewaehlten Kalender an (Feld `calendar`, sonst der erste). Gibt es die
/// UID schon (`412`), war ein frueherer Versuch erfolgreich: kein zweiter Termin.
pub fn create_event(
    acc: &Account,
    ev: &NewCalEvent<'_>,
    exec: &mut Exec,
) -> Result<Created, ServiceError> {
    if ev.uid.is_empty()
        || !ev
            .uid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._-".contains(c))
    {
        return Err(ServiceError::Config(
            "Die Kennung des Termins ist ungültig.".into(),
        ));
    }
    let cals = discover(acc, exec)?;
    let cal = pick(&cals, acc.field_opt("calendar").as_deref())?;
    let url = acc.checked(join(&cal.url, &format!("{}.ics", ev.uid))?)?;
    let req = acc.auth(
        ApiRequest::new(Method::Put, url.clone())
            .header("If-None-Match", "*")
            .raw("text/calendar; charset=utf-8", ics(ev)),
    )?;
    match exec(req) {
        Ok(_) | Err(ServiceError::Status(412, _)) => Ok(Created {
            id: ev.uid.to_string(),
            url: None,
        }),
        Err(e) => Err(e),
    }
}

/// „Verbindung testen“: die Kalender suchen und nennen (nichts anlegen).
pub fn check(acc: &Account, exec: &mut Exec) -> Result<String, ServiceError> {
    let cals = discover(acc, exec)?;
    let chosen = pick(&cals, acc.field_opt("calendar").as_deref())?;
    let names: Vec<&str> = cals.iter().map(|c| c.name.as_str()).collect();
    Ok(format!(
        "Verbunden; Folgetermine gehen in „{}“ (vorhanden: {}).",
        chosen.name,
        names.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::*;
    use crate::managers::integrations::services::http::ApiReply;
    use crate::managers::integrations::services::registry::ServiceId;

    const PRINCIPAL: &str = r#"<?xml version="1.0"?><multistatus xmlns="DAV:"><response><href>/</href><propstat><prop><current-user-principal><href>/1234567/principal/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
    const HOME: &str = r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/1234567/principal/</d:href><d:propstat><d:prop><cal:calendar-home-set><d:href xmlns:d="DAV:">https://p42-caldav.icloud.com:443/1234567/calendars/</d:href></cal:calendar-home-set></d:prop></d:propstat></d:response></d:multistatus>"#;
    const CALS: &str = r#"<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<response><href>/1234567/calendars/</href><propstat><prop><resourcetype><collection/></resourcetype></prop></propstat></response>
<response><href>/1234567/calendars/home/</href><propstat><prop><displayname>Privat</displayname><resourcetype><collection/><C:calendar/></resourcetype><C:supported-calendar-component-set><C:comp name="VEVENT"/></C:supported-calendar-component-set></prop></propstat></response>
<response><href>/1234567/calendars/tasks/</href><propstat><prop><displayname>Erinnerungen</displayname><resourcetype><collection/><C:calendar/></resourcetype><C:supported-calendar-component-set><C:comp name="VTODO"/></C:supported-calendar-component-set></prop></propstat></response>
<response><href>/1234567/calendars/work/</href><propstat><prop><displayname>Arbeit &amp; Kunden</displayname><resourcetype><collection/><C:calendar/></resourcetype></prop></propstat></response>
</multistatus>"#;

    fn settings(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect()
    }

    struct Fake {
        calls: Vec<ApiRequest>,
        replies: Vec<Result<ApiReply, ServiceError>>,
    }

    impl Fake {
        fn icloud(put: Result<ApiReply, ServiceError>) -> Self {
            Self {
                calls: Vec::new(),
                replies: vec![
                    Ok(ApiReply::text(207, PRINCIPAL)),
                    Ok(ApiReply::text(207, HOME)),
                    Ok(ApiReply::text(207, CALS)),
                    put,
                ],
            }
        }
        fn exec(&mut self) -> impl FnMut(ApiRequest) -> Result<ApiReply, ServiceError> + '_ {
            move |req| {
                self.calls.push(req);
                if self.replies.is_empty() {
                    Ok(ApiReply::text(201, ""))
                } else {
                    self.replies.remove(0)
                }
            }
        }
    }

    fn event() -> NewCalEvent<'static> {
        NewCalEvent {
            uid: "lva-abc123@local-voice-ai",
            summary: "Nachgespräch; Go-Live, Teil 2",
            description: "Offene Punkte:\n- Abnahme",
            start: chrono::NaiveDate::from_ymd_opt(2026, 10, 12)
                .unwrap()
                .and_hms_opt(12, 30, 0)
                .unwrap()
                .and_utc(),
            minutes: 45,
            stamp: chrono::NaiveDate::from_ymd_opt(2026, 10, 6)
                .unwrap()
                .and_hms_opt(8, 0, 0)
                .unwrap()
                .and_utc(),
        }
    }

    #[test]
    fn calendars_are_found_with_any_namespace_prefix_and_only_with_events() {
        let home = url::Url::parse("https://p42-caldav.icloud.com/1234567/calendars/").unwrap();
        let cals = parse_calendars(&home, CALS);
        let names: Vec<&str> = cals.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Privat", "Arbeit & Kunden"]);
        assert_eq!(
            cals[0].url.as_str(),
            "https://p42-caldav.icloud.com/1234567/calendars/home/"
        );
    }

    #[test]
    fn an_event_is_put_once_into_the_chosen_calendar_with_basic_auth() {
        let st = settings(&[("email", "ich@icloud.com"), ("calendar", "arbeit & kunden")]);
        let acc = Account {
            service: ServiceId::Icloud,
            token: "abcd-efgh-ijkl-mnop",
            settings: &st,
        };
        let mut f = Fake::icloud(Ok(ApiReply::text(201, "")));
        let c = create_event(&acc, &event(), &mut f.exec()).unwrap();
        assert_eq!(c.id, "lva-abc123@local-voice-ai");
        assert_eq!(f.calls.len(), 4);
        assert_eq!(f.calls[0].method, Method::Propfind);
        assert_eq!(f.calls[0].header_value("Depth"), Some("0"));
        assert_eq!(
            f.calls[1].url.as_str(),
            "https://caldav.icloud.com/1234567/principal/"
        );
        assert_eq!(f.calls[2].url.host_str(), Some("p42-caldav.icloud.com"));
        assert_eq!(f.calls[2].header_value("Depth"), Some("1"));
        let put = &f.calls[3];
        assert_eq!(put.method, Method::Put);
        assert_eq!(
            put.url.path(),
            "/1234567/calendars/work/lva-abc123@local-voice-ai.ics"
        );
        assert_eq!(put.header_value("If-None-Match"), Some("*"));
        assert!(put
            .header_value("Authorization")
            .unwrap()
            .starts_with("Basic "));
        let body = &put.raw.as_ref().unwrap().1;
        assert!(body.contains("DTSTART:20261012T123000Z\r\n"), "{body}");
        assert!(body.contains("DTEND:20261012T131500Z\r\n"));
        assert!(body.contains("SUMMARY:Nachgespräch\\; Go-Live\\, Teil 2"));
        assert!(body.contains("DESCRIPTION:Offene Punkte:\\n- Abnahme"));
        assert!(!body.contains("ATTENDEE"), "keine Einladung");
        assert!(body.lines().all(|l| l.len() <= 75), "gefaltet");
    }

    #[test]
    fn an_existing_uid_counts_as_done_and_nothing_is_overwritten() {
        let st = settings(&[("email", "ich@icloud.com")]);
        let acc = Account {
            service: ServiceId::Icloud,
            token: "pw",
            settings: &st,
        };
        let mut f = Fake::icloud(Err(ServiceError::Status(412, String::new())));
        assert!(create_event(&acc, &event(), &mut f.exec()).is_ok());
        assert_eq!(
            f.calls[3].url.path(),
            "/1234567/calendars/home/lva-abc123@local-voice-ai.ics"
        );
    }

    #[test]
    fn a_home_outside_icloud_is_refused_before_the_password_goes_there() {
        let st = settings(&[("email", "ich@icloud.com")]);
        let acc = Account {
            service: ServiceId::Icloud,
            token: "pw",
            settings: &st,
        };
        let evil = HOME.replace("p42-caldav.icloud.com:443", "evil.example");
        let mut f = Fake {
            calls: Vec::new(),
            replies: vec![
                Ok(ApiReply::text(207, PRINCIPAL)),
                Ok(ApiReply::text(207, evil)),
            ],
        };
        let e = create_event(&acc, &event(), &mut f.exec()).unwrap_err();
        assert!(matches!(e, ServiceError::Config(_)), "{e:?}");
        assert_eq!(f.calls.len(), 2, "kein Aufruf an den fremden Server");
    }

    #[test]
    fn an_unknown_calendar_name_lists_the_existing_ones() {
        let st = settings(&[("email", "ich@icloud.com"), ("calendar", "Gibts nicht")]);
        let acc = Account {
            service: ServiceId::Icloud,
            token: "pw",
            settings: &st,
        };
        let mut f = Fake::icloud(Ok(ApiReply::text(201, "")));
        let e = check(&acc, &mut f.exec()).unwrap_err();
        assert!(e.to_string().contains("Privat, Arbeit & Kunden"), "{e}");
        let st = settings(&[("email", "ich@icloud.com")]);
        let acc = Account {
            service: ServiceId::Icloud,
            token: "pw",
            settings: &st,
        };
        let mut f = Fake::icloud(Ok(ApiReply::text(201, "")));
        assert!(check(&acc, &mut f.exec()).unwrap().contains("„Privat“"));
        assert_eq!(f.calls.len(), 3, "testen legt nichts an");
    }

    #[test]
    fn long_lines_are_folded_on_character_boundaries() {
        let long = "ä".repeat(60);
        let folded = fold(&format!("SUMMARY:{long}"));
        for part in folded.split("\r\n") {
            assert!(part.len() <= 75, "{}", part.len());
        }
        assert_eq!(folded.replace("\r\n ", ""), format!("SUMMARY:{long}"));
    }
}
