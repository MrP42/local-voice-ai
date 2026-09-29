//! ICS lesen und Serien expandieren (M5, `entwurf/m5-m6-kalender-export.md` §3 F15).
//!
//! Rein: kein Netz, kein Datentraeger, keine Uhr (das Fenster kommt als
//! Argument). Die Bibliothek `calcard` steht ausschliesslich in dieser Datei;
//! ein Wechsel zu `icalendar` + `rrule` bleibt hinter `parse_and_expand`.
//!
//! Umgangene Fehler von calcard 0.3.14 (je ein Regressionstest):
//! - V8: eine Serie mit UTC-Beginn (`DTSTART:...Z` + RRULE) wird um den
//!   Ortsversatz verschoben. `normalize_utc_rrule_starts` schreibt den Beginn
//!   vor dem Parsen in `DTSTART;TZID=UTC:` um.
//! - V9: das Expansionslimit gilt fuer den ganzen Kalender. Eine alte
//!   Tagesserie frisst es, spaetere Serien liefern nichts. Wir expandieren
//!   je UID-Gruppe mit eigenem Limit.
//! - V7: ein Override (RECURRENCE-ID) erbt nichts vom Master. Teilnehmende,
//!   Organisator, Ort, Beschreibung und Beitritts-Adresse erbt unser Code.
//! - V12 (neu in P5a): Overrides werden nur dem Master zugeordnet, wenn ihre
//!   SEQUENCE gleich ist. Outlook zaehlt sie aber je Aenderung hoch, dann bliebe
//!   die Originalinstanz stehen und der Override kaeme doppelt. Wir entfernen
//!   SEQUENCE vor der Expansion.

use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

use calcard::common::timezone::Tz;
use calcard::icalendar::dates::TimeOrDelta;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarParameterName,
    ICalendarProperty, ICalendarStatus, ICalendarValue,
};
use calcard::{Entry, Parser};
use regex::Regex;
use sha2::{Digest, Sha256};

use super::model::{event_key, Attendee, CalEvent, CalendarError};

/// Obergrenze der Instanzen JE SERIE (UID-Gruppe). Eine Tagesserie ab dem Jahr
/// 2000 hat bis heute rund 9 700 Instanzen und passt darunter.
pub const MAX_INSTANCES_PER_SERIES: usize = 20_000;
/// Schutz gegen viele unbegrenzte Serien: Summe aller erzeugten Instanzen.
const MAX_TOTAL_INSTANCES: usize = 2_000_000;
/// Mehr Termine im Fenster speichert keine Quelle (Speicher, Datenbank).
pub const MAX_EVENTS_PER_SOURCE: usize = 20_000;
const MAX_DESCRIPTION_CHARS: usize = 4_000;
const MAX_LOCATION_CHARS: usize = 500;
const MAX_TITLE_CHARS: usize = 300;
const MAX_ATTENDEES: usize = 500;
const MAX_JOIN_URL_CHARS: usize = 2_000;
const MAX_WARNINGS: usize = 100;
/// Titel fuer Termine ohne SUMMARY.
pub const UNTITLED: &str = "(ohne Titel)";

/// Ergebnis von `parse_and_expand`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IcsResult {
    /// Termine im Fenster, nach Beginn und UID sortiert, ohne Dubletten.
    /// Abgesagte stehen darin (`cancelled = true`).
    pub events: Vec<CalEvent>,
    /// Klartext-Hinweise (uebersprungene Komponenten, unbekannte Zeitzone, ...).
    pub warnings: Vec<String>,
    /// Mindestens ein Termin der Datei fuehrt ATTENDEE-Zeilen.
    pub has_attendee_data: bool,
}

/// Beginnt der Text (nach BOM und Leerraum) mit `BEGIN:VCALENDAR`? Google
/// liefert bei falscher Adresse eine HTML-Seite mit Status 200.
pub fn looks_like_calendar(body: &str) -> bool {
    let t = body.trim_start_matches('\u{feff}').trim_start();
    t.get(..15)
        .is_some_and(|head| head.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
}

/// Name der lokalen Windows-Zeitzone (`W. Europe Standard Time`), sonst `None`.
/// `calcard` loest Windows-Namen und IANA-Namen auf.
pub fn system_tz_name() -> Option<String> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation")
            .ok()?;
        let name: String = key.get_value("TimeZoneKeyName").ok()?;
        let name = name.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!name.is_empty()).then(|| name.to_string())
    }
    #[cfg(not(windows))]
    {
        if let Ok(tz) = std::env::var("TZ") {
            let tz = tz.trim_start_matches(':').trim();
            if !tz.is_empty() {
                return Some(tz.to_string());
            }
        }
        let link = std::fs::read_link("/etc/localtime").ok()?;
        let s = link.to_string_lossy();
        s.split("zoneinfo/").nth(1).map(|n| n.to_string())
    }
}

// ---------------------------------------------------------------------------
// V8: UTC-Serien
// ---------------------------------------------------------------------------

/// Schreibt in jedem VEVENT mit RRULE die Zeilen `DTSTART:...Z` und `DTEND:...Z`
/// in `DTSTART;TZID=UTC:...` um (V8). Alles andere bleibt byte-gleich; ohne
/// betroffene Serie kommt der Eingabetext unveraendert (`Cow::Borrowed`) zurueck.
///
/// Overrides (RECURRENCE-ID) und Einzeltermine mit `Z` sind nicht betroffen:
/// calcard rechnet sie richtig.
pub fn normalize_utc_rrule_starts(raw: &str) -> Cow<'_, str> {
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    let mut replacements: Vec<(usize, String)> = Vec::new();
    let mut block_start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\r', '\n']);
        if body.eq_ignore_ascii_case("BEGIN:VEVENT") {
            block_start = Some(i);
        } else if body.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(start) = block_start.take() {
                collect_utc_series_rewrites(&lines[start..=i], start, &mut replacements);
            }
        }
    }
    if replacements.is_empty() {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len() + replacements.len() * 12);
    let mut next = replacements.iter().peekable();
    for (i, line) in lines.iter().enumerate() {
        match next.peek() {
            Some((idx, new_line)) if *idx == i => {
                out.push_str(new_line);
                next.next();
            }
            _ => out.push_str(line),
        }
    }
    Cow::Owned(out)
}

/// Gibt es ein `\n` ohne vorangehendes `\r`?
fn has_bare_lf(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter()
        .enumerate()
        .any(|(i, &c)| c == b'\n' && (i == 0 || b[i - 1] != b'\r'))
}

fn property_name(body: &str) -> &str {
    body.split([':', ';']).next().unwrap_or("")
}

fn collect_utc_series_rewrites(block: &[&str], offset: usize, out: &mut Vec<(usize, String)>) {
    let has_rrule = block.iter().any(|l| {
        let name = property_name(l);
        name.eq_ignore_ascii_case("RRULE") && l.len() > name.len()
    });
    if !has_rrule {
        return;
    }
    for (j, line) in block.iter().enumerate() {
        let eol = &line[line.trim_end_matches(['\r', '\n']).len()..];
        let body = line.trim_end_matches(['\r', '\n']);
        let name = property_name(body);
        if !(name.eq_ignore_ascii_case("DTSTART") || name.eq_ignore_ascii_case("DTEND")) {
            continue;
        }
        let rest = &body[name.len()..];
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let params = &rest[..colon];
        let value = rest[colon + 1..].trim();
        // Angefuehrte Parameterwerte koennen ':' enthalten: nicht anfassen.
        if params.contains('"') || params.to_ascii_uppercase().contains("TZID=") {
            continue;
        }
        let is_utc_stamp = value.len() == 16
            && value.ends_with(['Z', 'z'])
            && value[..8].bytes().all(|b| b.is_ascii_digit())
            && value.as_bytes()[8] == b'T'
            && value[9..15].bytes().all(|b| b.is_ascii_digit());
        if !is_utc_stamp {
            continue;
        }
        out.push((
            offset + j,
            format!("{name};TZID=UTC{params}:{}{eol}", &value[..15]),
        ));
    }
}

// ---------------------------------------------------------------------------
// Beitritts-Adresse
// ---------------------------------------------------------------------------

static JOIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)https?://(?:[a-z0-9-]+\.)*(?:teams\.microsoft\.com/l/meetup-join/|teams\.live\.com/meet/|meet\.google\.com/|zoom\.us/(?:j|my|wc/join)/|zoom\.com/(?:j|my)/|webex\.com/)[^\s<>"'\\)\]]+"#,
    )
    .expect("join regex")
});

