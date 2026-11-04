//! Sprechernamen in gespeicherten Texten aktuell halten (#242).
//!
//! Die Zuordnung Sprecher -> Name steht zentral in `speakers` (Kanal, Nummer ->
//! Name). Transkript und alles, was danach erzeugt wird, lesen sie dort. Ein
//! Protokoll, KI-Notizen oder eine Antwort unter "Fragen", die VOR dem Benennen
//! entstanden sind, tragen aber noch die alte Bezeichnung als Text ("Person 2",
//! "Aufnahme", ein frueherer Name). Diese Texte bleiben gespeichert, wie sie
//! sind; beim Lesen (Anzeige, Kopieren, Export) ersetzt [`Relabeler`] jede alte
//! Bezeichnung eines Sprechers durch seinen aktuellen Namen.
//!
//! Regeln (gegen falsche Treffer):
//! - Nummerierte Standardlabels ("Person 2", "Gegenseite 1", "Raum 3") und
//!   fruehere Namen gelten als ganze Woerter: "Person 1" trifft nicht in
//!   "Person 10", "Anna" nicht in "Annabell".
//! - Kanalwoerter ohne Nummer ("Aufnahme", "Ich", "Gegenseite") sind gewoehnliche
//!   Woerter. Sie werden nur dort ersetzt, wo sie sicher einen Sprecher meinen:
//!   als ganze Tabellenzelle, als Sprecher am Zeilenanfang ("Aufnahme:",
//!   "**Aufnahme [01:02]:**") und hinter "Wer: ".
//! - Eine Bezeichnung, die zu mehr als einem Sprecher gehoert oder die ein
//!   anderer Sprecher heute traegt, wird nie ersetzt (lieber alt als falsch).
//! - Ein Durchgang auf dem Originaltext: eine Ersetzung wird nie erneut ersetzt.
//!
//! Datenschutz: kein Name und kein Text gelangt ins Log.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use super::notes::model::ActionItem;
use super::speakers::{SpeakerDirectory, WHOLE_CHANNEL};
use super::store::{MeetingDocument, MeetingStore};

/// `metadata_json`-Schluessel: fruehere Namen je Sprecher
/// (`{"Kanal:Nummer": ["Anna", ...]}`, aelteste zuerst).
pub const FORMER_NAMES_KEY: &str = "speaker_former_names";
/// So viele fruehere Namen merkt sich ein Sprecher hoechstens.
const MAX_FORMER_NAMES: usize = 10;

/// Ein Sprecher, wie die Ersetzung ihn braucht.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelEntry {
    /// Was heute angezeigt wird (Name oder Standardlabel).
    pub label: String,
    /// Standardbezeichnung ohne Namen ("Person 2", "Aufnahme").
    pub default_label: String,
    /// Fruehere Namen, aelteste zuerst.
    pub former_names: Vec<String>,
}

/// Schluessel eines Sprechers in [`FORMER_NAMES_KEY`].
pub fn former_key(channel: u8, speaker_index: u32) -> String {
    format!("{channel}:{speaker_index}")
}

