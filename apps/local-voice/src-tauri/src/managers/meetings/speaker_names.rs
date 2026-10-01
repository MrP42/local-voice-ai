//! U8: Namensvorschlaege fuer erkannte Sprecher aus dem Gesagten.
//!
//! Wird in einem Satz eine Person angesprochen ("Vielen Dank, Andre", "Anna, was
//! meinst du?"), ist der vorherige (bei Dank) bzw. der naechste Sprecher (bei
//! Frage) mit hoher Wahrscheinlichkeit diese Person. Reine Heuristik ohne Modell,
//! ohne Tauri und ohne Einstellungen: Segmente, bekannte Personen und der eigene
//! Name kommen als Parameter herein. Ergebnis sind VORSCHLAEGE mit Konfidenz und
//! Belegstellen; nie wird ein Name von selbst uebernommen.
//!
//! Regeln (Gewichte siehe Konstanten):
//! - Dank + Name ("Danke, Anna", "Vielen Dank, Andre") -> vorheriger anderer Sprecher.
//! - Anrede mit Frage oder Du-/Sie-Form ("Anna, was meinst du?", "Was denken Sie,
//!   Anna?") -> naechster anderer Sprecher.
//! - Gruss + Name ("Hallo Anna") -> naechster Sprecher (schwach, nur bekannte Namen).
//! - Selbstvorstellung ("Ich bin Anna", "Mein Name ist Anna Berg") -> der Sprecher selbst.
//! - Nie: Namen in Zitaten, Titel + Nachname ("Danke, Frau Mueller"), Dank "an"
//!   Dritte, Namen weiter als [`MAX_GAP_MS`] vom Sprecherwechsel, der eigene Name,
//!   Sprecher mit Namen, Namen, die schon ein anderer Sprecher traegt.
//! - Mehrere Belege verstaerken (noisy-or), Widersprueche (anderer Name fuer denselben
//!   Sprecher, derselbe Name fuer einen anderen Sprecher) senken.
//!
//! Datenschutz: Namen und Belegstellen stehen in keinem Log.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::StoredSegment;
use crate::managers::people::normalize::normalize_name;

/// Hoechster Abstand zwischen Sprecherwechsel und Anrede.
pub const MAX_GAP_MS: u64 = 15_000;
/// Bis hierher gilt der Wechsel als "direkt".
pub const NEAR_GAP_MS: u64 = 5_000;
/// Darunter wird kein Vorschlag gezeigt.
pub const MIN_CONFIDENCE: f64 = 0.5;

const W_THANKS_NEAR: f64 = 0.85;
const W_THANKS_FAR: f64 = 0.65;
const W_ADDRESS_NEAR: f64 = 0.75;
const W_ADDRESS_FAR: f64 = 0.55;
const W_VOCATIVE: f64 = 0.3;
const W_GREETING: f64 = 0.4;
const W_INTRO: f64 = 0.85;
/// Faktor fuer ein grossgeschriebenes Wort, das weder eine bekannte Person noch
/// ein gaengiger Vorname ist (im Deutschen sind alle Nomen gross).
const UNKNOWN_FACTOR: f64 = 0.8;
/// Ein Widerspruch zieht diesen Anteil des staerksten Gegenbelegs ab.
const CONTRADICTION: f64 = 0.5;
/// Laengste Belegstelle (Zeichen).
const QUOTE_CHARS: usize = 140;

/// Eine Belegstelle: der Satz, in dem die Person angesprochen wird.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct NameEvidence {
    pub segment_index: u32,
    pub start_ms: u64,
    pub quote: String,
    /// `thanks`, `address`, `greeting` oder `intro`.
    pub rule: String,
}

/// "Person 2 ist vermutlich Andre (3 Belege)".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct NameSuggestion {
    pub channel: u8,
    pub speaker_index: u32,
    pub name: String,
    /// 0.5 bis 1.0.
    pub confidence: f64,
    pub evidence: Vec<NameEvidence>,
}

/// Was die Heuristik ausser den Segmenten weiss.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Namen bekannter Personen (Personen P5d, Teilnehmende des Kalenders).
    pub known_people: Vec<String>,
    /// Der eigene Name ("Mein Name"): wird nie einem anderen Sprecher zugeordnet.
    pub self_name: Option<String>,
    /// Sprecher, die schon einen Namen haben: `((Kanal, Nummer), Name)`.
    pub named: Vec<((u8, u32), String)>,
}

/// Schluessel fuer "nicht mehr vorschlagen" in `metadata_json`.
pub fn dismiss_key(channel: u8, speaker_index: u32, name: &str) -> String {
    format!("{channel}:{speaker_index}:{}", normalize_name(name))
}

// ---------------------------------------------------------------------------
// Woerter
// ---------------------------------------------------------------------------

