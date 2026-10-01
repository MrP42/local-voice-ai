//! C2 (Goal Lokaler Agent, AK4): Datumsangaben aus gesprochener Sprache -> ISO-Datum,
//! im Code statt im Modell.
//!
//! Der Eval von C1 (01.10.2026) hat gezeigt: Datum ist die schwaechste Klasse aller
//! gemessenen Modelle ("Freitag naechster Woche" falsch gerechnet, "uebermorgen" als
//! "fehlt" abgelehnt). Deshalb nennt das Modell bei einer Frist nur die Angabe, wie sie
//! gesagt wurde (`due_phrase`); das Datum rechnet dieses Modul mit dem Besprechungsdatum
//! als Bezug. Ein Datum, das das Modell selbst nennt, gilt nur, wenn der Code die Angabe
//! nicht aufloesen kann UND es die Pruefung ([`validate`]) besteht.
//!
//! Regeln (alle Wochen laufen Montag bis Sonntag):
//! - `heute`, `morgen`, `uebermorgen` (`gestern`, `vorgestern` ergeben ein Datum in der
//!   Vergangenheit, das die Pruefung verwirft);
//! - ein Wochentag allein (`Freitag`, `bis Freitag`, `naechsten Montag`): der naechste
//!   dieses Tages NACH dem Bezugsdatum (1 bis 7 Tage; am selben Wochentag also in
//!   einer Woche);
//! - Wochentag mit Woche (`Freitag naechster Woche`, `naechste Woche Dienstag`,
//!   `uebernaechste Woche`, `diese Woche`): dieser Tag in der Woche nach (vor-/nach-)
//!   dem Bezug;
//! - `Ende der Woche` = Freitag, `Anfang naechster Woche` = Montag;
//!   `Ende des Monats`, `Monatsende`, `letzter Tag dieses Monats`, `Ende naechsten
//!   Monats`, `Anfang naechsten Monats`; `Jahresende`, `Ende naechsten Jahres`;
//! - `in zwei Wochen`, `in 3 Tagen`, `in einem Monat` (Zahl oder Zahlwort);
//! - `KW 42` / `Kalenderwoche 42`: der Freitag dieser Woche;
//! - absolut: `2026-10-15`, `15.10.2026`, `15.10.26`, `15.10.`, `15. Oktober`,
//!   `15. Oktober 2026`. Ohne Jahr gilt das naechste Vorkommen ab dem Bezug.
//!
//! Mehrdeutiges ergibt `None`, nie eine Vermutung: zwei verschiedene Daten in einer
//! Angabe, ein Wochentag, der nicht zum genannten Datum passt, `naechste Woche` ohne
//! Tag, `Wochenende`, `zeitnah`. Wo ein Datum und ein relativer Ausdruck zugleich
//! stehen, muessen beide dasselbe meinen.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use once_cell::sync::Lazy;
use regex::Regex;

/// Weiter als so viele Jahre nach der Besprechung gilt eine Frist als unplausibel.
pub const MAX_YEARS_AHEAD: u32 = 2;

/// Warum ein Datum nicht gilt (steht im Vermerk des Laufs, deutsch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateIssue {
    /// Nicht in der Form JJJJ-MM-TT.
    Malformed,
    /// Die Form stimmt, den Tag gibt es nicht (2026-02-30).
    NotACalendarDate,
    /// Vor dem Besprechungsdatum.
    Past,
    /// Mehr als [`MAX_YEARS_AHEAD`] Jahre nach dem Besprechungsdatum.
    TooFar,
}

impl DateIssue {
    pub fn code(self) -> &'static str {
        match self {
            DateIssue::Malformed => "date_malformed",
            DateIssue::NotACalendarDate => "date_not_a_day",
            DateIssue::Past => "date_in_past",
            DateIssue::TooFar => "date_too_far",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            DateIssue::Malformed => "kein ISO-Datum (JJJJ-MM-TT)",
            DateIssue::NotACalendarDate => "den Tag gibt es im Kalender nicht",
            DateIssue::Past => "liegt vor dem Besprechungsdatum",
            DateIssue::TooFar => "liegt mehr als zwei Jahre nach dem Besprechungsdatum",
        }
    }
}

