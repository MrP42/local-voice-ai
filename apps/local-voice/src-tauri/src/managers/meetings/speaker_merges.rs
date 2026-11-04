//! Sprecher zusammenführen und wieder trennen (#266).
//!
//! Die Sprechertrennung zerlegt eine Person manchmal in mehrere Sprecher
//! ("Patrick Wolff" dreimal). Zusammenführen (`MeetingStore::merge_speakers`)
//! schreibt Segmente, Turns und Sprecherzeilen von `from` auf `into` um. Damit
//! das rückgängig zu machen ist, legt jede Zusammenführung in derselben
//! Transaktion einen [`MergeRecord`] in `meetings.metadata_json["speaker_merges"]`
//! ab: welche Segmente und welche Turns `from` gehörten, welche Sprecherzeilen
//! wegfielen oder umzogen, welcher Name mitging und welche früheren Namen
//! (#242) übernommen wurden. `MeetingStore::unmerge_speakers` stellt genau das
//! wieder her.
//!
//! Gültigkeit: Ein Eintrag gilt nur für die Segmentepoche, auf der er entstand.
//! Eine neue Sprechertrennung, Neu-Transkription oder ein Enddurchlauf
//! (Segmente neu geschrieben) verwirft alle Einträge der Besprechung.
//!
//! Datenschutz: Namen stehen nur in den Metadaten der Besprechung, nie im Log.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::relabel::{former_key, MAX_FORMER_NAMES};

/// `metadata_json`-Schlüssel der Zusammenführungen (älteste zuerst).
pub const MERGES_KEY: &str = "speaker_merges";
/// So viele Zusammenführungen merkt sich eine Besprechung höchstens.
pub const MAX_MERGES: usize = 50;

/// Was eine Zusammenführung geändert hat, damit "Trennen" es zurücknimmt.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeRecord {
    /// ULID; die Oberfläche trennt über diese Kennung.
    pub id: String,
    pub channel: u8,
    pub from: u32,
    pub into: u32,
    /// Segmentepoche beim Zusammenführen.
    pub epoch: u32,
    /// Zeitpunkt (Sekunden).
    pub at: i64,
    /// Bezeichnung von `from` beim Zusammenführen ("Person 3" oder Name).
    pub from_label: String,
    /// Standardbezeichnung von `from` ohne Namen ("Person 3", "Aufnahme").
    #[serde(default)]
    pub from_default_label: String,
    /// Segmente (Nummern), die `from` gehörten.
    pub segments: Vec<u32>,
    /// Positionen der Turns von `from` in `speaker_hints_json` (Kanal `channel`).
    #[serde(default)]
    pub turns: Vec<usize>,
    /// Zeilen von `from`, die als gelöscht markiert wurden.
    #[serde(default)]
    pub deleted_rows: Vec<String>,
    /// Zeile von `from`, die auf `into` umzog (`into` hatte keine).
    #[serde(default)]
    pub moved_row: Option<String>,
    /// Name, den `into` von `from` bekam (`into` hatte keinen).
    #[serde(default)]
    pub carried_name: Option<String>,
    /// Frühere Namen von `from` vor dem Zusammenführen.
    #[serde(default)]
    pub former_from: Vec<String>,
    /// Namen, die durch das Zusammenführen neu zu den früheren Namen von
    /// `into` kamen.
    #[serde(default)]
    pub carried_former: Vec<String>,
}

/// Für die Oberfläche: eine Zusammenführung, die sich trennen lässt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct SpeakerMerge {
    pub id: String,
    pub channel: u8,
    /// Der aufgegangene Sprecher.
    pub from: u32,
    /// Der Sprecher, in dem er aufging.
    pub into: u32,
    /// Bezeichnung von `from` beim Zusammenführen.
    pub from_label: String,
    /// Standardbezeichnung von `from` ohne Namen ("Person 3", "Aufnahme"):
    /// unterscheidet zusammengeführte Sprecher mit gleichem Namen.
    pub from_default_label: String,
    /// Wie viele Segmente zurückgehen.
    pub segment_count: u32,
    /// Zeitpunkt (Sekunden).
    pub at: i64,
    /// `false`: eine spätere Zusammenführung hat `into` selbst aufgenommen;
    /// erst die trennen.
    pub undoable: bool,
}