/// Gaengige deutsche Vornamen (klein, ohne Akzente): ein Wort daraus gilt auch
/// ohne Eintrag in der Personenliste als Vorname.
const FIRST_NAMES: &[&str] = &[
    "alexander",
    "andreas",
    "andre",
    "anna",
    "anne",
    "annika",
    "anja",
    "antje",
    "barbara",
    "bastian",
    "ben",
    "benjamin",
    "bernd",
    "birgit",
    "brigitte",
    "carsten",
    "christian",
    "christina",
    "christine",
    "christoph",
    "claudia",
    "clemens",
    "clara",
    "daniel",
    "dennis",
    "dieter",
    "dirk",
    "elena",
    "elisabeth",
    "emil",
    "emma",
    "eva",
    "fabian",
    "felix",
    "florian",
    "frank",
    "franziska",
    "friedrich",
    "gabriele",
    "georg",
    "gerd",
    "hannah",
    "hans",
    "heike",
    "heinz",
    "helmut",
    "henrik",
    "holger",
    "ines",
    "jan",
    "jana",
    "jannik",
    "jens",
    "jessica",
    "joachim",
    "johannes",
    "jonas",
    "julia",
    "juergen",
    "jutta",
    "karin",
    "karl",
    "katharina",
    "kathrin",
    "katja",
    "kerstin",
    "klaus",
    "kristin",
    "lars",
    "laura",
    "lea",
    "lena",
    "leon",
    "lisa",
    "lukas",
    "manfred",
    "manuel",
    "marc",
    "marcel",
    "marco",
    "maria",
    "marie",
    "mario",
    "marius",
    "markus",
    "martin",
    "martina",
    "matthias",
    "max",
    "melanie",
    "michael",
    "michaela",
    "monika",
    "nadine",
    "nicole",
    "nico",
    "nina",
    "norbert",
    "oliver",
    "patrick",
    "paul",
    "petra",
    "peter",
    "philipp",
    "ralf",
    "rainer",
    "reinhard",
    "robert",
    "sabine",
    "sandra",
    "sarah",
    "sascha",
    "sebastian",
    "silke",
    "simon",
    "sonja",
    "stefan",
    "stefanie",
    "steffen",
    "susanne",
    "sven",
    "tanja",
    "thomas",
    "tim",
    "tobias",
    "torsten",
    "ulrich",
    "ulrike",
    "ute",
    "uwe",
    "vera",
    "volker",
    "walter",
    "werner",
    "wolfgang",
    "yvonne",
];

/// Titel und Anreden: danach kommt ein Nachname, kein Vorname.
const TITLES: &[&str] = &[
    "herr",
    "herrn",
    "frau",
    "fraeulein",
    "dr",
    "prof",
    "professor",
    "doktor",
    "direktor",
    "direktorin",
    "kollege",
    "kollegin",
    "kollegen",
];

/// Grossgeschriebene Woerter, die nach Dank, Gruss oder am Satzanfang stehen und
/// keine Namen sind (Schluessel normalisiert).
const NOT_NAMES: &[&str] = &[
    "danke",
    "dank",
    "dankeschoen",
    "vielen",
    "herzlichen",
    "besten",
    "thanks",
    "thank",
    "schoen",
    "schon",
    "sehr",
    "dir",
    "dich",
    "ihnen",
    "ihr",
    "ihre",
    "euch",
    "dafuer",
    "vielmals",
    "allen",
    "alle",
    "allerseits",
    "zusammen",
    "leute",
    "team",
    "runde",
    "wir",
    "ich",
    "das",
    "die",
    "der",
    "den",
    "dem",
    "es",
    "sie",
    "er",
    "man",
    "und",
    "aber",
    "dann",
    "jetzt",
    "also",
    "ja",
    "nein",
    "gut",
    "okay",
    "ok",
    "genau",
    "stimmt",
    "so",
    "nun",
    "hier",
    "da",
    "wie",
    "was",
    "wer",
    "wo",
    "wann",
    "warum",
    "bitte",
    "gerne",
    "gern",
    "super",
    "perfekt",
    "klar",
    "richtig",
    "sorry",
    "hallo",
    "hi",
    "hey",
    "guten",
    "willkommen",
    "morgen",
    "tag",
    "abend",
    "liebe",
    "lieber",
    "herzlich",
    "kurz",
    "noch",
    "mal",
    "fuer",
    "fuers",
    "nochmal",
    "auch",
    "dass",
    "wenn",
    "weil",
    "damit",
    "zuerst",
    "zunaechst",
    "leider",
    "heute",
    "gestern",
    "wichtig",
    "natuerlich",
    "sicher",
    "wunderbar",
    "prima",
    "kunde",
    "kunden",
    "chef",
    "chefin",
    "moment",
    "stopp",
    "stop",
];

fn key_of(word: &str) -> String {
    normalize_name(word)
}

fn in_list(list: &[&str], word: &str) -> bool {
    let k = key_of(word);
    list.contains(&k.as_str())
}

// ---------------------------------------------------------------------------
// Muster
// ---------------------------------------------------------------------------

const NAME: &str = r"\p{Lu}[\p{L}\u{2019}'\-]{1,24}";