static ISO_STRICT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").expect("fester Ausdruck"));

/// `JJJJ-MM-TT` streng (keine einstelligen Teile, kein Leerraum, keine Uhrzeit).
pub fn parse_iso(text: &str) -> Result<NaiveDate, DateIssue> {
    let caps = ISO_STRICT
        .captures(text.trim())
        .ok_or(DateIssue::Malformed)?;
    let part = |i: usize| caps[i].parse::<u32>().map_err(|_| DateIssue::Malformed);
    NaiveDate::from_ymd_opt(part(1)? as i32, part(2)?, part(3)?).ok_or(DateIssue::NotACalendarDate)
}

/// Prueft ein Datum gegen das Besprechungsdatum: nicht davor, hoechstens
/// [`MAX_YEARS_AHEAD`] Jahre danach.
pub fn validate(date: NaiveDate, reference: NaiveDate) -> Result<NaiveDate, DateIssue> {
    if date < reference {
        return Err(DateIssue::Past);
    }
    let limit = reference
        .checked_add_months(Months::new(12 * MAX_YEARS_AHEAD))
        .unwrap_or(NaiveDate::MAX);
    if date > limit {
        return Err(DateIssue::TooFar);
    }
    Ok(date)
}

/// [`parse_iso`] und [`validate`] in einem.
pub fn validate_iso(text: &str, reference: NaiveDate) -> Result<NaiveDate, DateIssue> {
    validate(parse_iso(text)?, reference)
}

pub fn weekday_de(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Montag",
        Weekday::Tue => "Dienstag",
        Weekday::Wed => "Mittwoch",
        Weekday::Thu => "Donnerstag",
        Weekday::Fri => "Freitag",
        Weekday::Sat => "Samstag",
        Weekday::Sun => "Sonntag",
    }
}

/// Das Ergebnis der Aufloesung und die Regel, die es lieferte (fuer den Vermerk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub date: NaiveDate,
    pub rule: &'static str,
}

// -- Ausdruecke ------------------------------------------------------------------

const WD: &str = "(montag|dienstag|mittwoch|donnerstag|freitag|samstag|sonnabend|sonntag)";
const WEEKQ: &str = r"(naechst\w*|kommend\w*|uebernaechst\w*|dies\w*)";
const MONTHS: &str = "januar|jaenner|jan|februar|feb|maerz|maer|april|apr|mai|juni|jun|juli|jul|august|aug|september|sept|sep|oktober|okt|november|nov|dezember|dez";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("fester Ausdruck")
}

static ABS_ISO: Lazy<Regex> = Lazy::new(|| re(r"\b(\d{4})-(\d{1,2})-(\d{1,2})\b"));
static ABS_DOT_YEAR: Lazy<Regex> =
    Lazy::new(|| re(r"\b(\d{1,2})\.\s?(\d{1,2})\.\s?(\d{4}|\d{2})\b"));
static ABS_DOT: Lazy<Regex> = Lazy::new(|| re(r"\b(\d{1,2})\.\s?(\d{1,2})\.(?:\s|$)"));
static ABS_NAME: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\b(\d{{1,2}})\.?\s*({MONTHS})\b\.?(?:\s*(\d{{4}})\b)?"
    ))
});
static WEEK_WD: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\b{WD}\s+(?:der\s+|in\s+der\s+|dieser\s+)?{WEEKQ}\s+woche\b"
    ))
});
static WEEK_WD_FIRST: Lazy<Regex> =
    Lazy::new(|| re(&format!(r"\b{WEEKQ}\s+woche\s+(?:am\s+|den\s+)?{WD}\b")));
static WEEK_EDGE: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\b(ende|anfang)\s+(?:der\s+)?(?:{WEEKQ}\s+)?woche\b"
    ))
});
static MONTH_EDGE: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\b(ende|anfang)\s+(?:des\s+)?(?:{WEEKQ}\s+)?monat\w*\b"
    ))
});
static MONTH_NAME_EDGE: Lazy<Regex> =
    Lazy::new(|| re(&format!(r"\b(ende|anfang|mitte)\s+({MONTHS})\b")));