/// Fruehere Namen aller Sprecher der Besprechung (leer bei unlesbaren Daten).
pub fn former_names(store: &MeetingStore, meeting_id: &str) -> BTreeMap<String, Vec<String>> {
    store
        .metadata_json(meeting_id)
        .ok()
        .flatten()
        .and_then(|m| m.get(FORMER_NAMES_KEY).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Merkt sich `old` als frueheren Namen des Sprechers. Der neue Name `new`
/// steht danach bei keinem Sprecher mehr als frueherer Name.
pub fn remember_former_name(
    store: &MeetingStore,
    meeting_id: &str,
    channel: u8,
    speaker_index: u32,
    old: &str,
    new: Option<&str>,
) -> anyhow::Result<()> {
    let mut all = former_names(store, meeting_id);
    let key = former_key(channel, speaker_index);
    let list = all.entry(key).or_default();
    list.retain(|n| n != old);
    list.push(old.to_string());
    let excess = list.len().saturating_sub(MAX_FORMER_NAMES);
    list.drain(..excess);
    if let Some(new) = new {
        for names in all.values_mut() {
            names.retain(|n| n != new);
        }
    }
    all.retain(|_, names| !names.is_empty());
    store.set_metadata_key(meeting_id, FORMER_NAMES_KEY, json!(all))
}

/// Nach dem Zusammenfuehren (`from` geht in `into` auf): was Texte fuer
/// `from` sagen (Standardlabel, angezeigter Name, fruehere Namen), gehoert
/// jetzt zu `into`. Die Eintraege von `from` fallen weg.
pub fn carry_former_names(
    store: &MeetingStore,
    meeting_id: &str,
    channel: u8,
    from: u32,
    into: u32,
    from_labels: &[String],
) -> anyhow::Result<()> {
    let mut all = former_names(store, meeting_id);
    let mut carried = all.remove(&former_key(channel, from)).unwrap_or_default();
    carried.extend(from_labels.iter().cloned());
    let list = all.entry(former_key(channel, into)).or_default();
    for name in carried {
        let name = name.trim().to_string();
        if !name.is_empty() && !list.contains(&name) {
            list.push(name);
        }
    }
    let excess = list.len().saturating_sub(MAX_FORMER_NAMES);
    list.drain(..excess);
    all.retain(|_, names| !names.is_empty());
    store.set_metadata_key(meeting_id, FORMER_NAMES_KEY, json!(all))
}

/// Alle Sprecher der Besprechung mit aktuellem Label, Standardlabel und
/// frueheren Namen: die in den Segmenten vorkommen (ohne Sprecher: der ganze
/// Kanal) und die mit einer Zeile in `speakers`.
pub fn label_entries(store: &MeetingStore, meeting_id: &str) -> Vec<LabelEntry> {
    let directory = SpeakerDirectory::load(store, meeting_id);
    let plain = SpeakerDirectory::load_with(store, meeting_id, None).without_names();
    let former = former_names(store, meeting_id);
    let mut keys: Vec<(u8, u32)> = store
        .get_segments(meeting_id)
        .unwrap_or_default()
        .iter()
        .map(|s| (s.channel, s.speaker_index.unwrap_or(WHOLE_CHANNEL)))
        .collect();
    keys.extend(
        store
            .speaker_rows(meeting_id)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.channel, r.speaker_index)),
    );
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|(channel, index)| {
            let index_opt = (index != WHOLE_CHANNEL).then_some(index);
            LabelEntry {
                label: directory.label_for(channel, index_opt),
                default_label: plain.label_for(channel, index_opt),
                former_names: former
                    .get(&former_key(channel, index))
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// Ersetzt alte Bezeichnungen durch aktuelle Namen (siehe Modulkopf).
#[derive(Clone, Debug, Default)]
pub struct Relabeler {
    /// Ganze Woerter, laengste zuerst.
    words: Vec<(String, String)>,
    /// Kanalwoerter, nur an Sprecherstellen.
    channel_words: Vec<(String, String)>,
    /// Aktuelle Namen mit Leerzeichen ("Anna Berg"): ein frueherer Name, der
    /// nur ihr Anfang ist ("Anna"), wird dort nicht ersetzt.
    current: Vec<String>,
}

/// Ein Kanalwort ohne Nummer ("Aufnahme", "Ich", "Gegenseite"): ein
/// Standardlabel, das nicht auf eine Zahl endet.
fn is_channel_word(alias: &str, default_label: &str) -> bool {
    alias == default_label && !alias.ends_with(|c: char| c.is_ascii_digit())
}

impl Relabeler {
    /// Aus den Sprechern der Besprechung.
    pub fn new(entries: &[LabelEntry]) -> Self {
        // Bezeichnung -> (Ziel, Kanalwort?), mehrdeutige fallen weg.
        let mut owners: HashMap<&str, Vec<(&str, bool)>> = HashMap::new();
        for entry in entries {
            let candidates = std::iter::once(entry.default_label.as_str())
                .chain(entry.former_names.iter().map(String::as_str));
            for alias in candidates {
                let alias_trimmed = alias.trim();
                if alias_trimmed.is_empty() || alias_trimmed == entry.label {
                    continue;
                }
                let list = owners.entry(alias_trimmed).or_default();
                if !list.iter().any(|(target, _)| *target == entry.label) {
                    list.push((
                        entry.label.as_str(),
                        is_channel_word(alias_trimmed, &entry.default_label),
                    ));
                }
            }
        }
        let mut out = Self {
            current: entries
                .iter()
                .map(|e| e.label.clone())
                .filter(|l| l.contains(char::is_whitespace))
                .collect(),
            ..Self::default()
        };
        for (alias, list) in owners {
            let [(target, channel_word)] = list.as_slice() else {
                continue;
            };
            if entries.iter().any(|e| e.label == alias) {
                continue;
            }
            let pair = (alias.to_string(), (*target).to_string());
            if *channel_word {
                out.channel_words.push(pair);
            } else {
                out.words.push(pair);
            }
        }
        let by_length = |a: &(String, String), b: &(String, String)| {
            b.0.chars()
                .count()
                .cmp(&a.0.chars().count())
                .then_with(|| a.0.cmp(&b.0))
        };
        out.words.sort_by(by_length);
        out.channel_words.sort_by(by_length);
        out
    }

    /// Die Ersetzung fuer eine Besprechung aus dem Speicher.
    pub fn load(store: &MeetingStore, meeting_id: &str) -> Self {
        Self::new(&label_entries(store, meeting_id))
    }

    /// Gibt es nichts zu ersetzen?
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.channel_words.is_empty()
    }

    /// Ein Text mit aktuellen Namen.
    pub fn apply(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&self.apply_line(line));
        }
        out
    }

    /// Ein Feld, das als Ganzes einen Sprecher nennt (Aufgabe "Wer").
    pub fn apply_field(&self, value: &str) -> String {
        let trimmed = value.trim();
        if let Some((_, target)) = self
            .channel_words
            .iter()
            .chain(self.words.iter())
            .find(|(alias, _)| alias == trimmed)
        {
            return target.clone();
        }
        self.apply(value)
    }

    fn apply_line(&self, line: &str) -> String {
        let mut hits: Vec<(usize, usize, &str)> = Vec::new();
        self.channel_hits(line, &mut hits);
        self.word_hits(line, &mut hits);
        if hits.is_empty() {
            return line.to_string();
        }
        hits.sort_by_key(|h| h.0);
        let mut out = String::with_capacity(line.len() + 16);
        let mut pos = 0;
        for (start, end, target) in hits {
            if start < pos {
                continue;
            }
            out.push_str(&line[pos..start]);
            out.push_str(target);
            pos = end;
        }
        out.push_str(&line[pos..]);
        out
    }

    /// Kanalwoerter an Sprecherstellen: Tabellenzelle, Zeilenanfang, "Wer: ".
    fn channel_hits<'a>(&'a self, line: &str, hits: &mut Vec<(usize, usize, &'a str)>) {
        if self.channel_words.is_empty() {
            return;
        }
        // Tabellenzellen: der Inhalt zwischen zwei "|" ist genau das Wort.
        if line.trim_start().starts_with('|') {
            let bars: Vec<usize> = line.match_indices('|').map(|(i, _)| i).collect();
            for pair in bars.windows(2) {
                let (from, to) = (pair[0] + 1, pair[1]);
                let cell = &line[from..to];
                let trimmed = cell.trim();
                if let Some((_, target)) = self
                    .channel_words
                    .iter()
                    .find(|(alias, _)| alias == trimmed)
                {
                    let start = from + cell.find(trimmed).unwrap_or(0);
                    hits.push((start, start + trimmed.len(), target));
                }
            }
        }
        // Sprecher am Zeilenanfang: Einrueckung, Listen-/Zitatzeichen, "**".
        let mut start = line.len() - line.trim_start().len();
        for marker in ["- ", "* ", "> "] {
            if line[start..].starts_with(marker) {
                start += marker.len();
                break;
            }
        }
        if line[start..].starts_with("**") {
            start += 2;
        }
        for (alias, target) in &self.channel_words {
            if let Some(rest) = line[start..].strip_prefix(alias.as_str()) {
                if rest.starts_with(':')
                    || rest.starts_with("**")
                    || rest.starts_with(" [")
                    || rest.starts_with(" (")
                {
                    hits.push((start, start + alias.len(), target));
                    break;
                }
            }
        }
        // "Wer: Aufnahme" (Aufgaben im Protokoll).
        for (at, _) in line.match_indices("Wer: ") {
            let from = at + "Wer: ".len();
            for (alias, target) in &self.channel_words {
                if line[from..].starts_with(alias.as_str())
                    && !next_is_word_char(line, from + alias.len())
                {
                    hits.push((from, from + alias.len(), target));
                    break;
                }
            }
        }
    }

    /// Nummerierte Labels und fruehere Namen als ganze Woerter.
    fn word_hits<'a>(&'a self, line: &str, hits: &mut Vec<(usize, usize, &'a str)>) {
        if self.words.is_empty() {
            return;
        }
        let taken = |at: usize, hits: &Vec<(usize, usize, &str)>| {
            hits.iter().any(|(s, e, _)| at >= *s && at < *e)
        };
        let mut at = 0;
        while at < line.len() {
            let prev_ok = !prev_is_word_char(line, at);
            let mut advanced = false;
            if prev_ok && !taken(at, hits) {
                for (alias, target) in &self.words {
                    let end = at + alias.len();
                    if line[at..].starts_with(alias.as_str())
                        && !next_is_word_char(line, end)
                        // "Jürgen" -> "Jürgen Öz": steht der neue Name schon da,
                        // bleibt er stehen.
                        && !line[at..].starts_with(target.as_str())
                        && !self.current.iter().any(|name| {
                            name.len() > alias.len()
                                && line[at..].starts_with(name.as_str())
                                && !next_is_word_char(line, at + name.len())
                        })
                        && !hits.iter().any(|(s, e, _)| *s < end && at < *e)
                    {
                        hits.push((at, end, target));
                        at = end;
                        advanced = true;
                        break;
                    }
                }
            }
            if !advanced {
                at += line[at..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
}

impl Relabeler {
    /// Ein gespeichertes Dokument mit aktuellen Namen. Markdown wird als Text
    /// ersetzt; KI-Notizen (JSON) feldweise: Eintragstext und "Wer".
    pub fn document(&self, mut document: MeetingDocument) -> MeetingDocument {
        if self.is_empty() {
            return document;
        }
        if document.body_format.starts_with("markdown") {
            document.body = self.apply(&document.body);
            return document;
        }
        if let Ok(mut value) = serde_json::from_str::<Value>(&document.body) {
            self.json(&mut value);
            if let Ok(body) = serde_json::to_string(&value) {
                document.body = body;
            }
        } else {
            document.body = self.apply(&document.body);
        }
        document
    }

    /// Alle Felder `text` (Fliesstext) und `assignee` (ganzer Name) einer
    /// JSON-Struktur.
    fn json(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map.iter_mut() {
                    match (key.as_str(), inner) {
                        ("text", Value::String(s)) => *s = self.apply(s),
                        ("assignee", Value::String(s)) => *s = self.apply_field(s),
                        (_, other) => self.json(other),
                    }
                }
            }
            Value::Array(list) => list.iter_mut().for_each(|v| self.json(v)),
            _ => {}
        }
    }

    /// Aufgaben: "Wer" mit aktuellem Namen.
    pub fn action_items(&self, mut items: Vec<ActionItem>) -> Vec<ActionItem> {
        if self.is_empty() {
            return items;
        }
        for item in &mut items {
            if let Some(label) = item.assignee_label.as_mut() {
                *label = self.apply_field(label);
            }
            item.text = self.apply(&item.text);
        }
        items
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn prev_is_word_char(line: &str, at: usize) -> bool {
    line[..at].chars().next_back().is_some_and(is_word_char)
}

fn next_is_word_char(line: &str, at: usize) -> bool {
    line[at..].chars().next().is_some_and(is_word_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, default_label: &str, former: &[&str]) -> LabelEntry {
        LabelEntry {
            label: label.into(),
            default_label: default_label.into(),
            former_names: former.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn screenshot_case() -> Relabeler {
        Relabeler::new(&[
            entry("Anna Berg", "Person 1", &[]),
            entry("Bernd", "Aufnahme", &[]),
            entry("Person 2", "Person 2", &[]),
            entry("Carla", "Person 10", &["Carl"]),
        ])
    }

    #[test]
    fn numbered_labels_are_replaced_as_whole_words_only() {
        let r = screenshot_case();
        assert_eq!(
            r.apply("Person 1 stellt vor, Person 10 fragt, Person 2 antwortet."),
            "Anna Berg stellt vor, Carla fragt, Person 2 antwortet."
        );
        assert_eq!(
            r.apply("Person 1a und XPerson 1"),
            "Person 1a und XPerson 1"
        );
    }

    #[test]
    fn former_names_follow_the_rename() {
        let r = screenshot_case();
        assert_eq!(r.apply("Carl und Carlos"), "Carla und Carlos");
    }

    #[test]
    fn channel_words_only_in_speaker_positions() {
        let r = screenshot_case();
        let table = "| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Aufnahme | 03:18 | 6,7 % |";
        assert_eq!(
            r.apply(table),
            "| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Bernd | 03:18 | 6,7 % |"
        );
        assert_eq!(r.apply("Aufnahme: Guten Tag"), "Bernd: Guten Tag");
        assert_eq!(
            r.apply("**Aufnahme [01:02]:** Hallo"),
            "**Bernd [01:02]:** Hallo"
        );
        assert_eq!(
            r.apply("- Angebot schicken _(Wer: Aufnahme)_"),
            "- Angebot schicken _(Wer: Bernd)_"
        );
        // Gewoehnliches Wort im Fliesstext bleibt.
        assert_eq!(
            r.apply("Die Aufnahme beginnt um 10 Uhr."),
            "Die Aufnahme beginnt um 10 Uhr."
        );
    }

    #[test]
    fn ambiguous_or_taken_labels_stay() {
        // Person 1 heisst jetzt "Person 2"; das alte "Person 2" waere doppelt.
        let r = Relabeler::new(&[
            entry("Person 2", "Person 1", &[]),
            entry("Anna", "Person 2", &[]),
        ]);
        assert_eq!(r.apply("Person 1 und Person 2"), "Person 2 und Person 2");
        // Zwei Sprecher hiessen frueher "Max": nie ersetzen.
        let r = Relabeler::new(&[
            entry("Max A", "Person 1", &["Max"]),
            entry("Max B", "Person 2", &["Max"]),
        ]);
        assert_eq!(r.apply("Max sagt"), "Max sagt");
    }

    #[test]
    fn a_replacement_is_never_replaced_again() {
        let r = Relabeler::new(&[
            entry("Person 3 Team", "Person 1", &[]),
            entry("Bob", "Person 3", &[]),
        ]);
        assert_eq!(r.apply("Person 1"), "Person 3 Team");
    }

    #[test]
    fn nothing_named_nothing_changes() {
        let r = Relabeler::new(&[entry("Person 1", "Person 1", &[])]);
        assert!(r.is_empty());
        assert_eq!(r.apply("Person 1: hallo"), "Person 1: hallo");
    }

    #[test]
    fn umlauts_and_fields() {
        let r = Relabeler::new(&[entry("Jürgen Öz", "Gegenseite 1", &["Jürgen"])]);
        assert_eq!(
            r.apply("Jürgens Idee, Jürgen sagt"),
            "Jürgens Idee, Jürgen Öz sagt"
        );
        assert_eq!(r.apply("Jürgen Öz sagt"), "Jürgen Öz sagt");
        let r = screenshot_case();
        assert_eq!(r.apply_field(" Aufnahme "), "Bernd");
        assert_eq!(r.apply_field("Person 1"), "Anna Berg");
    }

    #[test]
    fn a_former_name_is_kept_where_it_starts_another_current_name() {
        let r = Relabeler::new(&[
            entry("Maria", "Person 1", &["Anna"]),
            entry("Anna Berg", "Person 2", &[]),
        ]);
        assert_eq!(r.apply("Anna fragt Anna Berg"), "Maria fragt Anna Berg");
    }
}