struct Patterns {
    thanks: Regex,
    greeting: Regex,
    intro_strong: Regex,
    intro_weak: Regex,
    vocative_lead: Regex,
    vocative_tail: Regex,
    marker: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let re = |s: String| Regex::new(&s).expect("Muster gueltig");
        Patterns {
            thanks: re(format!(
                r"(?i:\b(?:(?:vielen\s+herzlichen|vielen|herzlichen|besten|recht\s+herzlichen|ganz\s+herzlichen)\s+dank|danke(?:\s+(?:dir|euch|ihnen|schön|schoen|sehr|vielmals|dafür))*|dankeschön|dankeschoen|thanks|thank\s+you))\s*,?\s*(?P<n>{NAME})(?:\s+(?P<n2>{NAME}))?"
            )),
            greeting: re(format!(
                r"(?i:\b(?:hallo|hi|hey|guten\s+(?:morgen|tag|abend)|willkommen|servus|moin))\s*,?\s+(?P<n>{NAME})"
            )),
            intro_strong: re(format!(
                r"(?i:\b(?:mein\s+name\s+ist|ich\s+heiße|ich\s+heisse))\s+(?P<n>{NAME})(?:\s+(?P<n2>{NAME}))?"
            )),
            intro_weak: re(format!(
                r"(?i:\b(?:ich\s+bin|hier\s+spricht|hier\s+ist))\s+(?P<n>{NAME})(?:\s+(?P<n2>{NAME}))?"
            )),
            vocative_lead: re(format!(
                r"(?s)^\s*(?:(?i:hallo|hi|hey|also|okay|ok|ja|gut|und|dann|so|lieber|liebe)\s*,?\s+)*(?P<n>{NAME})\s*,\s*(?P<rest>.*)$"
            )),
            vocative_tail: re(format!(
                r"(?s)^(?P<lead>.*),\s*(?P<n>{NAME})\s*[?!.\u{{2026}}]*\s*$"
            )),
            marker: re(
                r"(?i:\b(?:du|dir|dich|dein|deine|deinen|deinem|deiner|deines|kannst|könntest|koenntest|möchtest|moechtest|willst|magst|hast|bist|meinst|denkst|würdest|wuerdest|ihr|euch)\b)|\b(?:Sie|Ihnen|Ihre|Ihr|Ihrer|Ihren|Ihrem)\b".to_string(),
            ),
        }
    })
}

// ---------------------------------------------------------------------------
// Textaufbereitung
// ---------------------------------------------------------------------------

/// Zitate durch Leerzeichen ersetzen (gleiche Zeichenzahl): Namen in
/// Anfuehrungszeichen sind nicht angesprochen. Ein offenes Zitat reicht bis zum Ende.
fn mask_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        if inside {
            if matches!(c, '\u{201C}' | '\u{201D}' | '\u{00AB}' | '\u{00BB}' | '"') {
                inside = false;
            }
            out.push(' ');
        } else if matches!(c, '\u{201E}' | '\u{00BB}' | '\u{201C}' | '\u{00AB}' | '"') {
            inside = true;
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Saetze als Zeichenbereiche `[start, end)` samt Fragezeichen am Ende.
fn sentences(text: &str) -> Vec<(usize, usize, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if matches!(chars[i], '.' | '!' | '?' | '\u{2026}') {
            let mut end = i + 1;
            while end < chars.len() && matches!(chars[end], '.' | '!' | '?' | '\u{2026}') {
                end += 1;
            }
            if end >= chars.len() || chars[end].is_whitespace() {
                let question = chars[i..end].contains(&'?');
                out.push((start, end, question));
                start = end;
                i = end;
                continue;
            }
            i = end;
            continue;
        }
        i += 1;
    }
    if chars[start..].iter().any(|c| !c.is_whitespace()) {
        out.push((start, chars.len(), false));
    }
    out
}

fn char_index(masked: &str, byte: usize) -> usize {
    masked[..byte].chars().count()
}

/// Der Satz um `pos` aus dem Originaltext, gekuerzt.
fn quote_at(original: &str, spans: &[(usize, usize, bool)], pos: usize) -> String {
    let chars: Vec<char> = original.chars().collect();
    let (from, to) = spans
        .iter()
        .find(|(a, b, _)| pos >= *a && pos < *b)
        .map(|(a, b, _)| (*a, (*b).min(chars.len())))
        .unwrap_or((0, chars.len()));
    let sentence: String = chars[from.min(chars.len())..to].iter().collect();
    let sentence = sentence.trim();
    if sentence.chars().count() <= QUOTE_CHARS {
        sentence.to_string()
    } else {
        let cut: String = sentence.chars().take(QUOTE_CHARS).collect();
        format!("{}…", cut.trim_end())
    }
}

// ---------------------------------------------------------------------------
// Heuristik
// ---------------------------------------------------------------------------

type Key = (u8, Option<u32>);

/// Ein gefundener Name im Text eines Segments.
struct Hit {
    /// Erstes Wort (Vorname) und ggf. der volle Name wie gesprochen.
    name: String,
    rule: &'static str,
    weight: f64,
    /// Wen der Beleg betrifft.
    target: Target,
    quote: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Previous,
    Next,
    Author,
}

struct Names {
    /// Vornamen-Schluessel: gaengige Namen plus erste Woerter bekannter Personen.
    first: HashSet<String>,
    /// Vorname-Schluessel -> volle Namen bekannter Personen.
    known_full: HashMap<String, Vec<String>>,
    own_first: Option<String>,
    own_full: Option<String>,
}

impl Names {
    fn new(ctx: &Context) -> Self {
        let mut first: HashSet<String> = FIRST_NAMES.iter().map(|s| s.to_string()).collect();
        let mut known_full: HashMap<String, Vec<String>> = HashMap::new();
        let people = ctx
            .known_people
            .iter()
            .chain(ctx.named.iter().map(|(_, n)| n));
        for full in people {
            let full = full.trim();
            let Some(head) = full.split_whitespace().next() else {
                continue;
            };
            let k = key_of(head);
            if k.is_empty() {
                continue;
            }
            first.insert(k.clone());
            let list = known_full.entry(k).or_default();
            if !list.iter().any(|f| f == full) {
                list.push(full.to_string());
            }
        }
        let own_full = ctx
            .self_name
            .as_deref()
            .map(key_of)
            .filter(|k| !k.is_empty());
        let own_first = ctx
            .self_name
            .as_deref()
            .and_then(|n| n.split_whitespace().next())
            .map(key_of)
            .filter(|k| !k.is_empty());
        Self {
            first,
            known_full,
            own_first,
            own_full,
        }
    }