static MONTH_EDGE_WORD: Lazy<Regex> = Lazy::new(|| re(r"\bmonats(ende|anfang)\b"));
static LAST_DAY: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\bletzte\w*\s+tag\s+(?:des\s+|dieses\s+|im\s+)?(?:{WEEKQ}\s+)?monat\w*\b"
    ))
});
static YEAR_END: Lazy<Regex> = Lazy::new(|| {
    re(&format!(
        r"\b(?:jahresende|ende\s+(?:des\s+)?(?:{WEEKQ}\s+)?jahr\w*)\b"
    ))
});
static IN_UNITS: Lazy<Regex> = Lazy::new(|| {
    re(r"\bin\s+(\d{1,4}|einer|einem|einen|eine|ein|zwei|zwo|drei|vier|fuenf|sechs|sieben|acht|neun|zehn|elf|zwoelf|vierzehn|fuenfzehn|zwanzig|dreissig)\s+(tag\w*|woche\w*|monat\w*|jahr\w*)\b")
});
static CALENDAR_WEEK: Lazy<Regex> =
    Lazy::new(|| re(r"\b(?:kw|kalenderwoche)\s*(\d{1,2})\b"));
static WEEKDAY_ANY: Lazy<Regex> = Lazy::new(|| re(&format!(r"\b{WD}\b")));
static DAY_WORD: Lazy<Regex> =
    Lazy::new(|| re(r"\b(uebermorgen|vorgestern|gestern|morgen|heute)\b"));

/// Kleinschreibung, Umlaute und ss ausgeschrieben, Satzzeichen (ausser `.` und `-`) zu
/// Leerraum, Leerraum zusammengefasst.
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() || c == '.' || c == '-' => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn weekday_of(word: &str) -> Option<Weekday> {
    Some(match word {
        "montag" => Weekday::Mon,
        "dienstag" => Weekday::Tue,
        "mittwoch" => Weekday::Wed,
        "donnerstag" => Weekday::Thu,
        "freitag" => Weekday::Fri,
        "samstag" | "sonnabend" => Weekday::Sat,
        "sonntag" => Weekday::Sun,
        _ => return None,
    })
}

fn month_of(word: &str) -> Option<u32> {
    Some(match word {
        "januar" | "jaenner" | "jan" => 1,
        "februar" | "feb" => 2,
        "maerz" | "maer" => 3,
        "april" | "apr" => 4,
        "mai" => 5,
        "juni" | "jun" => 6,
        "juli" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sept" | "sep" => 9,
        "oktober" | "okt" => 10,
        "november" | "nov" => 11,
        "dezember" | "dez" => 12,
        _ => return None,
    })
}

fn number_of(word: &str) -> Option<i64> {
    if let Ok(n) = word.parse::<i64>() {
        return Some(n);
    }
    Some(match word {
        "zwei" | "zwo" => 2,
        "drei" => 3,
        "vier" => 4,
        "fuenf" => 5,
        "sechs" => 6,
        "sieben" => 7,
        "acht" => 8,
        "neun" => 9,
        "zehn" => 10,
        "elf" => 11,
        "zwoelf" => 12,
        "vierzehn" => 14,
        "fuenfzehn" => 15,
        "zwanzig" => 20,
        "dreissig" => 30,
        "ein" | "eine" | "einer" | "einem" | "einen" => 1,
        _ => return None,
    })
}

/// Wochenversatz eines Wortes: `naechste`/`kommende` +1, `uebernaechste` +2, `diese` 0.
fn week_offset(word: &str) -> Option<i64> {
    if word.starts_with("uebernaechst") {
        Some(2)
    } else if word.starts_with("naechst") || word.starts_with("kommend") {
        Some(1)
    } else if word.starts_with("dies") {
        Some(0)
    } else {
        None
    }
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

fn last_day_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    first
        .checked_add_months(Months::new(1))
        .map(|next| next - Duration::days(1))
}

fn add_months(date: NaiveDate, months: i64) -> Option<NaiveDate> {
    let n = u32::try_from(months.abs()).ok()?;
    if months >= 0 {
        date.checked_add_months(Months::new(n))
    } else {
        date.checked_sub_months(Months::new(n))
    }
}