/// Die Einträge aus den Metadaten (leer bei fehlenden oder unlesbaren Daten).
pub fn records(metadata: Option<&Value>) -> Vec<MergeRecord> {
    metadata
        .and_then(|m| m.get(MERGES_KEY))
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Nur die Einträge der aktuellen Epoche.
pub fn live(records: Vec<MergeRecord>, epoch: u32) -> Vec<MergeRecord> {
    records.into_iter().filter(|r| r.epoch == epoch).collect()
}

/// Lässt sich `records[at]` trennen? Nein, wenn ein späterer Eintrag `into`
/// selbst in einen anderen Sprecher überführt hat (dann erst den trennen).
pub fn is_undoable(records: &[MergeRecord], at: usize) -> bool {
    let rec = &records[at];
    !records[at + 1..]
        .iter()
        .any(|later| later.channel == rec.channel && later.from == rec.into)
}

/// Die Einträge für die Oberfläche.
pub fn summaries(records: &[MergeRecord]) -> Vec<SpeakerMerge> {
    records
        .iter()
        .enumerate()
        .map(|(i, r)| SpeakerMerge {
            id: r.id.clone(),
            channel: r.channel,
            from: r.from,
            into: r.into,
            from_label: r.from_label.clone(),
            from_default_label: r.from_default_label.clone(),
            segment_count: r.segments.len() as u32,
            at: r.at,
            undoable: is_undoable(records, i),
        })
        .collect()
}

/// Frühere Namen (#242) beim Zusammenführen: was Texte für `from` sagen
/// (frühere Namen, Standardlabel, angezeigter Name), gehört jetzt zu `into`.
/// Gibt die Liste von `from` vor der Änderung und die bei `into` neu
/// hinzugekommenen Namen zurück.
pub fn carry_former(
    all: &mut BTreeMap<String, Vec<String>>,
    channel: u8,
    from: u32,
    into: u32,
    from_labels: &[String],
) -> (Vec<String>, Vec<String>) {
    let former_from = all.remove(&former_key(channel, from)).unwrap_or_default();
    let mut carried = former_from.clone();
    carried.extend(from_labels.iter().cloned());
    let list = all.entry(former_key(channel, into)).or_default();
    let mut added = Vec::new();
    for name in carried {
        let name = name.trim().to_string();
        if !name.is_empty() && !list.contains(&name) {
            list.push(name.clone());
            added.push(name);
        }
    }
    let excess = list.len().saturating_sub(MAX_FORMER_NAMES);
    list.drain(..excess);
    all.retain(|_, names| !names.is_empty());
    (former_from, added)
}

/// Frühere Namen beim Trennen: `from` bekommt seine Liste zurück, bei `into`
/// fallen die Namen weg, die erst durch das Zusammenführen dazukamen (was
/// danach durch Umbenennen dazukam, bleibt).
pub fn restore_former(all: &mut BTreeMap<String, Vec<String>>, rec: &MergeRecord) {
    if let Some(list) = all.get_mut(&former_key(rec.channel, rec.into)) {
        list.retain(|n| !rec.carried_former.contains(n));
    }
    if !rec.former_from.is_empty() {
        all.insert(former_key(rec.channel, rec.from), rec.former_from.clone());
    }
    all.retain(|_, names| !names.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, from: u32, into: u32) -> MergeRecord {
        MergeRecord {
            id: id.into(),
            channel: 2,
            from,
            into,
            ..Default::default()
        }
    }

    #[test]
    fn a_merge_absorbed_later_is_not_undoable_until_the_later_one_is() {
        // 3 -> 2, dann 2 -> 1: erst 2 -> 1 trennen.
        let list = vec![rec("a", 3, 2), rec("b", 2, 1)];
        assert!(!is_undoable(&list, 0));
        assert!(is_undoable(&list, 1));
        // 3 -> 1, dann 2 -> 1: beide unabhaengig.
        let list = vec![rec("a", 3, 1), rec("b", 2, 1)];
        assert!(is_undoable(&list, 0) && is_undoable(&list, 1));
        let s = summaries(&list);
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|m| m.undoable));
    }

    #[test]
    fn former_names_go_to_the_target_and_come_back() {
        let mut all = BTreeMap::from([
            ("2:3".to_string(), vec!["Pat".to_string()]),
            ("2:1".to_string(), vec!["Person 1".to_string()]),
        ]);
        let labels = vec!["Person 3".to_string(), "Patrick".to_string()];
        let (from, added) = carry_former(&mut all, 2, 3, 1, &labels);
        assert_eq!(from, ["Pat"]);
        assert_eq!(added, ["Pat", "Person 3", "Patrick"]);
        assert_eq!(all["2:1"], ["Person 1", "Pat", "Person 3", "Patrick"]);
        assert!(!all.contains_key("2:3"));

        let mut r = rec("a", 3, 1);
        r.former_from = from;
        r.carried_former = added;
        // Danach umbenannt: "Neu" kam bei 1 dazu und bleibt.
        all.get_mut("2:1").unwrap().push("Neu".into());
        restore_former(&mut all, &r);
        assert_eq!(all["2:3"], ["Pat"]);
        assert_eq!(all["2:1"], ["Person 1", "Neu"]);
    }

    #[test]
    fn records_read_only_the_current_epoch_and_survive_garbage() {
        assert!(records(None).is_empty());
        let bad = serde_json::json!({ MERGES_KEY: "kaputt" });
        assert!(records(Some(&bad)).is_empty());
        let mut a = rec("a", 2, 1);
        a.epoch = 3;
        let mut b = rec("b", 3, 1);
        b.epoch = 4;
        let meta = serde_json::json!({ MERGES_KEY: [a, b] });
        let list = live(records(Some(&meta)), 4);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "b");
    }
}