    fn is_known_first(&self, word: &str) -> bool {
        self.first.contains(&key_of(word))
    }

    fn is_own(&self, name: &str) -> bool {
        let full = key_of(name);
        let head = name
            .split_whitespace()
            .next()
            .map(key_of)
            .unwrap_or_default();
        self.own_full.as_deref() == Some(full.as_str())
            || self.own_first.as_deref() == Some(head.as_str())
    }

    /// Anzeigename: die eindeutig bekannte Person mit diesem Vornamen, sonst wie gesprochen.
    fn display(&self, spoken: &str) -> String {
        let words: Vec<&str> = spoken.split_whitespace().collect();
        let Some(head) = words.first() else {
            return spoken.to_string();
        };
        if let Some(list) = self.known_full.get(&key_of(head)) {
            if words.len() == 1 && list.len() == 1 {
                return list[0].clone();
            }
            if let Some(full) = list.iter().find(|f| key_of(f) == key_of(spoken)) {
                return full.clone();
            }
        }
        spoken.to_string()
    }
}

/// Name aus den Treffergruppen: ein Wort, oder zwei, wenn das erste ein bekannter
/// Vorname ist. `None` fuer Titel, Fuellwoerter und Unbrauchbares.
fn pick_name(first: &str, second: Option<&str>, names: &Names) -> Option<(String, bool)> {
    if in_list(TITLES, first) || in_list(NOT_NAMES, first) {
        return None;
    }
    let known = names.is_known_first(first);
    let two = second.filter(|w| {
        known && !in_list(NOT_NAMES, w) && !in_list(TITLES, w) && !names.is_known_first(w)
    });
    let name = match two {
        Some(w) => format!("{first} {w}"),
        None => first.to_string(),
    };
    Some((name, known))
}

fn marker_in(text: &str) -> bool {
    patterns().marker.is_match(text)
}

fn near(gap_ms: u64, near_w: f64, far_w: f64) -> f64 {
    if gap_ms <= NEAR_GAP_MS {
        near_w
    } else {
        far_w
    }
}

/// Alle Anreden in einem Segment. Das Gewicht hier ist ohne Abstand
/// (`Previous`/`Next` werden spaeter mit dem Zeitabstand gewichtet).
fn hits_in(original: &str, names: &Names) -> Vec<Hit> {
    let masked = mask_quotes(original);
    let spans = sentences(&masked);
    let p = patterns();
    let mut hits: Vec<Hit> = Vec::new();

    // Dank + Name -> vorheriger Sprecher.
    for caps in p.thanks.captures_iter(&masked) {
        let n = caps.name("n").expect("Gruppe n");
        let second = caps.name("n2").map(|m| m.as_str());
        let Some((name, known)) = pick_name(n.as_str(), second, names) else {
            continue;
        };
        let pos = char_index(&masked, n.start());
        hits.push(Hit {
            name,
            rule: "thanks",
            weight: if known { 1.0 } else { UNKNOWN_FACTOR },
            target: Target::Previous,
            quote: quote_at(original, &spans, pos),
        });
    }

    // Gruss + Name -> naechster Sprecher (nur bekannte Namen).
    for caps in p.greeting.captures_iter(&masked) {
        let n = caps.name("n").expect("Gruppe n");
        let Some((name, known)) = pick_name(n.as_str(), None, names) else {
            continue;
        };
        if !known {
            continue;
        }
        let pos = char_index(&masked, n.start());
        hits.push(Hit {
            name,
            rule: "greeting",
            weight: W_GREETING,
            target: Target::Next,
            quote: quote_at(original, &spans, pos),
        });
    }

    // Selbstvorstellung -> der Sprecher selbst.
    for (re, needs_known) in [(&p.intro_strong, false), (&p.intro_weak, true)] {
        for caps in re.captures_iter(&masked) {
            let n = caps.name("n").expect("Gruppe n");
            let second = caps.name("n2").map(|m| m.as_str());
            let Some((name, known)) = pick_name(n.as_str(), second, names) else {
                continue;
            };
            if needs_known && !known {
                continue;
            }
            let pos = char_index(&masked, n.start());
            hits.push(Hit {
                name,
                rule: "intro",
                weight: if known { 1.0 } else { UNKNOWN_FACTOR },
                target: Target::Author,
                quote: quote_at(original, &spans, pos),
            });
        }
    }

    // Anrede mit Frage oder Du-/Sie-Form -> naechster Sprecher.
    let chars: Vec<char> = masked.chars().collect();
    for &(a, b, question) in &spans {
        let sentence: String = chars[a..b.min(chars.len())].iter().collect();
        let body = sentence.trim_end_matches(|c: char| ".!?…".contains(c) || c.is_whitespace());
        let mut found: Option<(String, bool)> = None;
        if let Some(caps) = p.vocative_lead.captures(body) {
            let n = caps.name("n").expect("Gruppe n").as_str();
            let rest = caps.name("rest").map(|m| m.as_str()).unwrap_or("");
            if let Some((name, known)) = pick_name(n, None, names) {
                if known {
                    found = Some((name, question || marker_in(rest)));
                }
            }
        }
        if found.is_none() {
            if let Some(caps) = p.vocative_tail.captures(body) {
                let n = caps.name("n").expect("Gruppe n").as_str();
                let lead = caps.name("lead").map(|m| m.as_str()).unwrap_or("");
                if let Some((name, known)) = pick_name(n, None, names) {
                    if known && (question || marker_in(lead)) {
                        found = Some((name, true));
                    }
                }
            }
        }
        if let Some((name, strong)) = found {
            hits.push(Hit {
                name,
                rule: "address",
                // Stark: Gewicht wird mit dem Abstand gewaehlt (siehe `weigh`).
                weight: if strong { 1.0 } else { W_VOCATIVE },
                target: Target::Next,
                quote: quote_at(original, &spans, a),
            });
        }
    }
    hits
}