/// Das naechste Vorkommen von Tag/Monat ab `reference` (dieses Jahr, sonst naechstes;
/// bis zu vier Jahre fuer den 29.2.).
fn next_occurrence(day: u32, month: u32, reference: NaiveDate) -> Option<NaiveDate> {
    (0..=4).find_map(|add| {
        NaiveDate::from_ymd_opt(reference.year() + add, month, day).filter(|d| *d >= reference)
    })
}

// -- Aufloesung ----------------------------------------------------------------------

/// Ergebnis einer Familie von Ausdruecken.
enum Found {
    /// Kein Treffer in dieser Familie.
    Nothing,
    One(NaiveDate, &'static str),
    /// Mehrere verschiedene Daten: nicht entscheidbar.
    Ambiguous,
}

fn collect(dates: Vec<NaiveDate>, rule: &'static str) -> Found {
    let mut unique: Vec<NaiveDate> = Vec::new();
    for d in dates {
        if !unique.contains(&d) {
            unique.push(d);
        }
    }
    match unique.as_slice() {
        [] => Found::Nothing,
        [one] => Found::One(*one, rule),
        _ => Found::Ambiguous,
    }
}

/// Absolute Daten. `None` heisst: ein Treffer war kein Kalendertag (`31.2.`), die
/// Angabe ist dann unbrauchbar.
fn absolute(norm: &str, reference: NaiveDate) -> Option<Found> {
    let mut dates: Vec<NaiveDate> = Vec::new();
    for c in ABS_ISO.captures_iter(norm) {
        let y: i32 = c[1].parse().ok()?;
        let (m, d): (u32, u32) = (c[2].parse().ok()?, c[3].parse().ok()?);
        dates.push(NaiveDate::from_ymd_opt(y, m, d)?);
    }
    for c in ABS_DOT_YEAR.captures_iter(norm) {
        let d: u32 = c[1].parse().ok()?;
        let m: u32 = c[2].parse().ok()?;
        let mut y: i32 = c[3].parse().ok()?;
        if c[3].len() == 2 {
            y += 2000;
        }
        dates.push(NaiveDate::from_ymd_opt(y, m, d)?);
    }
    for c in ABS_DOT.captures_iter(norm) {
        let d: u32 = c[1].parse().ok()?;
        let m: u32 = c[2].parse().ok()?;
        dates.push(next_occurrence(d, m, reference)?);
    }
    for c in ABS_NAME.captures_iter(norm) {
        let d: u32 = c[1].parse().ok()?;
        let m = month_of(&c[2])?;
        match c.get(3) {
            Some(y) => dates.push(NaiveDate::from_ymd_opt(y.as_str().parse().ok()?, m, d)?),
            None => dates.push(next_occurrence(d, m, reference)?),
        }
    }
    Some(collect(dates, "absolut"))
}

fn week_weekday(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    let monday = monday_of(reference);
    for c in WEEK_WD.captures_iter(norm) {
        if let (Some(day), Some(offset)) = (weekday_of(&c[1]), week_offset(&c[2])) {
            dates.push(
                monday
                    + Duration::days(7 * offset + i64::from(day.num_days_from_monday())),
            );
        }
    }
    for c in WEEK_WD_FIRST.captures_iter(norm) {
        if let (Some(offset), Some(day)) = (week_offset(&c[1]), weekday_of(&c[2])) {
            dates.push(
                monday
                    + Duration::days(7 * offset + i64::from(day.num_days_from_monday())),
            );
        }
    }
    collect(dates, "wochentag_in_woche")
}

fn week_edge(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    let monday = monday_of(reference);
    for c in WEEK_EDGE.captures_iter(norm) {
        let offset = c
            .get(2)
            .and_then(|q| week_offset(q.as_str()))
            .unwrap_or(0);
        let week = monday + Duration::days(7 * offset);
        if &c[1] == "ende" {
            let friday = week + Duration::days(4);
            // "Ende der Woche" am Wochenende meint die kommende Woche.
            let friday = if c.get(2).is_none() && friday < reference {
                friday + Duration::days(7)
            } else {
                friday
            };
            dates.push(friday);
        } else {
            dates.push(week);
        }
    }
    collect(dates, "wochenrand")
}

fn month_edge(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    let mut push = |edge: &str, offset: i64| {
        let Some(base) = add_months(
            NaiveDate::from_ymd_opt(reference.year(), reference.month(), 1)
                .unwrap_or(reference),
            offset,
        ) else {
            return;
        };
        if edge == "ende" {
            dates.extend(last_day_of_month(base.year(), base.month()));
        } else {
            dates.push(base);
        }
    };
    for c in MONTH_EDGE.captures_iter(norm) {
        let offset = c.get(2).and_then(|q| week_offset(q.as_str())).unwrap_or(0);
        push(&c[1], offset);
    }
    for c in MONTH_EDGE_WORD.captures_iter(norm) {
        push(&c[1], 0);
    }
    for c in LAST_DAY.captures_iter(norm) {
        let offset = c.get(1).and_then(|q| week_offset(q.as_str())).unwrap_or(0);
        push("ende", offset);
    }
    collect(dates, "monatsrand")
}

/// `Ende Oktober`, `Anfang November`, `Mitte Dezember`: der Monat im laufenden Jahr, ein
/// frueherer Monat des Jahres im naechsten.
fn month_name_edge(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    for c in MONTH_NAME_EDGE.captures_iter(norm) {
        let Some(month) = month_of(&c[2]) else { continue };
        let year = if month < reference.month() {
            reference.year() + 1
        } else {
            reference.year()
        };
        match &c[1] {
            "ende" => dates.extend(last_day_of_month(year, month)),
            "mitte" => dates.extend(NaiveDate::from_ymd_opt(year, month, 15)),
            _ => dates.extend(NaiveDate::from_ymd_opt(year, month, 1)),
        }
    }
    collect(dates, "monatsname_rand")
}

fn year_end(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    for c in YEAR_END.captures_iter(norm) {
        let offset = c.get(1).and_then(|q| week_offset(q.as_str())).unwrap_or(0);
        dates.extend(NaiveDate::from_ymd_opt(
            reference.year() + offset as i32,
            12,
            31,
        ));
    }
    collect(dates, "jahresende")
}

fn in_units(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    for c in IN_UNITS.captures_iter(norm) {
        let Some(n) = number_of(&c[1]) else { continue };
        let unit = &c[2];
        let date = if unit.starts_with("tag") {
            reference.checked_add_signed(Duration::days(n))
        } else if unit.starts_with("woche") {
            reference.checked_add_signed(Duration::days(7 * n))
        } else if unit.starts_with("monat") {
            add_months(reference, n)
        } else {
            add_months(reference, 12 * n)
        };
        dates.extend(date);
    }
    collect(dates, "in_n_einheiten")
}

fn calendar_week(norm: &str, reference: NaiveDate) -> Found {
    let mut dates = Vec::new();
    let current = reference.iso_week();
    for c in CALENDAR_WEEK.captures_iter(norm) {
        let Ok(week) = c[1].parse::<u32>() else { continue };
        // Eine Woche vor der jetzigen meint das naechste Jahr.
        let year = if week < current.week() {
            current.year() + 1
        } else {
            current.year()
        };
        dates.extend(NaiveDate::from_isoywd_opt(year, week, Weekday::Fri));
    }
    collect(dates, "kalenderwoche")
}

/// Genau ein Wochentag in der Angabe: der naechste nach dem Bezug.
fn weekday_alone(norm: &str, reference: NaiveDate) -> Found {
    let mut days: Vec<Weekday> = Vec::new();
    for c in WEEKDAY_ANY.captures_iter(norm) {
        if let Some(day) = weekday_of(&c[1]) {
            if !days.contains(&day) {
                days.push(day);
            }
        }
    }
    match days.as_slice() {
        [] => Found::Nothing,
        [day] => {
            let from = i64::from(reference.weekday().num_days_from_monday());
            let to = i64::from(day.num_days_from_monday());
            let ahead = (to - from).rem_euclid(7);
            let ahead = if ahead == 0 { 7 } else { ahead };
            Found::One(reference + Duration::days(ahead), "wochentag")
        }
        _ => Found::Ambiguous,
    }
}

fn day_words(norm: &str, reference: NaiveDate) -> Found {
    let dates = DAY_WORD
        .captures_iter(norm)
        .map(|c| {
            reference
                + Duration::days(match &c[1] {
                    "uebermorgen" => 2,
                    "morgen" => 1,
                    "heute" => 0,
                    "gestern" => -1,
                    _ => -2,
                })
        })
        .collect();
    collect(dates, "tageswort")
}

/// Loest eine gesprochene Datumsangabe zu einem Datum auf (siehe Moduldoku). `None`: nicht
/// eindeutig aufloesbar. Die Pruefung gegen Vergangenheit und Obergrenze ist [`validate`].
pub fn resolve(phrase: &str, reference: NaiveDate) -> Option<Resolved> {
    let norm = fold(phrase);
    if norm.is_empty() {
        return None;
    }
    let abs = match absolute(&norm, reference)? {
        Found::Ambiguous => return None,
        Found::One(date, rule) => Some((date, rule)),
        Found::Nothing => None,
    };

    // Relative Familien in fester Reihenfolge; die erste mit Treffer entscheidet.
    let families: [fn(&str, NaiveDate) -> Found; 9] = [
        week_weekday,
        week_edge,
        month_edge,
        month_name_edge,
        year_end,
        in_units,
        calendar_week,
        weekday_alone,
        day_words,
    ];
    let mut relative: Option<(NaiveDate, &'static str)> = None;
    for family in families {
        match family(&norm, reference) {
            Found::Nothing => continue,
            Found::Ambiguous => return None,
            Found::One(date, rule) => {
                relative = Some((date, rule));
                break;
            }
        }
    }

    let (date, rule) = match (abs, relative) {
        (Some((a, _)), Some((r, rule))) => {
            // Ein Wochentag allein ("Freitag, den 9. Oktober") ist nur eine Gegenprobe,
            // alles andere muss dasselbe Datum meinen.
            if rule != "wochentag" && a != r {
                return None;
            }
            (a, "absolut")
        }
        (Some((a, rule)), None) => (a, rule),
        (None, Some((r, rule))) => (r, rule),
        (None, None) => return None,
    };

    // Ein genannter Wochentag muss zum Ergebnis passen ("Freitag" und der 8. Oktober
    // 2026, ein Donnerstag, sind ein Widerspruch).
    if rule != "wochentag" && rule != "wochentag_in_woche" {
        let mut spoken = WEEKDAY_ANY
            .captures_iter(&norm)
            .filter_map(|c| weekday_of(&c[1]));
        if let Some(first) = spoken.next() {
            if !(first == date.weekday() && spoken.all(|d| d == date.weekday())) {
                return None;
            }
        }
    }
    Some(Resolved { date, rule })
}

/// Die Datumshilfe fuer den Prompt: vom Code berechnet, damit das Modell nicht rechnet.
pub fn help_table(reference: NaiveDate) -> String {
    let day = |d: NaiveDate| format!("{}, {}", weekday_de(d.weekday()), d.format("%Y-%m-%d"));
    let next_days = (1..=7)
        .map(|n| {
            let d = reference + Duration::days(n);
            format!("{} {}", weekday_de(d.weekday()), d.format("%Y-%m-%d"))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let month_end = last_day_of_month(reference.year(), reference.month()).unwrap_or(reference);
    format!(
        "Datumshilfe (vom Programm berechnet; Bezug ist das Datum der Besprechung):\n\
         - heute = {}\n\
         - morgen = {}\n\
         - übermorgen = {}\n\
         - die nächsten sieben Tage: {}\n\
         - in einer Woche = {}, in zwei Wochen = {}\n\
         - Ende des Monats = {}",
        day(reference),
        day(reference + Duration::days(1)),
        day(reference + Duration::days(2)),
        next_days,
        (reference + Duration::days(7)).format("%Y-%m-%d"),
        (reference + Duration::days(14)).format("%Y-%m-%d"),
        month_end.format("%Y-%m-%d"),
    )
}

#[cfg(test)]
mod tests;