/// Beitritts-Adresse (Teams, Meet, Zoom, Webex) aus den Feldern, in der Reihenfolge
/// der Felder: das erste Feld mit Treffer gewinnt, innerhalb eines Feldes der
/// erste Treffer. Ein angehaengtes Satzzeichen wird abgeschnitten.
pub fn extract_join_url(fields: &[&str]) -> Option<String> {
    for field in fields {
        if let Some(m) = JOIN_RE.find(field) {
            let url = m
                .as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?', '>', '"', '\'']);
            if !url.is_empty() && url.chars().count() <= MAX_JOIN_URL_CHARS {
                return Some(url.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Felder eines Termins
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Person {
    email: Option<String>,
    name: Option<String>,
    partstat: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Fields {
    summary: Option<String>,
    location: Option<String>,
    description: Option<String>,
    /// `X-MICROSOFT-SKYPETEAMSMEETINGURL`.
    teams_url: Option<String>,
    /// `X-GOOGLE-CONFERENCE`, `CONFERENCE`, `URL`.
    other_urls: Vec<String>,
    organizer: Option<Person>,
    attendees: Vec<Person>,
    /// `Some(true)` bei STATUS:CANCELLED, `Some(false)` bei anderem Status.
    cancelled: Option<bool>,
}

impl Fields {
    /// V7: was der Override nicht selbst traegt, kommt vom Master.
    fn inherit_from(mut self, master: &Fields) -> Fields {
        if self.summary.is_none() {
            self.summary = master.summary.clone();
        }
        if self.location.is_none() {
            self.location = master.location.clone();
        }
        if self.description.is_none() {
            self.description = master.description.clone();
        }
        if self.teams_url.is_none() {
            self.teams_url = master.teams_url.clone();
        }
        if self.other_urls.is_empty() {
            self.other_urls = master.other_urls.clone();
        }
        if self.organizer.is_none() {
            self.organizer = master.organizer.clone();
        }
        if self.attendees.is_empty() {
            self.attendees = master.attendees.clone();
        }
        if self.cancelled.is_none() {
            self.cancelled = master.cancelled;
        }
        self
    }
}

fn first_text(comp: &ICalendarComponent, prop: &ICalendarProperty) -> Option<String> {
    comp.property(prop)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn other_property(comp: &ICalendarComponent, name: &str) -> Option<String> {
    comp.entries.iter().find_map(|e| match &e.name {
        ICalendarProperty::Other(n) if n.eq_ignore_ascii_case(name) => e
            .values
            .first()
            .and_then(|v| v.as_text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        _ => None,
    })
}

fn normalize_email(raw: &str) -> Option<String> {
    let t = raw.trim();
    let t = match t.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("mailto:") => &t[7..],
        _ => t,
    };
    let e = t.trim().to_lowercase();
    // Exchange liefert bei internen Adressen teils `/O=...` oder `X500:` statt
    // einer E-Mail-Adresse: das ist keine Adresse.
    (e.contains('@') && !e.contains(char::is_whitespace) && !e.contains('/')).then_some(e)
}

fn person_from(entry: &ICalendarEntry) -> Option<Person> {
    let address = entry.values.first().and_then(|v| v.as_text());
    let email = address.and_then(normalize_email);
    let cn = entry
        .parameter(&ICalendarParameterName::Cn)
        .and_then(|v| v.as_text())
        .map(|s| s.trim().trim_matches('"').trim().to_string())
        .filter(|s| !s.is_empty());
    // Google traegt die Adresse als Anzeigenamen ein: das ist kein Name.
    let name = cn.filter(|n| email.as_deref() != Some(n.to_lowercase().as_str()));
    if email.is_none() && name.is_none() {
        return None;
    }
    let partstat = entry
        .parameter(&ICalendarParameterName::Partstat)
        .and_then(|v| v.as_text())
        .map(str::to_string);
    Some(Person {
        email,
        name,
        partstat,
    })
}

fn read_fields(comp: &ICalendarComponent) -> Fields {
    let mut f = Fields {
        summary: first_text(comp, &ICalendarProperty::Summary),
        location: first_text(comp, &ICalendarProperty::Location),
        description: first_text(comp, &ICalendarProperty::Description),
        teams_url: other_property(comp, "X-MICROSOFT-SKYPETEAMSMEETINGURL"),
        ..Default::default()
    };
    for name in ["X-GOOGLE-CONFERENCE"] {
        if let Some(u) = other_property(comp, name) {
            f.other_urls.push(u);
        }
    }
    if let Some(u) = first_text(comp, &ICalendarProperty::Conference) {
        f.other_urls.push(u);
    }
    if let Some(u) = first_text(comp, &ICalendarProperty::Url) {
        f.other_urls.push(u);
    }
    f.organizer = comp
        .property(&ICalendarProperty::Organizer)
        .and_then(person_from);
    f.attendees = comp
        .properties(&ICalendarProperty::Attendee)
        .filter_map(person_from)
        .collect();
    f.cancelled = comp
        .status()
        .map(|s| matches!(s, ICalendarStatus::Cancelled));
    f
}

/// Teilnehmende fuer den Cache: Organisator zuerst, Dubletten (gleiche Adresse,
/// sonst gleicher Name) zusammengefasst, der Organisator-Merker bleibt erhalten.
fn build_attendees(fields: &Fields, truncated: &mut bool) -> Vec<Attendee> {
    let mut out: Vec<Attendee> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut push = |p: &Person, organizer: bool| {
        let key = match (&p.email, &p.name) {
            (Some(e), _) => format!("e:{e}"),
            (None, Some(n)) => format!("n:{}", n.to_lowercase()),
            (None, None) => return,
        };
        if let Some(&i) = index.get(&key) {
            let a = &mut out[i];
            a.organizer |= organizer;
            if a.name.is_none() {
                a.name = p.name.clone();
            }
            if a.partstat.is_none() {
                a.partstat = p.partstat.clone();
            }
            return;
        }
        if out.len() >= MAX_ATTENDEES {
            *truncated = true;
            return;
        }
        index.insert(key, out.len());
        out.push(Attendee {
            email: p.email.clone(),
            name: p.name.clone(),
            organizer,
            is_self: false,
            partstat: p.partstat.clone(),
        });
    };
    if let Some(o) = &fields.organizer {
        push(o, true);
    }
    for a in &fields.attendees {
        push(a, false);
    }
    out
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

fn is_http_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

fn join_url_of(f: &Fields) -> Option<String> {
    // Die ausdrueckliche Teams-Eigenschaft gilt auch dann, wenn die Adresse
    // keinem Muster entspricht.
    if let Some(t) = f.teams_url.as_deref().filter(|t| is_http_url(t)) {
        let t = t.trim();
        if t.chars().count() <= MAX_JOIN_URL_CHARS {
            return Some(t.to_string());
        }
    }
    let mut fields: Vec<&str> = f.other_urls.iter().map(String::as_str).collect();
    if let Some(l) = &f.location {
        fields.push(l);
    }
    if let Some(d) = &f.description {
        fields.push(d);
    }
    extract_join_url(&fields)
}

// ---------------------------------------------------------------------------
// Aufbau der Gruppen
// ---------------------------------------------------------------------------

/// Tiefer als VCALENDAR > VEVENT > VALARM oder VTIMEZONE > STANDARD wird nichts
/// verschachtelt. Fuer Eingaben mit tausenden offenen `BEGIN:` lehnt
/// `parse_and_expand` schon vor dem Parsen ab; die Tiefe hier ist der zweite
/// Riegel, damit die Rekursion des Kopierens nie den Stapel sprengt.
const MAX_NESTING: usize = 16;

fn nesting_too_deep(text: &str) -> bool {
    let starts = |line: &str, prefix: &str| {
        line.get(..prefix.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(prefix))
    };
    let mut depth = 0usize;
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            continue; // Fortsetzungszeile
        }
        if starts(line, "BEGIN:") {
            depth += 1;
            if depth > MAX_NESTING {
                return true;
            }
        } else if starts(line, "END:") {
            depth = depth.saturating_sub(1);
        }
    }
    false
}

/// Kopie eines Teilbaums samt SEQUENCE-Entfernung (V12) in `out`.
fn clone_subtree(cal: &ICalendar, id: u32, out: &mut Vec<ICalendarComponent>) -> u32 {
    clone_subtree_at(cal, id, 0, out)
}

fn clone_subtree_at(
    cal: &ICalendar,
    id: u32,
    depth: usize,
    out: &mut Vec<ICalendarComponent>,
) -> u32 {
    let new_id = out.len() as u32;
    let src = &cal.components[id as usize];
    let mut entries = src.entries.clone();
    if src.component_type == ICalendarComponentType::VEvent {
        entries.retain(|e| e.name != ICalendarProperty::Sequence);
    }
    out.push(ICalendarComponent {
        component_type: src.component_type.clone(),
        entries,
        component_ids: Vec::new(),
    });
    let children: Vec<u32> = if depth >= MAX_NESTING {
        Vec::new()
    } else {
        src.component_ids
            .iter()
            .filter(|c| (**c as usize) < cal.components.len())
            .map(|c| clone_subtree_at(cal, *c, depth + 1, out))
            .collect()
    };
    out[new_id as usize].component_ids = children;
    new_id
}

/// Ein Kalender nur aus den gewaehlten VEVENTs und allen VTIMEZONEs.
fn sub_calendar(cal: &ICalendar, timezones: &[u32], events: &[u32]) -> ICalendar {
    let mut components = vec![ICalendarComponent {
        component_type: ICalendarComponentType::VCalendar,
        entries: Vec::new(),
        component_ids: Vec::new(),
    }];
    let mut children = Vec::new();
    for id in timezones.iter().chain(events.iter()) {
        children.push(clone_subtree(cal, *id, &mut components));
    }
    components[0].component_ids = children;
    ICalendar { components }
}

struct Group {
    uid: String,
    ids: Vec<u32>,
}

fn is_override(comp: &ICalendarComponent) -> bool {
    comp.has_property(&ICalendarProperty::RecurrenceId)
}

fn is_recurring(comp: &ICalendarComponent) -> bool {
    comp.has_property(&ICalendarProperty::Rrule) || comp.has_property(&ICalendarProperty::Rdate)
}

fn synthetic_uid(comp: &ICalendarComponent) -> String {
    let mut h = Sha256::new();
    h.update(first_text(comp, &ICalendarProperty::Summary).unwrap_or_default());
    if let Some(e) = comp.property(&ICalendarProperty::Dtstart) {
        h.update(format!("{:?}", e.values.first()));
    }
    let digest = h.finalize();
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("noid-{hex}")
}

struct Warnings {
    list: Vec<String>,
    dropped: usize,
}

impl Warnings {
    fn new() -> Self {
        Self {
            list: Vec::new(),
            dropped: 0,
        }
    }
    fn push(&mut self, msg: String) {
        if self.list.contains(&msg) {
            return;
        }
        if self.list.len() < MAX_WARNINGS {
            self.list.push(msg);
        } else {
            self.dropped += 1;
        }
    }
    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.list
                .push(format!("... und {} weitere Hinweise", self.dropped));
        }
        self.list
    }
}

fn dtstart_is_all_day(comp: &ICalendarComponent) -> Option<bool> {
    let e = comp.property(&ICalendarProperty::Dtstart)?;
    match e.values.first() {
        Some(ICalendarValue::PartialDateTime(dt)) => Some(!dt.has_time()),
        _ => None,
    }
}

fn error_reason(e: &calcard::icalendar::dates::CalendarErrorType) -> &'static str {
    use calcard::icalendar::dates::CalendarErrorType as T;
    match e {
        T::MissingDtStart => "kein Beginn",
        T::InvalidDtStart => "ungültiger Beginn",
        T::InvalidDtEnd => "ungültiges Ende",
        T::InvalidDuration => "ungültige Dauer",
        T::RRule(_) => "ungültige Wiederholungsregel",
    }
}

/// Liest ICS, expandiert Serien und liefert die Termine im Fenster
/// `[from_ms, to_ms)` (ms UTC; ein Termin zaehlt, wenn er das Fenster beruehrt).
///
/// `default_tz`: Zone fuer Zeiten ohne Zone (schwebend, Ganztag) und
/// unbekannte TZIDs; Windows- oder IANA-Name, sonst UTC mit Hinweis.
///
/// Fehler nur, wenn der Text gar kein Kalender ist. Einzelne kaputte
/// Komponenten werden uebersprungen und als Hinweis gemeldet.
pub fn parse_and_expand(
    raw: &str,
    source_id: &str,
    from_ms: i64,
    to_ms: i64,
    default_tz: &str,
) -> Result<IcsResult, CalendarError> {
    if !looks_like_calendar(raw) {
        return Err(CalendarError::NotCalendar);
    }
    let mut warnings = Warnings::new();
    if nesting_too_deep(raw) {
        return Err(CalendarError::Parse(
            "Komponenten sind zu tief verschachtelt".to_string(),
        ));
    }

    let normalized = normalize_utc_rrule_starts(raw);
    // calcard erwartet CRLF (wie ein echter Server); reine LF-Dateien
    // (Export von Hand, Git-Checkout) wandeln wir vorher um.
    let text: Cow<'_, str> = if has_bare_lf(&normalized) {
        Cow::Owned(normalized.replace("\r\n", "\n").replace('\n', "\r\n"))
    } else {
        normalized
    };

    let cal = match Parser::new(&text).entry() {
        Entry::ICalendar(c) => c,
        Entry::Eof => return Err(CalendarError::NotCalendar),
        Entry::InvalidLine(line) => {
            return Err(CalendarError::Parse(format!(
                "ungültige Zeile „{}“",
                cap_chars(&line, 40)
            )))
        }
        Entry::UnterminatedComponent(name) => {
            return Err(CalendarError::Parse(format!(
                "Komponente {name} nicht beendet"
            )))
        }
        _ => return Err(CalendarError::Parse("unbekanntes Format".to_string())),
    };
    if cal.components.first().map(|c| &c.component_type) != Some(&ICalendarComponentType::VCalendar)
    {
        return Err(CalendarError::NotCalendar);
    }

    let tz = match Tz::from_str(default_tz.trim()) {
        Ok(tz) => tz,
        Err(()) => {
            warnings.push(format!(
                "Zeitzone „{}“ unbekannt, UTC als Standardzone verwendet.",
                cap_chars(default_tz, 60)
            ));
            Tz::UTC
        }
    };

    let root = &cal.components[0];
    let mut timezones: Vec<u32> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut group_of: HashMap<String, usize> = HashMap::new();
    let mut has_attendee_data = false;
    for &id in &root.component_ids {
        let Some(comp) = cal.components.get(id as usize) else {
            continue;
        };
        match comp.component_type {
            ICalendarComponentType::VTimezone => timezones.push(id),
            ICalendarComponentType::VEvent => {
                if comp.has_property(&ICalendarProperty::Attendee) {
                    has_attendee_data = true;
                }
                // Eine Regel, die calcard nicht versteht, laesst den Termin als
                // Einzeltermin stehen: das ist besser als er fehlt, aber nicht still.
                let bad_rule = comp.property(&ICalendarProperty::Rrule).is_some_and(|e| {
                    !matches!(e.values.first(), Some(ICalendarValue::RecurrenceRule(_)))
                });
                if bad_rule {
                    warnings.push(format!(
                        "Wiederholungsregel nicht lesbar, nur der erste Termin gezeigt: {}",
                        cap_chars(
                            &first_text(comp, &ICalendarProperty::Summary)
                                .unwrap_or_else(|| UNTITLED.to_string()),
                            80
                        )
                    ));
                }
                let uid = comp
                    .uid()
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| synthetic_uid(comp));
                let gi = *group_of.entry(uid.clone()).or_insert_with(|| {
                    groups.push(Group {
                        uid: uid.clone(),
                        ids: Vec::new(),
                    });
                    groups.len() - 1
                });
                groups[gi].ids.push(id);
            }
            _ => {}
        }
    }

    // Unbekannte TZIDs: calcard nimmt dann still die Standardzone.
    {
        let resolver = cal.build_tz_resolver();
        let mut seen: Vec<&str> = Vec::new();
        for g in &groups {
            for &id in &g.ids {
                for entry in &cal.components[id as usize].entries {
                    if let Some(tzid) = entry.tz_id() {
                        if !seen.contains(&tzid) {
                            seen.push(tzid);
                            if resolver.resolve(tzid).is_none() {
                                warnings.push(format!(
                                    "Unbekannte Zeitzone „{}“, Standardzone verwendet.",
                                    cap_chars(tzid, 60)
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    let mut events: Vec<CalEvent> = Vec::new();
    let mut total_instances = 0usize;
    let mut budget_hit = false;

    // Einfache Termine (ein VEVENT, keine Wiederholung, kein Override) verbrauchen
    // kein Limit und werden in EINEM Aufruf expandiert.
    let (simple, complex): (Vec<&Group>, Vec<&Group>) = groups.iter().partition(|g| {
        g.ids.len() == 1 && {
            let c = &cal.components[g.ids[0] as usize];
            !is_recurring(c) && !is_override(c)
        }
    });

    if !simple.is_empty() {
        let ids: Vec<u32> = simple.iter().flat_map(|g| g.ids.iter().copied()).collect();
        let sub = sub_calendar(&cal, &timezones, &ids);
        let uids: Vec<&str> = simple.iter().map(|g| g.uid.as_str()).collect();
        // Die Komponenten der Teilkalender stehen in der Reihenfolge von `ids`,
        // hinter der VCALENDAR-Wurzel und den Zeitzonen (samt Kindern).
        let by_comp = map_events_to_uids(&sub, &uids);
        let expand = sub.expand_dates(tz, MAX_INSTANCES_PER_SERIES);
        total_instances += expand.events.len();
        collect_expanded(
            &sub,
            &expand,
            &|comp_id| by_comp.get(&comp_id).map(|u| (*u).to_string()),
            None,
            source_id,
            from_ms,
            to_ms,
            &mut events,
            &mut warnings,
        );
    }

    for g in complex {
        if total_instances >= MAX_TOTAL_INSTANCES {
            budget_hit = true;
            break;
        }
        let sub = sub_calendar(&cal, &timezones, &g.ids);
        let expand = sub.expand_dates(tz, MAX_INSTANCES_PER_SERIES);
        total_instances += expand.events.len();

        // Master: erste Komponente ohne RECURRENCE-ID, bevorzugt mit RRULE.
        let vevents: Vec<usize> = sub
            .components
            .iter()
            .enumerate()
            .filter(|(_, c)| c.component_type == ICalendarComponentType::VEvent)
            .map(|(i, _)| i)
            .collect();
        let master = vevents
            .iter()
            .find(|&&i| {
                let c = &sub.components[i];
                !is_override(c) && c.has_property(&ICalendarProperty::Rrule)
            })
            .or_else(|| vevents.iter().find(|&&i| !is_override(&sub.components[i])))
            .map(|&i| read_fields(&sub.components[i]));

        // Gekuerzte Serie: das Limit ist ausgeschoepft und die letzte erzeugte
        // Instanz liegt vor dem Fensterende. Erzeugte Instanzen sind richtig;
        // es fehlen nur die spaeteren.
        let exdates: usize = sub
            .components
            .iter()
            .flat_map(|c| c.properties(&ICalendarProperty::Exdate))
            .map(|e| e.values.len())
            .sum();
        if expand.events.len() + exdates >= MAX_INSTANCES_PER_SERIES {
            let last = expand
                .events
                .iter()
                .map(|e| e.start.timestamp_millis())
                .max()
                .unwrap_or(i64::MIN);
            if last < to_ms {
                let title = vevents
                    .first()
                    .and_then(|&i| first_text(&sub.components[i], &ICalendarProperty::Summary))
                    .unwrap_or_else(|| UNTITLED.to_string());
                warnings.push(format!(
                    "Serie gekürzt (mehr als {MAX_INSTANCES_PER_SERIES} Wiederholungen): {}",
                    cap_chars(&title, 80)
                ));
            }
        }

        let uid = g.uid.clone();
        collect_expanded(
            &sub,
            &expand,
            &|_| Some(uid.clone()),
            master.as_ref(),
            source_id,
            from_ms,
            to_ms,
            &mut events,
            &mut warnings,
        );
    }
    if budget_hit {
        warnings.push(
            "Zu viele Wiederholungen im Kalender: einige Serien wurden ausgelassen.".to_string(),
        );
    }

    // Dubletten (gleiche UID und Beginn): der spaetere Eintrag gewinnt.
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut unique: Vec<CalEvent> = Vec::with_capacity(events.len());
    for e in events {
        match by_key.get(&e.key) {
            Some(&i) => unique[i] = e,
            None => {
                by_key.insert(e.key.clone(), unique.len());
                unique.push(e);
            }
        }
    }
    unique.sort_by(|a, b| (a.starts_at, &a.uid).cmp(&(b.starts_at, &b.uid)));
    if unique.len() > MAX_EVENTS_PER_SOURCE {
        unique.truncate(MAX_EVENTS_PER_SOURCE);
        warnings.push(format!(
            "Mehr als {MAX_EVENTS_PER_SOURCE} Termine im Fenster: die spätesten wurden ausgelassen."
        ));
    }

    Ok(IcsResult {
        events: unique,
        warnings: warnings.finish(),
        has_attendee_data,
    })
}

/// Ordnet in einem Teilkalender aus lauter Einzelterminen jede VEVENT-
/// Komponente ihrer UID zu (die Reihenfolge der VEVENTs entspricht der der UIDs).
fn map_events_to_uids<'a>(sub: &ICalendar, uids: &[&'a str]) -> HashMap<u32, &'a str> {
    sub.components
        .iter()
        .enumerate()
        .filter(|(_, c)| c.component_type == ICalendarComponentType::VEvent)
        .map(|(i, _)| i as u32)
        .zip(uids.iter().copied())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn collect_expanded(
    sub: &ICalendar,
    expand: &calcard::icalendar::dates::CalendarExpand,
    uid_of: &dyn Fn(u32) -> Option<String>,
    master: Option<&Fields>,
    source_id: &str,
    from_ms: i64,
    to_ms: i64,
    out: &mut Vec<CalEvent>,
    warnings: &mut Warnings,
) {
    for err in &expand.errors {
        let title = sub
            .components
            .get(err.comp_id as usize)
            .and_then(|c| first_text(c, &ICalendarProperty::Summary))
            .unwrap_or_else(|| UNTITLED.to_string());
        warnings.push(format!(
            "Termin übersprungen ({}): {}",
            error_reason(&err.error),
            cap_chars(&title, 80)
        ));
    }
    for e in &expand.events {
        let Some(comp) = sub.components.get(e.comp_id as usize) else {
            continue;
        };
        if comp.component_type != ICalendarComponentType::VEvent {
            continue;
        }
        let Some(uid) = uid_of(e.comp_id) else {
            continue;
        };
        let start_ms = e.start.timestamp_millis();
        let end_ms = match &e.end {
            TimeOrDelta::Time(t) => t.timestamp_millis(),
            TimeOrDelta::Delta(d) => start_ms.saturating_add(d.num_milliseconds()),
        }
        .max(start_ms);
        // Beruehrt der Termin das Fenster? Ein Termin ohne Dauer zaehlt bei
        // Beginn im Fenster.
        if !(start_ms < to_ms && (end_ms > from_ms || start_ms >= from_ms)) {
            continue;
        }

        let own = read_fields(comp);
        let fields = match (master, is_override(comp)) {
            (Some(m), true) => own.inherit_from(m),
            _ => own,
        };
        let all_day = dtstart_is_all_day(comp).unwrap_or(false);
        let mut truncated = false;
        let attendees = build_attendees(&fields, &mut truncated);
        if truncated {
            warnings.push(format!(
                "Teilnehmerliste gekürzt (mehr als {MAX_ATTENDEES}): {}",
                cap_chars(fields.summary.as_deref().unwrap_or(UNTITLED), 80)
            ));
        }
        let join_url = join_url_of(&fields);
        out.push(CalEvent {
            key: event_key(source_id, &uid, start_ms),
            source_id: source_id.to_string(),
            uid,
            title: cap_chars(
                fields.summary.as_deref().unwrap_or(UNTITLED),
                MAX_TITLE_CHARS,
            ),
            starts_at: start_ms,
            ends_at: end_ms,
            all_day,
            cancelled: fields.cancelled.unwrap_or(false),
            location: fields
                .location
                .as_deref()
                .map(|l| cap_chars(l, MAX_LOCATION_CHARS)),
            join_url,
            description: fields
                .description
                .as_deref()
                .map(|d| cap_chars(d, MAX_DESCRIPTION_CHARS)),
            attendees,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const BERLIN: &str = "W. Europe Standard Time";

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/calendar")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn iso(ms: i64) -> String {
        Utc.timestamp_millis_opt(ms)
            .unwrap()
            .format("%Y-%m-%dT%H:%MZ")
            .to_string()
    }

    fn expand(raw: &str, from: i64, to: i64, tz: &str) -> IcsResult {
        parse_and_expand(raw, "src1", from, to, tz).expect("Kalender lesbar")
    }

    /// Fenster der Fixture-Tabellen: 28.09. bis 08.11.2026 (UTC), wie der CLI-Aufruf.
    fn window(raw: &str, tz: &str) -> IcsResult {
        expand(raw, ms(2026, 9, 28, 0, 0), ms(2026, 11, 8, 0, 0), tz)
    }

    fn who(a: &Attendee) -> String {
        format!(
            "{}<{}>{}{}",
            a.name.as_deref().unwrap_or("-"),
            a.email.as_deref().unwrap_or("-"),
            if a.organizer { "*" } else { "" },
            a.partstat
                .as_deref()
                .map(|p| format!("[{p}]"))
                .unwrap_or_default()
        )
    }

    /// Eine Tabellenzeile je Termin: Zeiten in UTC, Titel, Merker, Teilnehmende.
    fn row(e: &CalEvent) -> String {
        format!(
            "{}..{} {}{}{} att=[{}] url={}",
            iso(e.starts_at),
            &iso(e.ends_at)[11..],
            e.title,
            if e.cancelled { " (abgesagt)" } else { "" },
            if e.all_day { " (ganztägig)" } else { "" },
            e.attendees.iter().map(who).collect::<Vec<_>>().join("; "),
            e.join_url.as_deref().unwrap_or("-")
        )
    }

    fn rows(r: &IcsResult) -> Vec<String> {
        r.events.iter().map(row).collect()
    }

    fn titles_at(r: &IcsResult, title: &str) -> Vec<String> {
        r.events
            .iter()
            .filter(|e| e.title == title)
            .map(|e| iso(e.starts_at))
            .collect()
    }

    const ANNA: &str = "Anna Berg<anna.berg@example.com>*";
    const PATRICK: &str = "Patrick Wolff<patrick@example.com>[ACCEPTED]";
    const JONAS: &str = "Jonas Kurz<jonas.kurz@kunde.example>[TENTATIVE]";
    const TEAMS: &str =
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting_SYNTHETIC%40thread.v2/0";
    const ZOOM: &str = "https://us02web.zoom.us/j/81234567890?pwd=SYNTHETIC";

    fn jour_fixe_att() -> String {
        format!("{ANNA}; {PATRICK}; {JONAS}")
    }

    // ---------------------------------------------------------------------
    // Fixture-Tabellen (AK9 Teil 1)
    // ---------------------------------------------------------------------

    #[test]
    fn outlook_series_matches_the_fixture_table() {
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        let att = jour_fixe_att();
        let expected = vec![
            format!("2026-09-28T08:00Z..08:30Z Jour fixe Vertrieb att=[{att}] url={TEAMS}"),
            format!(
                "2026-10-05T12:00Z..12:30Z Jour fixe Vertrieb (verschoben) att=[{att}] url={TEAMS}"
            ),
            format!("2026-10-15T13:00Z..14:00Z Sprint Review att=[Mara Lind<mara.lind@example.com>*; {PATRICK}] url={ZOOM}"),
            format!(
                "2026-10-19T08:00Z..08:30Z Abgesagt: Jour fixe Vertrieb (abgesagt) att=[{att}] url={TEAMS}"
            ),
            format!("2026-10-26T09:00Z..09:30Z Jour fixe Vertrieb att=[{att}] url={TEAMS}"),
            "2026-10-27T10:00Z..11:00Z Angebotsbesprechung att=[Mara Lind<mara.lind@example.com>*] url=-".to_string(),
            format!("2026-10-29T14:00Z..15:00Z Sprint Review att=[Mara Lind<mara.lind@example.com>*; {PATRICK}] url={ZOOM}"),
            format!("2026-11-02T09:00Z..09:30Z Jour fixe Vertrieb att=[{att}] url={TEAMS}"),
        ];
        assert_eq!(rows(&r), expected);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.has_attendee_data);
        // Der Beschreibungstext ist entschluesselt (\n, \,).
        let d = r.events[0].description.as_deref().unwrap();
        assert_eq!(d, "Agenda\nPunkt 1, Punkt 2");
        assert_eq!(r.events[5].location.as_deref(), Some("Raum 2.14"));
        // Schluessel: Quelle, UID, Beginn.
        assert_eq!(
            r.events[0].key,
            format!(
                "src1:040000008200E00074C5B7101A82E008-jourfixe:{}",
                ms(2026, 9, 28, 8, 0)
            )
        );
    }

    #[test]
    fn google_fixture_matches_the_table() {
        let r = window(&fixture("google_secret.ics"), BERLIN);
        let meet = "https://meet.google.com/abc-defg-hij";
        let expected = vec![
            "2026-09-28T08:00Z..08:30Z Wochenplanung att=[] url=https://meet.google.com/wpl-anun-gxx".to_string(),
            "2026-10-02T12:00Z..13:00Z Kickoff Lieferantenprojekt (abgesagt) att=[Ben Falk<ben.falk@example.net>[DECLINED]] url=-".to_string(),
            "2026-10-12T08:00Z..08:30Z Wochenplanung att=[] url=https://meet.google.com/wpl-anun-gxx".to_string(),
            "2026-10-19T08:00Z..08:30Z Wochenplanung att=[] url=https://meet.google.com/wpl-anun-gxx".to_string(),
            // 26.10.: Sommerzeit ist seit dem 25.10. vorbei, 09:00 Berlin = 08:00Z.
            format!("2026-10-26T08:00Z..09:00Z Kundentermin Nordlicht att=[Lea Sommer<lea.sommer@example.org>*[NEEDS-ACTION]; -<jonas.kurz@kunde.example>[ACCEPTED]] url={meet}"),
            "2026-10-26T08:00Z..08:30Z Wochenplanung att=[] url=https://meet.google.com/wpl-anun-gxx".to_string(),
            "2026-11-02T08:00Z..08:30Z Wochenplanung att=[] url=https://meet.google.com/wpl-anun-gxx".to_string(),
        ];
        // Beide Termine des 26.10. beginnen um 08:00Z: die Reihenfolge ist nach UID stabil.
        let mut got = rows(&r);
        let mut want = expected;
        got.sort();
        want.sort();
        assert_eq!(got, want);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.has_attendee_data);
    }

    #[test]
    fn google_cn_equal_to_the_address_is_not_a_name() {
        let r = window(&fixture("google_secret.ics"), BERLIN);
        let e = r
            .events
            .iter()
            .find(|e| e.title == "Kundentermin Nordlicht")
            .unwrap();
        let jonas = e
            .attendees
            .iter()
            .find(|a| a.email.as_deref() == Some("jonas.kurz@kunde.example"))
            .unwrap();
        assert_eq!(jonas.name, None);
    }

    // ---------------------------------------------------------------------
    // V8: UTC-Serien
    // ---------------------------------------------------------------------

    const UTC_WEEKLY: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:w-utc\nSUMMARY:Weekly UTC\nDTSTART:20260105T080000Z\nDTEND:20260105T090000Z\nRRULE:FREQ=WEEKLY;COUNT=60\nEND:VEVENT\nEND:VCALENDAR\n";

    #[test]
    fn utc_dtstart_series_not_shifted() {
        // Standardzone Berlin: ohne Umgehung kaeme 06:00Z heraus.
        let r = expand(
            UTC_WEEKLY,
            ms(2026, 9, 28, 0, 0),
            ms(2026, 10, 13, 0, 0),
            BERLIN,
        );
        let starts: Vec<String> = r.events.iter().map(|e| iso(e.starts_at)).collect();
        assert_eq!(
            starts,
            [
                "2026-09-28T08:00Z",
                "2026-10-05T08:00Z",
                "2026-10-12T08:00Z"
            ]
        );
        assert_eq!(r.events[0].ends_at - r.events[0].starts_at, 3_600_000);
        // Dasselbe mit einer Zone weit weg von UTC.
        let r = expand(
            UTC_WEEKLY,
            ms(2026, 9, 28, 0, 0),
            ms(2026, 10, 13, 0, 0),
            "Pacific Standard Time",
        );
        assert_eq!(iso(r.events[0].starts_at), "2026-09-28T08:00Z");
    }

    #[test]
    fn calcard_shifts_utc_series_without_the_workaround() {
        // Belegt, WARUM `normalize_utc_rrule_starts` noetig ist: faellt dieser Test
        // nach einem calcard-Update, kann die Umgehung entfallen.
        let text = UTC_WEEKLY.replace('\n', "\r\n");
        let Entry::ICalendar(cal) = Parser::new(&text).entry() else {
            panic!("kein Kalender")
        };
        let tz = Tz::from_str(BERLIN).unwrap();
        let exp = cal.expand_dates(tz, 100);
        let first = exp
            .events
            .iter()
            .map(|e| e.start.timestamp_millis())
            .min()
            .unwrap();
        assert_eq!(
            iso(first),
            "2026-01-05T07:00Z",
            "calcard 0.3.14 verschiebt um den Ortsversatz (Winter: 1 h)"
        );
    }

    #[test]
    fn utc_series_override_still_applies() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:w-utc\nSUMMARY:Weekly UTC\nDTSTART:20260105T080000Z\nDTEND:20260105T090000Z\nRRULE:FREQ=WEEKLY;COUNT=60\nEND:VEVENT\nBEGIN:VEVENT\nUID:w-utc\nRECURRENCE-ID:20261005T080000Z\nSUMMARY:Weekly UTC (später)\nDTSTART:20261005T100000Z\nDTEND:20261005T110000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 9, 28, 0, 0), ms(2026, 10, 13, 0, 0), BERLIN);
        let got: Vec<(String, String)> = r
            .events
            .iter()
            .map(|e| (iso(e.starts_at), e.title.clone()))
            .collect();
        assert_eq!(
            got,
            [
                ("2026-09-28T08:00Z".to_string(), "Weekly UTC".to_string()),
                (
                    "2026-10-05T10:00Z".to_string(),
                    "Weekly UTC (später)".to_string()
                ),
                ("2026-10-12T08:00Z".to_string(), "Weekly UTC".to_string()),
            ]
        );
    }

    #[test]
    fn normalize_rewrites_only_series_with_a_utc_start() {
        // Outlook-Fixture: Serien mit TZID, nichts zu tun: derselbe Text, ohne Kopie.
        let outlook = fixture("outlook_series.ics");
        assert!(matches!(
            normalize_utc_rrule_starts(&outlook),
            Cow::Borrowed(_)
        ));
        // Google-Fixture: die UTC-Serie wird umgeschrieben, der Einzeltermin mit Z nicht.
        let google = fixture("google_secret.ics");
        let out = normalize_utc_rrule_starts(&google);
        assert!(matches!(out, Cow::Owned(_)));
        assert!(out.contains("DTSTART;TZID=UTC:20260105T080000\n"), "{out}");
        assert!(out.contains("DTEND;TZID=UTC:20260105T083000\n"));
        assert!(
            out.contains("EXDATE:20261005T080000Z\n"),
            "EXDATE bleibt in Z"
        );
        assert!(
            out.contains("DTSTART:20261002T120000Z\n"),
            "Einzeltermin ohne RRULE bleibt"
        );
        // Genau zwei Zeilen unterscheiden sich.
        let changed = google
            .lines()
            .zip(out.lines())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(changed, 2);
        assert_eq!(google.lines().count(), out.lines().count());
    }

    #[test]
    fn normalize_keeps_line_endings_and_existing_parameters() {
        let raw = "BEGIN:VEVENT\r\nDTSTART;VALUE=DATE-TIME:20260105T080000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n";
        let out = normalize_utc_rrule_starts(raw);
        assert_eq!(
            out,
            "BEGIN:VEVENT\r\nDTSTART;TZID=UTC;VALUE=DATE-TIME:20260105T080000\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n"
        );
        // Schon mit Zone: nicht anfassen. Ohne RRULE: nicht anfassen.
        for untouched in [
            "BEGIN:VEVENT\nDTSTART;TZID=Europe/Berlin:20260105T080000\nRRULE:FREQ=DAILY\nEND:VEVENT\n",
            "BEGIN:VEVENT\nDTSTART:20260105T080000Z\nEND:VEVENT\n",
            "BEGIN:VEVENT\nDTSTART:20260105T080000\nRRULE:FREQ=DAILY\nEND:VEVENT\n",
        ] {
            assert!(matches!(
                normalize_utc_rrule_starts(untouched),
                Cow::Borrowed(_)
            ));
        }
    }

    // ---------------------------------------------------------------------
    // Serien: EXDATE, Overrides, Sommerzeit
    // ---------------------------------------------------------------------

    #[test]
    fn exdate_removes_instance() {
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        let starts = titles_at(&r, "Jour fixe Vertrieb");
        assert_eq!(
            starts,
            [
                "2026-09-28T08:00Z",
                "2026-10-26T09:00Z",
                "2026-11-02T09:00Z"
            ]
        );
        assert!(
            !r.events
                .iter()
                .any(|e| iso(e.starts_at).starts_with("2026-10-12")),
            "12.10. steht in EXDATE"
        );
        // Auch die UTC-Serie mit EXDATE in Z.
        let g = window(&fixture("google_secret.ics"), BERLIN);
        assert!(!titles_at(&g, "Wochenplanung").contains(&"2026-10-05T08:00Z".to_string()));
        assert_eq!(titles_at(&g, "Wochenplanung").len(), 5);
    }

    #[test]
    fn override_moves_and_inherits_attendees() {
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        // Der Override steht am neuen Zeitpunkt, die Originalinstanz ist weg.
        let moved: Vec<&CalEvent> = r
            .events
            .iter()
            .filter(|e| e.title == "Jour fixe Vertrieb (verschoben)")
            .collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(iso(moved[0].starts_at), "2026-10-05T12:00Z");
        assert!(!r
            .events
            .iter()
            .any(|e| iso(e.starts_at) == "2026-10-05T08:00Z"));
        // V7: ohne eigene Zeilen erbt er Teilnehmende, Organisator und Teams-Adresse.
        let att: Vec<String> = moved[0].attendees.iter().map(who).collect();
        assert_eq!(att, [ANNA, PATRICK, JONAS]);
        assert_eq!(moved[0].join_url.as_deref(), Some(TEAMS));
        assert_eq!(
            moved[0].location.as_deref(),
            Some("Microsoft Teams-Besprechung")
        );
        assert_eq!(
            moved[0].description.as_deref(),
            Some("Agenda\nPunkt 1, Punkt 2")
        );
        // Die Schluessel beider Instanzen unterscheiden sich (Beginn).
        assert_eq!(moved[0].uid, r.events[0].uid);
        assert_ne!(moved[0].key, r.events[0].key);
    }

    #[test]
    fn override_with_its_own_attendees_keeps_them() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:s1\nSUMMARY:Serie\nDTSTART:20261005T080000Z\nDTEND:20261005T090000Z\nRRULE:FREQ=WEEKLY;COUNT=3\nATTENDEE;CN=Alt:mailto:alt@example.com\nEND:VEVENT\nBEGIN:VEVENT\nUID:s1\nRECURRENCE-ID:20261012T080000Z\nSUMMARY:Serie\nDTSTART:20261012T080000Z\nDTEND:20261012T090000Z\nATTENDEE;CN=Neu:mailto:neu@example.com\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        let by_start = |day: &str| {
            r.events
                .iter()
                .find(|e| iso(e.starts_at).starts_with(day))
                .unwrap()
                .attendees
                .iter()
                .map(who)
                .collect::<Vec<_>>()
        };
        assert_eq!(by_start("2026-10-05"), ["Alt<alt@example.com>"]);
        assert_eq!(by_start("2026-10-12"), ["Neu<neu@example.com>"]);
        assert_eq!(by_start("2026-10-19"), ["Alt<alt@example.com>"]);
    }

    #[test]
    fn cancelled_override_flagged() {
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        let cancelled: Vec<&CalEvent> = r.events.iter().filter(|e| e.cancelled).collect();
        assert_eq!(cancelled.len(), 1, "genau die abgesagte Instanz");
        assert_eq!(iso(cancelled[0].starts_at), "2026-10-19T08:00Z");
        assert_eq!(cancelled[0].title, "Abgesagt: Jour fixe Vertrieb");
        // Sie bleibt im Ergebnis (Cache), die anderen sind nicht abgesagt.
        assert_eq!(r.events.iter().filter(|e| !e.cancelled).count(), 7);
        // Auch ein abgesagter Einzeltermin (Google).
        let g = window(&fixture("google_secret.ics"), BERLIN);
        assert!(g
            .events
            .iter()
            .any(|e| e.uid == "kickoff-abgesagt@google.com" && e.cancelled));
    }

    #[test]
    fn override_with_a_different_sequence_replaces_the_instance() {
        // Outlook zaehlt SEQUENCE je Aenderung hoch (Master 0, Overrides 3 und 5,
        // siehe Fixture). calcard ordnet einen Override nur bei gleicher SEQUENCE
        // zu: ohne unsere Umgehung stuende die Originalinstanz zusaetzlich da.
        let raw = fixture("outlook_series.ics").replace('\n', "\r\n");
        let Entry::ICalendar(cal) = Parser::new(&raw).entry() else {
            panic!()
        };
        let plain = cal.expand_dates(Tz::from_str(BERLIN).unwrap(), 20_000);
        let at_original_slot = plain
            .events
            .iter()
            .filter(|e| iso(e.start.timestamp_millis()) == "2026-10-05T08:00Z")
            .count();
        assert_eq!(
            at_original_slot, 1,
            "calcard 0.3.14 laesst die Originalinstanz stehen"
        );
        // Unser Weg: genau eine Instanz an diesem Tag, und zwar die verschobene.
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        let on_5th: Vec<String> = r
            .events
            .iter()
            .filter(|e| iso(e.starts_at).starts_with("2026-10-05"))
            .map(|e| e.title.clone())
            .collect();
        assert_eq!(on_5th, ["Jour fixe Vertrieb (verschoben)"]);
    }

    #[test]
    fn windows_tzid_dst_switch() {
        // 10:00 Berlin: vor dem 25.10. Sommerzeit (08:00Z), danach Winterzeit (09:00Z).
        let r = window(&fixture("outlook_series.ics"), BERLIN);
        let jf: Vec<String> = titles_at(&r, "Jour fixe Vertrieb");
        assert_eq!(jf[0], "2026-09-28T08:00Z");
        assert_eq!(jf[1], "2026-10-26T09:00Z");
        assert_eq!(jf[2], "2026-11-02T09:00Z");
        // 15:00 Berlin: der 15.10. noch Sommer, der 29.10. Winter.
        assert_eq!(
            titles_at(&r, "Sprint Review"),
            ["2026-10-15T13:00Z", "2026-10-29T14:00Z"]
        );
        // Die Dauer bleibt ueber den Wechsel gleich.
        assert!(r
            .events
            .iter()
            .filter(|e| e.title == "Jour fixe Vertrieb")
            .all(|e| e.ends_at - e.starts_at == 30 * 60_000));
        // Die Standardzone spielt hier keine Rolle: der Termin traegt seine Zone.
        let utc = window(&fixture("outlook_series.ics"), "UTC");
        assert_eq!(
            titles_at(&utc, "Jour fixe Vertrieb"),
            titles_at(&r, "Jour fixe Vertrieb")
        );
    }

    // ---------------------------------------------------------------------
    // V9: Limit je Serie, gekuerzte Serien
    // ---------------------------------------------------------------------

    fn series_calendar(parts: &[(&str, &str, &str)]) -> String {
        // (uid, dtstart, rrule)
        let mut s = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
        for (uid, start, rule) in parts {
            s.push_str(&format!(
                "BEGIN:VEVENT\nUID:{uid}\nSUMMARY:{uid}\nDTSTART;TZID=UTC:{start}\nDTEND;TZID=UTC:{}\nRRULE:{rule}\nEND:VEVENT\n",
                start.replace("T08", "T09")
            ));
        }
        s.push_str("END:VCALENDAR\n");
        s
    }

    #[test]
    fn daily_series_does_not_starve_weekly() {
        // Zwei unbegrenzte Tagesserien seit 2000 (je 20 000 Instanzen bis 2054) und
        // eine Wochenserie seit 2026: mit einem gemeinsamen Limit bekaeme sie nichts.
        let raw = series_calendar(&[
            ("daily-a", "20000101T080000", "FREQ=DAILY"),
            ("daily-b", "20000101T080000", "FREQ=DAILY"),
            ("weekly", "20260105T080000", "FREQ=WEEKLY;BYDAY=MO"),
        ]);
        let r = expand(&raw, ms(2026, 9, 28, 0, 0), ms(2026, 11, 8, 0, 0), "UTC");
        let count = |uid: &str| r.events.iter().filter(|e| e.uid == uid).count();
        assert_eq!(count("weekly"), 6, "Montage 28.9. bis 2.11.");
        assert_eq!(count("daily-a"), 41);
        assert_eq!(count("daily-b"), 41);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn old_daily_series_is_truncated_with_warning() {
        // Seit 1960 taeglich: die 20 000 Instanzen enden 2014, weit vor dem Fenster.
        let raw = series_calendar(&[
            ("uralt", "19600101T080000", "FREQ=DAILY"),
            ("weekly", "20260105T080000", "FREQ=WEEKLY;BYDAY=MO"),
        ]);
        let r = expand(&raw, ms(2026, 9, 28, 0, 0), ms(2026, 11, 8, 0, 0), "UTC");
        assert!(
            r.events.iter().all(|e| e.uid != "uralt"),
            "keine falschen Instanzen"
        );
        assert!(
            r.warnings
                .iter()
                .any(|w| w.contains("Serie gekürzt") && w.contains("uralt")),
            "{:?}",
            r.warnings
        );
        // Die andere Serie ist davon nicht betroffen.
        assert_eq!(r.events.iter().filter(|e| e.uid == "weekly").count(), 6);
    }

    #[test]
    fn big_calendar_keeps_every_event_and_stays_fast() {
        // 3 000 Einzeltermine (63 davon im Fenster) + zwei unbegrenzte Serien.
        let mut raw = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
        let base = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let mut expected_in_window = 0;
        let (from, to) = (ms(2026, 9, 28, 0, 0), ms(2026, 11, 8, 0, 0));
        for i in 0..3000i64 {
            let start = base + chrono::Duration::days(i / 3 * 2) + chrono::Duration::hours(i % 3);
            let end = start + chrono::Duration::hours(1);
            if start.timestamp_millis() >= from && start.timestamp_millis() < to {
                expected_in_window += 1;
            }
            raw.push_str(&format!(
                "BEGIN:VEVENT\nUID:e{i}\nSUMMARY:Termin {i}\nDTSTART:{}\nDTEND:{}\nATTENDEE;CN=Person {i}:mailto:p{i}@example.com\nDESCRIPTION:{}\nEND:VEVENT\n",
                start.format("%Y%m%dT%H%M%SZ"),
                end.format("%Y%m%dT%H%M%SZ"),
                "Lorem ipsum dolor sit amet ".repeat(30)
            ));
        }
        raw.push_str("BEGIN:VEVENT\nUID:tag\nSUMMARY:Tagesserie\nDTSTART;TZID=UTC:20000101T060000\nDTEND;TZID=UTC:20000101T063000\nRRULE:FREQ=DAILY\nEND:VEVENT\n");
        raw.push_str("BEGIN:VEVENT\nUID:woche\nSUMMARY:Wochenserie\nDTSTART;TZID=UTC:20260105T060000\nDTEND;TZID=UTC:20260105T063000\nRRULE:FREQ=WEEKLY;BYDAY=TU\nEND:VEVENT\nEND:VCALENDAR\n");
        let started = std::time::Instant::now();
        let r = expand(&raw, from, to, BERLIN);
        let elapsed = started.elapsed();
        let singles = r.events.iter().filter(|e| e.uid.starts_with('e')).count();
        assert_eq!(singles, expected_in_window);
        assert_eq!(r.events.iter().filter(|e| e.uid == "tag").count(), 41);
        assert_eq!(r.events.iter().filter(|e| e.uid == "woche").count(), 6);
        // Debug-Build: grosszuegige Grenze, die nur eine quadratische Laufzeit reisst.
        assert!(elapsed.as_secs() < 30, "{elapsed:?}");
    }

    // ---------------------------------------------------------------------
    // Ganztaegig, schwebend, Fenster
    // ---------------------------------------------------------------------

    #[test]
    fn all_day_flagged() {
        let r = window(&fixture("allday_floating.ics"), BERLIN);
        let flags: Vec<(String, bool, String)> = r
            .events
            .iter()
            .map(|e| (e.title.clone(), e.all_day, iso(e.starts_at)))
            .collect();
        assert_eq!(
            flags,
            [
                // 01.10. 09:00 ohne Zone = Ortszeit Berlin (Sommer) = 07:00Z.
                (
                    "Lokaler Termin ohne Zone".to_string(),
                    false,
                    "2026-10-01T07:00Z".to_string()
                ),
                // Ganztaegige Termine beginnen um Mitternacht der Standardzone.
                (
                    "Homeoffice".to_string(),
                    true,
                    "2026-10-04T22:00Z".to_string()
                ),
                (
                    "Homeoffice".to_string(),
                    true,
                    "2026-10-11T22:00Z".to_string()
                ),
                (
                    "Messe Hannover".to_string(),
                    true,
                    "2026-10-11T22:00Z".to_string()
                ),
                (
                    "Homeoffice".to_string(),
                    true,
                    "2026-10-18T22:00Z".to_string()
                ),
                // 30.10.: die Sommerzeit ist vorbei, Mitternacht Berlin = 23:00Z am Vortag.
                ("Urlaub".to_string(), true, "2026-10-29T23:00Z".to_string()),
            ]
        );
        // Mehrtaegig: das Ende (exklusiv) ist Mitternacht des 16.10. in Berlin.
        let messe = r
            .events
            .iter()
            .find(|e| e.title == "Messe Hannover")
            .unwrap();
        assert_eq!(iso(messe.ends_at), "2026-10-15T22:00Z");
        assert!(!r.has_attendee_data);
    }

    #[test]
    fn floating_uses_default_zone() {
        let raw = fixture("allday_floating.ics");
        let at = |tz: &str| {
            let r = window(&raw, tz);
            let e = r
                .events
                .iter()
                .find(|e| e.title == "Lokaler Termin ohne Zone")
                .unwrap();
            iso(e.starts_at)
        };
        assert_eq!(at(BERLIN), "2026-10-01T07:00Z");
        assert_eq!(at("UTC"), "2026-10-01T09:00Z");
        assert_eq!(at("Pacific Standard Time"), "2026-10-01T16:00Z");
    }

    fn single(uid: &str, start: &str, end: &str) -> String {
        format!(
            "BEGIN:VEVENT\nUID:{uid}\nSUMMARY:{uid}\nDTSTART:{start}\nDTEND:{end}\nEND:VEVENT\n"
        )
    }

    #[test]
    fn window_filters_events() {
        let mut raw = String::from("BEGIN:VCALENDAR\nVERSION:2.0\n");
        // Fenster: 10.10.2026 00:00Z bis 20.10.2026 00:00Z.
        raw += &single("endet-am-anfang", "20261009T230000Z", "20261010T000000Z");
        raw += &single("ragt-hinein", "20261009T230000Z", "20261010T010000Z");
        raw += &single(
            "ohne-dauer-am-anfang",
            "20261010T000000Z",
            "20261010T000000Z",
        );
        raw += &single("mittendrin", "20261015T100000Z", "20261015T110000Z");
        raw += &single("ragt-heraus", "20261019T230000Z", "20261020T010000Z");
        raw += &single("beginnt-am-ende", "20261020T000000Z", "20261020T010000Z");
        raw += &single("davor", "20261001T100000Z", "20261001T110000Z");
        raw += &single("danach", "20261101T100000Z", "20261101T110000Z");
        raw += "END:VCALENDAR\n";
        let r = expand(&raw, ms(2026, 10, 10, 0, 0), ms(2026, 10, 20, 0, 0), "UTC");
        let uids: Vec<&str> = r.events.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(
            uids,
            [
                "ragt-hinein",
                "ohne-dauer-am-anfang",
                "mittendrin",
                "ragt-heraus"
            ]
        );
    }

    // ---------------------------------------------------------------------
    // Kaputtes, Dubletten, Grenzen
    // ---------------------------------------------------------------------

    #[test]
    fn html_response_is_not_calendar() {
        let html = "<!DOCTYPE html><html><head><title>Anmelden</title></head><body>BEGIN:VCALENDAR</body></html>";
        assert!(!looks_like_calendar(html));
        assert!(!looks_like_calendar(""));
        assert!(!looks_like_calendar("   \n  "));
        assert!(!looks_like_calendar("BEGIN:VCARD\nEND:VCARD"));
        assert_eq!(
            parse_and_expand(html, "s", 0, i64::MAX, "UTC"),
            Err(CalendarError::NotCalendar)
        );
        assert_eq!(
            parse_and_expand("", "s", 0, i64::MAX, "UTC"),
            Err(CalendarError::NotCalendar)
        );
        // Mit BOM, Leerraum und kleinen Buchstaben ist es einer.
        assert!(looks_like_calendar("\u{feff}\r\n  begin:vcalendar\r\n"));
        assert!(looks_like_calendar("BEGIN:VCALENDAR"));
    }

    #[test]
    fn broken_components_are_skipped_with_warnings() {
        let r = window(&fixture("broken.ics"), BERLIN);
        let got: Vec<(String, String)> = r
            .events
            .iter()
            .map(|e| (iso(e.starts_at), e.title.clone()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "2026-10-01T08:00Z".to_string(),
                    "Gültiger Termin vor der Störung".to_string()
                ),
                // 10:00 Ortszeit Berlin (Sommer): die unbekannte Zone faellt auf die Standardzone.
                (
                    "2026-10-02T08:00Z".to_string(),
                    "Unbekannte Zeitzone".to_string()
                ),
                // Die Regel ist unlesbar: nur der erste Termin, mit Hinweis (siehe unten).
                ("2026-10-03T08:00Z".to_string(), "Kaputte Regel".to_string()),
                (
                    "2026-10-04T08:00Z".to_string(),
                    "Offene Komponente am Ende".to_string()
                ),
            ],
            "{:?}",
            r.warnings
        );
        let w = r.warnings.join(" | ");
        assert!(w.contains("Ohne Beginn"), "{w}");
        assert!(w.contains("Kaputtes Datum"), "{w}");
        assert!(w.contains("Kaputte Regel"), "{w}");
        assert!(w.contains("Mars/Olympus_Mons"), "{w}");
    }

    #[test]
    fn unknown_tzid_uses_default_zone_and_warns() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:z1\nSUMMARY:Zone\nDTSTART;TZID=Nirgendwo Standard Time:20261002T100000\nDTEND;TZID=Nirgendwo Standard Time:20261002T110000\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = window(raw, BERLIN);
        assert_eq!(iso(r.events[0].starts_at), "2026-10-02T08:00Z");
        assert_eq!(
            r.warnings
                .iter()
                .filter(|w| w.contains("Nirgendwo"))
                .count(),
            1,
            "{:?}",
            r.warnings
        );
        // Eine unbekannte Standardzone: UTC, mit Hinweis.
        let r = window(raw, "Mond Standard Time");
        assert_eq!(iso(r.events[0].starts_at), "2026-10-02T10:00Z");
        assert!(r.warnings.iter().any(|w| w.contains("Mond Standard Time")));
    }

    #[test]
    fn a_nonexistent_local_time_does_not_bring_the_run_down() {
        // 29.03.2026 02:30 gibt es in Berlin nicht (Sprung auf Sommerzeit).
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:l1\nSUMMARY:Luecke\nDTSTART;TZID=Europe/Berlin:20260329T023000\nDTEND;TZID=Europe/Berlin:20260329T033000\nEND:VEVENT\nBEGIN:VEVENT\nUID:l2\nSUMMARY:Danach\nDTSTART;TZID=Europe/Berlin:20260329T120000\nDTEND;TZID=Europe/Berlin:20260329T130000\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 3, 1, 0, 0), ms(2026, 4, 30, 0, 0), BERLIN);
        assert!(r.events.iter().any(|e| e.title == "Danach"));
        assert!(
            r.events.iter().any(|e| e.title == "Luecke")
                || r.warnings.iter().any(|w| w.contains("Luecke")),
            "die Luecke wird entweder gerechnet oder gemeldet, nie still verschluckt"
        );
    }

    #[test]
    fn duplicate_uid_and_start_keep_the_later_entry() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:d1\nSUMMARY:Alt\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nEND:VEVENT\nBEGIN:VEVENT\nUID:d1\nSUMMARY:Neu\nDTSTART:20261015T100000Z\nDTEND:20261015T113000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert_eq!(r.events.len(), 1);
        assert_eq!(r.events[0].title, "Neu");
        assert_eq!(r.events[0].ends_at - r.events[0].starts_at, 90 * 60_000);
    }

    #[test]
    fn attendees_are_normalised_and_merged() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:a1\nSUMMARY:Runde\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nORGANIZER;CN=\"Wolff, Patrick\":MAILTO:Patrick.Wolff@Example.COM\nATTENDEE;CN=Patrick Wolff;PARTSTAT=ACCEPTED:mailto:patrick.wolff@example.com\nATTENDEE;CN=Intern Person:X500:/O=EXCH/OU=X/CN=RECIPIENTS/CN=INTERN\nATTENDEE;CN=Intern Person:X500:/O=EXCH/OU=X/CN=RECIPIENTS/CN=INTERN\nATTENDEE:mailto:nur-adresse@example.org\nATTENDEE;CN=Leer:\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        let att: Vec<String> = r.events[0].attendees.iter().map(who).collect();
        assert_eq!(
            att,
            [
                "Wolff, Patrick<patrick.wolff@example.com>*[ACCEPTED]",
                "Intern Person<->",
                "-<nur-adresse@example.org>",
                "Leer<->",
            ]
        );
    }

    #[test]
    fn description_is_capped_and_a_missing_title_is_defaulted() {
        let long = "ä".repeat(10_000);
        let raw = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:t1\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nDESCRIPTION:{long}\nLOCATION:{}\nEND:VEVENT\nEND:VCALENDAR\n",
            "o".repeat(2_000)
        );
        let r = expand(&raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        let e = &r.events[0];
        assert_eq!(e.title, UNTITLED);
        assert_eq!(e.description.as_deref().unwrap().chars().count(), 4_000);
        assert_eq!(e.location.as_deref().unwrap().chars().count(), 500);
        assert!(r.events[0].join_url.is_none());
    }

    #[test]
    fn events_without_uid_get_a_stable_synthetic_uid() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nSUMMARY:Ohne UID\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let a = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        let b = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert!(a.events[0].uid.starts_with("noid-"));
        assert_eq!(a.events[0].key, b.events[0].key);
    }

    #[test]
    fn line_endings_do_not_change_the_result() {
        let lf = fixture("outlook_series.ics").replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let mixed = lf.replacen('\n', "\r\n", 3);
        let a = rows(&window(&lf, BERLIN));
        assert_eq!(a.len(), 8);
        assert_eq!(a, rows(&window(&crlf, BERLIN)));
        assert_eq!(a, rows(&window(&mixed, BERLIN)));
    }

    #[test]
    fn folded_lines_are_joined() {
        let raw = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:f1\r\nSUMMARY:Ein sehr langer\r\n  Titel ueber zwei Zeilen\r\nDTSTART:20261015T100000Z\r\nDTEND:20261015T110000Z\r\nDESCRIPTION:Einwahl https://meet.google.com/abc-\r\n defg-hij bitte\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert_eq!(r.events[0].title, "Ein sehr langer Titel ueber zwei Zeilen");
        assert_eq!(
            r.events[0].join_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn has_attendee_data_reflects_the_whole_file() {
        assert!(window(&fixture("outlook_series.ics"), BERLIN).has_attendee_data);
        assert!(!window(&fixture("allday_floating.ics"), BERLIN).has_attendee_data);
        // Auch wenn der Termin mit Teilnehmenden ausserhalb des Fensters liegt.
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:x\nSUMMARY:Alt\nDTSTART:20200101T100000Z\nDTEND:20200101T110000Z\nATTENDEE:mailto:a@example.com\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert!(r.events.is_empty());
        assert!(r.has_attendee_data);
    }

    // ---------------------------------------------------------------------
    // Beitritts-Adresse, Zeitzone des Systems
    // ---------------------------------------------------------------------

    #[test]
    fn join_url_extraction() {
        // Reihenfolge der Felder gewinnt, nicht das Muster.
        assert_eq!(
            extract_join_url(&[
                "kein Link",
                "Ort: https://meet.google.com/abc-defg-hij",
                "https://zoom.us/j/123"
            ])
            .as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        let cases = [
            ("Bitte hier: https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0?context=%7b%22Tid%22%7d.", "https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0?context=%7b%22Tid%22%7d"),
            ("<https://us02web.zoom.us/j/81234567890?pwd=abc>", "https://us02web.zoom.us/j/81234567890?pwd=abc"),
            ("Webex: https://firma.webex.com/meet/max.mustermann, danach", "https://firma.webex.com/meet/max.mustermann"),
            ("(https://meet.google.com/abc-defg-hij)", "https://meet.google.com/abc-defg-hij"),
            ("HTTPS://TEAMS.MICROSOFT.COM/L/MEETUP-JOIN/ABC", "HTTPS://TEAMS.MICROSOFT.COM/L/MEETUP-JOIN/ABC"),
            ("https://teams.live.com/meet/9876543210?p=x", "https://teams.live.com/meet/9876543210?p=x"),
        ];
        for (text, want) in cases {
            assert_eq!(extract_join_url(&[text]).as_deref(), Some(want), "{text}");
        }
        for none in [
            "",
            "https://example.com/meet",
            "https://meet.google.com/",
            "https://zoom.us/",
            "zoom.us/j/123 ohne Schema",
            "https://evilzoom.us/j/123",
        ] {
            assert_eq!(extract_join_url(&[none]), None, "{none}");
        }
    }

    #[test]
    fn the_explicit_teams_property_wins_over_the_text_fields() {
        let raw = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:j1\nSUMMARY:Link\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nX-MICROSOFT-SKYPETEAMSMEETINGURL:https://teams.microsoft.com/l/meetup-join/EIGENSCHAFT\nLOCATION:https://meet.google.com/abc-defg-hij\nEND:VEVENT\nEND:VCALENDAR\n";
        let r = expand(raw, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert_eq!(
            r.events[0].join_url.as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/EIGENSCHAFT")
        );
        // Ein Wert ohne http(s) taugt nicht als Adresse.
        let raw2 = raw.replace(
            "https://teams.microsoft.com/l/meetup-join/EIGENSCHAFT",
            "javascript:alert(1)",
        );
        let r2 = expand(&raw2, ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC");
        assert_eq!(
            r2.events[0].join_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_system_zone_of_this_machine_resolves() {
        let name = system_tz_name().expect("TimeZoneKeyName unter Windows");
        assert!(Tz::from_str(&name).is_ok(), "{name} unbekannt");
    }

    // ---------------------------------------------------------------------
    // Feindliche Eingaben: nie Absturz, nie Endlosschleife
    // ---------------------------------------------------------------------

    fn event_with(rule_and_props: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:h1\nSUMMARY:Feindlich\nDTSTART;TZID=UTC:20260105T080000\nDTEND;TZID=UTC:20260105T090000\n{rule_and_props}\nEND:VEVENT\nEND:VCALENDAR\n"
        )
    }

    /// Fuehrt `parse_and_expand` in einem Thread aus und verlangt ein Ergebnis
    /// binnen `secs` Sekunden; ein Absturz des Threads faellt ebenfalls auf.
    fn expand_within(raw: String, secs: u64) -> Result<IcsResult, CalendarError> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = parse_and_expand(
                &raw,
                "s",
                ms(2026, 9, 1, 0, 0),
                ms(2026, 12, 1, 0, 0),
                "UTC",
            );
            let _ = tx.send(r);
        });
        rx.recv_timeout(std::time::Duration::from_secs(secs))
            .expect("kein Ergebnis binnen der Frist: Absturz oder Endlosschleife")
    }

    #[test]
    fn hostile_recurrence_rules_terminate_without_a_crash() {
        let rules = [
            "RRULE:FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30",
            "RRULE:FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=31;BYDAY=MO",
            "RRULE:FREQ=MONTHLY;BYMONTHDAY=31;BYMONTH=4",
            "RRULE:FREQ=DAILY;INTERVAL=0",
            "RRULE:FREQ=DAILY;INTERVAL=-3",
            "RRULE:FREQ=SECONDLY;COUNT=99999999",
            "RRULE:FREQ=WEEKLY;BYDAY=XX,YY;COUNT=5",
            "RRULE:FREQ=MONTHLY;BYSETPOS=400;BYDAY=MO",
            "RRULE:FREQ=YEARLY;BYWEEKNO=60;BYDAY=MO",
            "RRULE:FREQ=YEARLY;BYYEARDAY=400",
            "RRULE:FREQ=DAILY;UNTIL=19000101T000000Z",
            "RRULE:FREQ=DAILY;COUNT=0",
            "RRULE:",
            "RRULE:FREQ=DAILY;COUNT=5\nRRULE:FREQ=WEEKLY",
            "RDATE:99990101T000000Z\nRDATE:00010101T000000Z\nRDATE:garbage",
            "EXDATE:garbage,20261001T000000Z",
            "DURATION:-P9999D",
            "DURATION:PT99999999H",
        ];
        for rule in rules {
            let started = std::time::Instant::now();
            let r = expand_within(event_with(rule), 20);
            assert!(r.is_ok(), "{rule}: {r:?}");
            assert!(
                started.elapsed().as_secs() < 15,
                "{rule}: {:?}",
                started.elapsed()
            );
        }
    }

    #[test]
    fn deeply_nested_components_are_refused_before_they_can_blow_the_stack() {
        // Tausende offene BEGIN:VEVENT liessen frueher die Rekursion beim Kopieren
        // den Stapel sprengen (Absturz des ganzen Prozesses, nicht abfangbar).
        let nested = format!(
            "BEGIN:VCALENDAR\n{}END:VCALENDAR\n",
            "BEGIN:VEVENT\n".repeat(5_000)
        );
        assert!(matches!(
            parse_and_expand(&nested, "s", 0, i64::MAX, "UTC"),
            Err(CalendarError::Parse(_))
        ));
        // Normale Tiefe (VCALENDAR > VEVENT > VALARM) bleibt erlaubt.
        let ok = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:a\nSUMMARY:Mit Alarm\nDTSTART:20261015T100000Z\nDTEND:20261015T110000Z\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n";
        let r =
            parse_and_expand(ok, "s", ms(2026, 10, 1, 0, 0), ms(2026, 11, 1, 0, 0), "UTC").unwrap();
        assert_eq!(r.events.len(), 1);
    }

    #[test]
    fn hostile_dates_and_structures_are_answered_not_crashed() {
        let cases: Vec<String> = vec![
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART:99999999T999999Z\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART:00000101T000000Z\nDTEND:99991231T235959Z\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART:20261001T100000Z\nDTEND:20200101T100000Z\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\nEND:VEVENT\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            format!("BEGIN:VCALENDAR\n{}END:VCALENDAR\n", "BEGIN:VEVENT\n".repeat(5_000)),
            format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nSUMMARY:{}\nDTSTART:20261001T100000Z\nEND:VEVENT\nEND:VCALENDAR\n", "A".repeat(1_000_000)),
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\u{0}y\nSUMMARY:Nul\u{0}Byte\nDTSTART:20261001T100000Z\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Falten\r\n \r\n \r\n \r\nDTSTART:20261001T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR".into(),
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART;TZID=:20261001T100000\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART;TZID=\"\":20261001T100000\nATTENDEE;CN=:mailto:\nORGANIZER:\nEND:VEVENT\nEND:VCALENDAR\n".into(),
            "\u{feff}BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:ü\nSUMMARY:Übergrößenänderung 🚀\nDTSTART:20261001T100000Z\nEND:VEVENT\nEND:VCALENDAR\n".into(),
        ];
        for (i, raw) in cases.into_iter().enumerate() {
            let r = expand_within(raw, 30);
            // Ok oder ein Fehler mit Klartext; wichtig ist: eine Antwort.
            match r {
                Ok(_) => {}
                Err(e) => assert!(!e.to_string().is_empty(), "Fall {i}"),
            }
        }
    }
}