/// Gewicht eines Belegs mit Zeitabstand.
fn weigh(hit: &Hit, gap_ms: u64) -> f64 {
    match (hit.rule, hit.target) {
        ("thanks", _) => near(gap_ms, W_THANKS_NEAR, W_THANKS_FAR) * hit.weight,
        ("address", _) if hit.weight >= 1.0 => near(gap_ms, W_ADDRESS_NEAR, W_ADDRESS_FAR),
        ("address", _) => hit.weight,
        ("greeting", _) => hit.weight,
        ("intro", _) => W_INTRO * hit.weight,
        _ => 0.0,
    }
}

#[derive(Default)]
struct Raw {
    /// Belege dieses (Sprecher, Name): je Segment der staerkste.
    by_segment: BTreeMap<u32, (f64, NameEvidence)>,
    spellings: HashMap<String, usize>,
}

/// Vorschlaege, nach Konfidenz absteigend, hoechstens einer je Sprecher.
pub fn suggest(segments: &[StoredSegment], ctx: &Context) -> Vec<NameSuggestion> {
    let mut sorted: Vec<&StoredSegment> = segments.iter().collect();
    sorted.sort_by_key(|s| (s.start_ms, s.segment_index));
    let names = Names::new(ctx);
    let key = |s: &StoredSegment| -> Key { (s.channel, s.speaker_index) };
    let named: HashMap<(u8, u32), &str> = ctx.named.iter().map(|(k, n)| (*k, n.as_str())).collect();
    let taken: HashSet<String> = ctx
        .named
        .iter()
        .filter_map(|(_, n)| n.split_whitespace().next().map(key_of))
        .collect();

    let mut raw: BTreeMap<((u8, u32), String), Raw> = BTreeMap::new();

    for (i, seg) in sorted.iter().enumerate() {
        let author = key(seg);
        for hit in hits_in(&seg.text, &names) {
            if names.is_own(&hit.name) {
                continue;
            }
            let head = hit
                .name
                .split_whitespace()
                .next()
                .map(key_of)
                .unwrap_or_default();
            if head.is_empty() || taken.contains(&head) {
                continue;
            }
            // Wer ist gemeint, und wie weit ist der Sprecherwechsel weg?
            let (target, gap) = match hit.target {
                Target::Author => (author, 0),
                Target::Previous => {
                    let Some(prev) = sorted[..i].iter().rev().find(|s| key(**s) != author) else {
                        continue;
                    };
                    (key(*prev), seg.start_ms.saturating_sub(prev.end_ms))
                }
                Target::Next => {
                    let Some(next) = sorted[i + 1..].iter().find(|s| key(**s) != author) else {
                        continue;
                    };
                    (key(*next), next.start_ms.saturating_sub(seg.end_ms))
                }
            };
            if gap > MAX_GAP_MS {
                continue;
            }
            let (channel, Some(index)) = target else {
                continue;
            };
            // Wer schon einen Namen hat, bekommt keinen Vorschlag; wer selbst so
            // heisst, wird nicht nach dem eigenen Dank benannt.
            if named.contains_key(&(channel, index)) {
                continue;
            }
            if let (author_channel, Some(author_index)) = author {
                if let Some(own) = named.get(&(author_channel, author_index)) {
                    if own.split_whitespace().next().map(key_of).as_deref() == Some(head.as_str()) {
                        continue;
                    }
                }
            }
            let weight = weigh(&hit, gap);
            if weight <= 0.0 {
                continue;
            }
            let entry = raw.entry(((channel, index), head)).or_default();
            *entry.spellings.entry(hit.name.clone()).or_insert(0) += 1;
            let evidence = NameEvidence {
                segment_index: seg.segment_index,
                start_ms: seg.start_ms,
                quote: hit.quote,
                rule: hit.rule.to_string(),
            };
            let slot = entry
                .by_segment
                .entry(seg.segment_index)
                .or_insert((0.0, evidence.clone()));
            if weight > slot.0 {
                *slot = (weight, evidence);
            }
        }
    }

    // Belege je (Sprecher, Name) zu einer Bewertung verrechnen (noisy-or).
    let scores: Vec<(((u8, u32), String), f64)> = raw
        .iter()
        .map(|(k, r)| {
            let miss: f64 = r.by_segment.values().map(|(w, _)| 1.0 - w).product();
            (k.clone(), 1.0 - miss)
        })
        .collect();

    // Widersprueche: anderer Name fuer denselben Sprecher, derselbe Name fuer
    // einen anderen Sprecher.
    let mut best: BTreeMap<(u8, u32), NameSuggestion> = BTreeMap::new();
    for ((speaker, head), score) in &scores {
        let against = scores
            .iter()
            .filter(|((s2, h2), _)| (s2 == speaker) != (h2 == head))
            .map(|(_, v)| *v)
            .fold(0.0_f64, f64::max);
        let confidence = (score - CONTRADICTION * against).clamp(0.0, 1.0);
        if confidence < MIN_CONFIDENCE {
            continue;
        }
        let entry = &raw[&(*speaker, head.clone())];
        let spoken = entry
            .spellings
            .iter()
            .max_by(|a, b| {
                a.1.cmp(b.1)
                    .then_with(|| a.0.chars().count().cmp(&b.0.chars().count()))
            })
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| head.clone());
        let suggestion = NameSuggestion {
            channel: speaker.0,
            speaker_index: speaker.1,
            name: names.display(&spoken),
            confidence: (confidence * 100.0).round() / 100.0,
            evidence: entry.by_segment.values().map(|(_, e)| e.clone()).collect(),
        };
        let better = best
            .get(speaker)
            .is_none_or(|old| suggestion.confidence > old.confidence);
        if better {
            best.insert(*speaker, suggestion);
        }
    }

    let mut out: Vec<NameSuggestion> = best.into_values().collect();
    for s in &mut out {
        s.evidence.sort_by_key(|e| (e.start_ms, e.segment_index));
    }
    out.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| (a.channel, a.speaker_index).cmp(&(b.channel, b.speaker_index)))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(
        index: u32,
        channel: u8,
        speaker: Option<u32>,
        from_s: f64,
        to_s: f64,
        text: &str,
    ) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.to_string(),
            start_ms: (from_s * 1000.0) as u64,
            end_ms: (to_s * 1000.0) as u64,
            channel,
            speaker_index: speaker,
            words: None,
        }
    }

    fn ctx() -> Context {
        Context::default()
    }

    /// Gegenseite: Sprecher 1 redet, dann Sprecher 2 mit `reply`.
    fn thanks_after_speaker_one(reply: &str) -> Vec<StoredSegment> {
        vec![
            seg(
                0,
                1,
                Some(1),
                0.0,
                10.0,
                "Wir haben das Budget für 2027 geprüft.",
            ),
            seg(1, 1, Some(2), 10.5, 14.0, reply),
        ]
    }

    fn one(list: &[NameSuggestion]) -> &NameSuggestion {
        assert_eq!(list.len(), 1, "{list:?}");
        &list[0]
    }

    #[test]
    fn thanks_names_the_previous_speaker() {
        let list = suggest(
            &thanks_after_speaker_one("Vielen Dank, André. Dann kommen wir zum Zeitplan."),
            &ctx(),
        );
        let s = one(&list);
        assert_eq!((s.channel, s.speaker_index), (1, 1));
        assert_eq!(s.name, "André");
        assert!(s.confidence >= MIN_CONFIDENCE, "{s:?}");
        assert_eq!(s.evidence.len(), 1);
        assert_eq!(s.evidence[0].segment_index, 1);
        assert_eq!(s.evidence[0].start_ms, 10_500);
        assert_eq!(s.evidence[0].rule, "thanks");
        assert!(s.evidence[0].quote.contains("Vielen Dank, André"), "{s:?}");
    }

    #[test]
    fn thanks_forms_with_dir_and_without_comma_work() {
        for text in [
            "Danke dir, Anna.",
            "Danke Anna, das war hilfreich.",
            "Herzlichen Dank, Anna!",
        ] {
            let list = suggest(&thanks_after_speaker_one(text), &ctx());
            assert_eq!(one(&list).name, "Anna", "{text}");
            assert_eq!(list[0].speaker_index, 1, "{text}");
        }
    }

    #[test]
    fn a_question_names_the_next_speaker() {
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 6.0, "André, was meinst du dazu?"),
            seg(1, 1, Some(2), 6.5, 12.0, "Ich halte den Plan für machbar."),
        ];
        let list = suggest(&segments, &ctx());
        let s = one(&list);
        assert_eq!(
            (s.channel, s.speaker_index, s.name.as_str()),
            (1, 2, "André")
        );
        assert_eq!(s.evidence[0].rule, "address");

        let trailing = vec![
            seg(0, 1, Some(1), 0.0, 6.0, "Was meinst du dazu, André?"),
            seg(1, 1, Some(2), 6.5, 12.0, "Ich halte den Plan für machbar."),
        ];
        let list = suggest(&trailing, &ctx());
        assert_eq!(
            (one(&list).speaker_index, one(&list).name.as_str()),
            (2, "André")
        );
    }

    #[test]
    fn several_pieces_of_evidence_raise_the_confidence() {
        let single = suggest(&thanks_after_speaker_one("Danke, Anna."), &ctx());
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Erster Punkt ist das Budget."),
            seg(1, 1, Some(2), 10.5, 14.0, "Danke, Anna."),
            seg(2, 1, Some(1), 14.5, 25.0, "Zweiter Punkt ist der Zeitplan."),
            seg(
                3,
                1,
                Some(2),
                25.5,
                28.0,
                "Vielen Dank, Anna, das hilft uns.",
            ),
        ];
        let double = suggest(&segments, &ctx());
        assert_eq!(one(&double).evidence.len(), 2);
        assert!(double[0].confidence > one(&single).confidence);
        assert!(double[0].confidence <= 1.0);
    }

    #[test]
    fn contradictions_remove_the_suggestion() {
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Erster Punkt ist das Budget."),
            seg(1, 1, Some(2), 10.5, 14.0, "Danke, Anna."),
            seg(2, 1, Some(1), 14.5, 25.0, "Zweiter Punkt ist der Zeitplan."),
            seg(3, 1, Some(2), 25.5, 28.0, "Danke, Ben."),
        ];
        assert!(suggest(&segments, &ctx()).is_empty());
    }

    #[test]
    fn the_same_name_for_two_speakers_weakens_the_weaker_one() {
        // Sprecher 1 wird zweimal "Anna" genannt, Sprecher 3 einmal.
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Erster Punkt."),
            seg(1, 1, Some(2), 10.5, 14.0, "Danke, Anna."),
            seg(2, 1, Some(1), 14.5, 20.0, "Zweiter Punkt."),
            seg(3, 1, Some(2), 20.5, 24.0, "Danke, Anna."),
            seg(4, 1, Some(3), 24.5, 30.0, "Dritter Punkt."),
            seg(5, 1, Some(2), 30.5, 34.0, "Danke, Anna."),
        ];
        let list = suggest(&segments, &ctx());
        // Nach 1 kommt 2 (zweimal Anna), nach 3 kommt 2 (einmal Anna): beide
        // Belege betreffen den VORHERIGEN Sprecher; nur Sprecher 1 bleibt.
        let s = one(&list);
        assert_eq!((s.speaker_index, s.name.as_str()), (1, "Anna"));
    }

    #[test]
    fn title_and_surname_are_not_a_first_name() {
        for text in [
            "Danke, Frau Müller.",
            "Vielen Dank, Herr Schmidt.",
            "Danke, Dr. Meier, das war klar.",
            "Danke Frau Berg für die Erklärung.",
        ] {
            assert!(
                suggest(&thanks_after_speaker_one(text), &ctx()).is_empty(),
                "{text}"
            );
        }
    }

    #[test]
    fn names_inside_quotes_are_ignored() {
        for text in [
            "Er sagte: „Danke, Anna“ und ging.",
            "Sie schrieb \"Vielen Dank, Anna\" unter die Mail.",
            "Im Text steht »Danke Anna«, das ist alles.",
        ] {
            assert!(
                suggest(&thanks_after_speaker_one(text), &ctx()).is_empty(),
                "{text}"
            );
        }
    }

    #[test]
    fn thanks_to_third_parties_and_plain_mentions_do_not_count() {
        for text in [
            "Danke an Anna für die Vorarbeit.",
            "Anna hat das Angebot geschrieben.",
            "Wir haben mit Anna telefoniert.",
            "Danke für die Zahlen.",
        ] {
            assert!(
                suggest(&thanks_after_speaker_one(text), &ctx()).is_empty(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_own_name_is_never_given_to_another_speaker() {
        let own = Context {
            self_name: Some("Patrick Wolff".into()),
            ..Context::default()
        };
        let segments = thanks_after_speaker_one("Danke, Patrick.");
        assert!(suggest(&segments, &own).is_empty());
        // Auch wenn er nur als bekannte Person da ist, der Vorname genuegt.
        let own_first = Context {
            self_name: Some("Patrick".into()),
            known_people: vec!["Patrick Wolff".into()],
            ..Context::default()
        };
        assert!(suggest(&segments, &own_first).is_empty());
        // Ohne eigenen Namen waere es ein Vorschlag.
        assert_eq!(suggest(&segments, &ctx()).len(), 1);
    }

    #[test]
    fn a_thanks_from_me_names_the_remote_speaker() {
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Das ist mein Vorschlag."),
            seg(1, 0, None, 10.5, 13.0, "Danke, Anna, klingt gut."),
        ];
        let list = suggest(&segments, &ctx());
        let s = one(&list);
        assert_eq!(
            (s.channel, s.speaker_index, s.name.as_str()),
            (1, 1, "Anna")
        );
    }

    #[test]
    fn a_long_pause_breaks_the_link() {
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Das ist mein Vorschlag."),
            seg(1, 1, Some(2), 40.0, 44.0, "Danke, Anna."),
        ];
        assert!(suggest(&segments, &ctx()).is_empty());
        // Knapp innerhalb der Grenze, aber schwaecher als direkt anschliessend.
        let far = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Das ist mein Vorschlag."),
            seg(1, 1, Some(2), 22.0, 26.0, "Danke, Anna."),
        ];
        let near = thanks_after_speaker_one("Danke, Anna.");
        let (f, n) = (suggest(&far, &ctx()), suggest(&near, &ctx()));
        assert!(one(&f).confidence < one(&n).confidence);
    }

    #[test]
    fn named_speakers_and_taken_names_get_no_suggestion() {
        let segments = thanks_after_speaker_one("Danke, Anna.");
        let named = Context {
            named: vec![((1, 1), "Ben Kaya".into())],
            ..Context::default()
        };
        assert!(
            suggest(&segments, &named).is_empty(),
            "Sprecher 1 hat einen Namen"
        );
        let taken = Context {
            named: vec![((1, 2), "Anna Berg".into())],
            ..Context::default()
        };
        assert!(
            suggest(&segments, &taken).is_empty(),
            "Anna ist schon Sprecher 2"
        );
    }

    #[test]
    fn a_speaker_is_not_named_after_their_own_thanks() {
        let segments = thanks_after_speaker_one("Danke, Anna.");
        let ctx = Context {
            named: vec![((1, 2), "Anna".into())],
            ..Context::default()
        };
        assert!(suggest(&segments, &ctx).is_empty());
    }

    #[test]
    fn known_people_complete_the_name_and_unknown_words_are_weaker() {
        let known = Context {
            known_people: vec!["Anna Berg".into(), "Ben Kaya".into()],
            ..Context::default()
        };
        let list = suggest(&thanks_after_speaker_one("Danke, Anna."), &known);
        assert_eq!(one(&list).name, "Anna Berg");
        // Zwei Annas: der Vorname allein, keine Raterei.
        let two = Context {
            known_people: vec!["Anna Berg".into(), "Anna Kaya".into()],
            ..Context::default()
        };
        let list = suggest(&thanks_after_speaker_one("Danke, Anna."), &two);
        assert_eq!(one(&list).name, "Anna");
        // Unbekannter, aber grossgeschriebener Vorname nach Dank: Vorschlag, schwaecher.
        let unknown = suggest(&thanks_after_speaker_one("Danke, Wiebke."), &ctx());
        let usual = suggest(&thanks_after_speaker_one("Danke, Anna."), &ctx());
        assert_eq!(one(&unknown).name, "Wiebke");
        assert!(unknown[0].confidence < usual[0].confidence);
    }

    #[test]
    fn common_capitalised_words_after_thanks_are_not_names() {
        for text in [
            "Danke schön. Weiter geht es.",
            "Danke, Zusammen.",
            "Vielen Dank, Wir machen weiter.",
            "Danke, Team.",
            "Danke Ihnen.",
        ] {
            assert!(
                suggest(&thanks_after_speaker_one(text), &ctx()).is_empty(),
                "{text}"
            );
        }
    }

    #[test]
    fn self_introductions_name_the_speaker_but_only_real_names() {
        let segments = vec![seg(
            0,
            1,
            Some(1),
            0.0,
            5.0,
            "Hallo zusammen, ich bin Anna von der Buchhaltung.",
        )];
        let list = suggest(&segments, &ctx());
        let s = one(&list);
        assert_eq!((s.speaker_index, s.name.as_str()), (1, "Anna"));
        assert_eq!(s.evidence[0].rule, "intro");
        let job = vec![seg(
            0,
            1,
            Some(1),
            0.0,
            5.0,
            "Ich bin Ingenieur und kümmere mich um die Anlage.",
        )];
        assert!(suggest(&job, &ctx()).is_empty());
    }

    #[test]
    fn targets_without_a_speaker_number_cannot_be_named() {
        // Der Angesprochene ist "Ich" (Kanal 0 ohne Trennung): nichts zu benennen.
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 6.0, "Patrick, was meinst du dazu?"),
            seg(1, 0, None, 6.5, 12.0, "Das passt so."),
        ];
        assert!(suggest(&segments, &ctx()).is_empty());
    }

    #[test]
    fn a_plain_vocative_alone_is_too_weak_but_a_greeting_needs_a_known_name() {
        let weak = vec![
            seg(0, 1, Some(1), 0.0, 6.0, "Anna, das Protokoll liegt vor."),
            seg(1, 1, Some(2), 6.5, 12.0, "Ja, gut."),
        ];
        assert!(suggest(&weak, &ctx()).is_empty());
        let hello = vec![
            seg(0, 1, Some(1), 0.0, 3.0, "Hallo Anna."),
            seg(1, 1, Some(2), 3.5, 8.0, "Hallo, schön, dabei zu sein."),
        ];
        assert!(
            suggest(&hello, &ctx()).is_empty(),
            "eine Quelle reicht nicht"
        );
        let twice = vec![
            seg(0, 1, Some(1), 0.0, 3.0, "Hallo Anna."),
            seg(1, 1, Some(2), 3.5, 8.0, "Hallo, schön, dabei zu sein."),
            seg(
                2,
                1,
                Some(1),
                8.5,
                12.0,
                "Anna, kannst du uns die Zahlen nennen?",
            ),
            seg(3, 1, Some(2), 12.5, 18.0, "Gern."),
        ];
        let list = suggest(&twice, &ctx());
        assert_eq!(
            (one(&list).speaker_index, one(&list).name.as_str()),
            (2, "Anna")
        );
        assert_eq!(one(&list).evidence.len(), 2);
    }

    #[test]
    fn the_dismiss_key_ignores_case_and_accents() {
        assert_eq!(dismiss_key(1, 2, "André"), dismiss_key(1, 2, "andre"));
        assert_ne!(dismiss_key(1, 2, "Anna"), dismiss_key(1, 3, "Anna"));
    }

    #[test]
    fn the_result_is_sorted_and_has_at_most_one_name_per_speaker() {
        let segments = vec![
            seg(0, 1, Some(1), 0.0, 10.0, "Erster Punkt."),
            seg(1, 1, Some(2), 10.5, 14.0, "Danke, Anna."),
            seg(2, 1, Some(2), 14.5, 20.0, "Zweiter Punkt."),
            seg(3, 1, Some(3), 20.5, 24.0, "Vielen Dank, Ben, das war klar."),
            seg(4, 1, Some(2), 24.5, 27.0, "Danke, Clara."),
            seg(5, 1, Some(1), 27.5, 30.0, "Gern geschehen."),
            seg(6, 1, Some(2), 30.5, 33.0, "Danke, Anna."),
        ];
        let list = suggest(&segments, &ctx());
        assert_eq!(list.len(), 3, "{list:?}");
        assert!(list.windows(2).all(|w| w[0].confidence >= w[1].confidence));
        let speakers: HashSet<u32> = list.iter().map(|s| s.speaker_index).collect();
        assert_eq!(speakers.len(), list.len());
    }
}
