//! Lesestatistik (lva#354, Spec `docs/superpowers/specs/2026-11-06-lesestatistik.md`).
//!
//! Je Gerät eine Datei `<bibliothek>/lesestatistik/<gerät>.json` mit Zählern
//! je Tag und `<buch>|<format>|<sprache>`. Jedes Gerät schreibt nur seine
//! eigene Datei, Zähler wachsen nur. Zusammenführen ist deshalb konfliktfrei:
//! je Eimer gilt die jüngere Epoche (`since`), bei gleicher Epoche das Maximum.
//!
//! Löschen setzt einen Grabstein je Tag (`cleared`, ms). Eimer, deren Epoche
//! nicht jünger ist als der höchste Grabstein des Tages (über alle Geräte),
//! zählen nicht mehr; das eigene Gerät beginnt danach eine neue Epoche.
//!
//! Der Kern (alles außer den Befehlen) kennt kein Tauri, damit dieselben
//! Regeln auf dem iPhone in Swift nachgebaut und mit gemeinsamen
//! Testvektoren geprüft werden können.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const COLLECTION_READLOG: &str = "readlog";
pub const FORMATS: [&str; 3] = ["lesen", "hoeren", "vorlesen"];
const DIR: &str = "lesestatistik";
const DEVICE_FILE: &str = "geraet";
/// Mehr als eine Stunde je Meldung ist ein Messfehler (Uhr gesprungen).
const MAX_SEC_PER_DELTA: u64 = 3600;
/// Mehr als ein Tag je Eimer ist unmöglich.
const MAX_SEC_PER_BUCKET: u64 = 86_400;
const MAX_CHARS_PER_DELTA: u64 = 2_000_000;

// ---------------------------------------------------------------------------
// Datenformate
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Bucket {
    #[serde(default)]
    pub sec: u64,
    #[serde(default)]
    pub chars: u64,
    #[serde(default)]
    pub audio_ms: u64,
    /// Beginn der Epoche (Unix-ms). Ein Grabstein ab diesem Zeitpunkt setzt zurück.
    #[serde(default)]
    pub since: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DeviceLog {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub days: BTreeMap<String, BTreeMap<String, Bucket>>,
    #[serde(default)]
    pub cleared: BTreeMap<String, i64>,
}

fn one() -> u32 {
    1
}

/// Eine Meldung der Oberfläche: Zuwachs seit der letzten Meldung.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type, PartialEq)]
pub struct ReadingDelta {
    /// Ortsdatum `JJJJ-MM-TT`.
    pub day: String,
    pub book_id: String,
    /// `lesen`, `hoeren` oder `vorlesen`.
    pub format: String,
    /// Sprachcode (`de`, `en`, …) oder `und`.
    pub lang: String,
    pub sec: f64,
    #[serde(default)]
    pub chars: f64,
    #[serde(default)]
    pub audio_ms: f64,
}

/// Eine Zeile der Auswertung, über alle Geräte summiert.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type, PartialEq)]
pub struct ReadingRow {
    pub day: String,
    pub book_id: String,
    pub format: String,
    pub lang: String,
    pub sec: f64,
    pub chars: f64,
    pub audio_ms: f64,
}

// ---------------------------------------------------------------------------
// Prüfungen
// ---------------------------------------------------------------------------

pub fn valid_day(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && (1..=12).contains(&d[5..7].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&d[8..10].parse::<u32>().unwrap_or(0))
}

fn valid_month(m: &str) -> bool {
    valid_day(&format!("{m}-01"))
}

pub fn valid_device(id: &str) -> bool {
    (4..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_book(id: &str) -> bool {
    id.strip_prefix("book_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn valid_lang(l: &str) -> bool {
    (2..=16).contains(&l.len())
        && l.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Schlüssel eines Eimers oder `None`, wenn ein Teil ungültig ist.
pub fn bucket_key(book: &str, format: &str, lang: &str) -> Option<String> {
    (valid_book(book) && FORMATS.contains(&format) && valid_lang(lang))
        .then(|| format!("{book}|{format}|{lang}"))
}

fn split_key(k: &str) -> Option<(&str, &str, &str)> {
    let mut it = k.splitn(3, '|');
    let (b, f, l) = (it.next()?, it.next()?, it.next()?);
    bucket_key(b, f, l).map(|_| (b, f, l))
}

fn count(v: f64, max: u64) -> u64 {
    if v.is_finite() && v > 0.0 {
        (v.round() as u64).min(max)
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Kern
// ---------------------------------------------------------------------------

/// Zuwächse einbuchen. Ungültige Meldungen werden verworfen; die Zahl der
/// angenommenen kommt zurück.
pub fn add(log: &mut DeviceLog, deltas: &[ReadingDelta], now: i64) -> usize {
    let mut taken = 0;
    for d in deltas {
        let Some(key) = bucket_key(&d.book_id, &d.format, &d.lang) else {
            continue;
        };
        if !valid_day(&d.day) {
            continue;
        }
        let sec = count(d.sec, MAX_SEC_PER_DELTA);
        let chars = count(d.chars, MAX_CHARS_PER_DELTA);
        let audio_ms = count(d.audio_ms, MAX_SEC_PER_DELTA * 1000);
        if sec == 0 && chars == 0 && audio_ms == 0 {
            continue;
        }
        let tomb = log.cleared.get(&d.day).copied();
        let day = log.days.entry(d.day.clone()).or_default();
        let b = day.entry(key).or_insert_with(|| Bucket {
            since: now,
            ..Default::default()
        });
        if tomb.is_some_and(|t| b.since <= t) {
            *b = Bucket {
                since: now.max(tomb.unwrap_or(0) + 1),
                ..Default::default()
            };
        }
        b.sec = (b.sec + sec).min(MAX_SEC_PER_BUCKET);
        b.chars = b.chars.saturating_add(chars);
        b.audio_ms = b.audio_ms.saturating_add(audio_ms);
        taken += 1;
    }
    taken
}

fn merge_bucket(a: &mut Bucket, b: &Bucket) {
    if b.since > a.since {
        *a = b.clone();
    } else if b.since == a.since {
        a.sec = a.sec.max(b.sec);
        a.chars = a.chars.max(b.chars);
        a.audio_ms = a.audio_ms.max(b.audio_ms);
    }
}

/// Zwei Stände desselben Geräts zusammenführen (kommutativ, idempotent).
pub fn merge(into: &mut DeviceLog, other: &DeviceLog) {
    for (day, buckets) in &other.days {
        if !valid_day(day) {
            continue;
        }
        let mine = into.days.entry(day.clone()).or_default();
        for (k, b) in buckets {
            if split_key(k).is_none() {
                continue;
            }
            match mine.get_mut(k) {
                Some(a) => merge_bucket(a, b),
                None => {
                    mine.insert(k.clone(), b.clone());
                }
            }
        }
    }
    for (day, t) in &other.cleared {
        if valid_day(day) {
            let e = into.cleared.entry(day.clone()).or_insert(*t);
            *e = (*e).max(*t);
        }
    }
}

/// Höchster Grabstein je Tag über alle Geräte.
pub fn tombstones(logs: &[DeviceLog]) -> BTreeMap<String, i64> {
    let mut out: BTreeMap<String, i64> = BTreeMap::new();
    for l in logs {
        for (d, t) in &l.cleared {
            let e = out.entry(d.clone()).or_insert(*t);
            *e = (*e).max(*t);
        }
    }
    out
}

/// Grabsteine anderer Geräte übernehmen und eigene, nun ungültige Eimer
/// entfernen. Liefert, ob sich etwas geändert hat.
pub fn prune(own: &mut DeviceLog, tombs: &BTreeMap<String, i64>) -> bool {
    let mut changed = false;
    for (d, t) in tombs {
        let e = own.cleared.entry(d.clone()).or_insert(*t);
        if *t > *e {
            *e = *t;
            changed = true;
        }
        if let Some(day) = own.days.get_mut(d) {
            let n = day.len();
            day.retain(|_, b| b.since > *t);
            changed |= day.len() != n;
        }
    }
    own.days.retain(|_, b| !b.is_empty());
    changed
}

/// Alle Geräte summiert, ohne gelöschte Eimer; sortiert nach Tag und Schlüssel.
pub fn aggregate(logs: &[DeviceLog]) -> Vec<ReadingRow> {
    let tombs = tombstones(logs);
    let mut sum: BTreeMap<(String, String), (u64, u64, u64)> = BTreeMap::new();
    for l in logs {
        for (day, buckets) in &l.days {
            let t = tombs.get(day).copied();
            for (k, b) in buckets {
                if split_key(k).is_none() || t.is_some_and(|t| b.since <= t) {
                    continue;
                }
                let e = sum.entry((day.clone(), k.clone())).or_default();
                e.0 += b.sec;
                e.1 += b.chars;
                e.2 += b.audio_ms;
            }
        }
    }
    sum.into_iter()
        .filter_map(|((day, k), (sec, chars, audio_ms))| {
            let (b, f, l) = split_key(&k)?;
            Some(ReadingRow {
                day,
                book_id: b.into(),
                format: f.into(),
                lang: l.into(),
                sec: sec as f64,
                chars: chars as f64,
                audio_ms: audio_ms as f64,
            })
        })
        .collect()
}

/// Tage im Bereich `from..=to` löschen (alle Geräte, über Grabsteine).
/// `book` begrenzt nicht: ein Grabstein gilt für den ganzen Tag.
pub fn clear_range(
    own: &mut DeviceLog,
    logs: &[DeviceLog],
    from: &str,
    to: &str,
    now: i64,
) -> usize {
    let mut days: BTreeSet<String> = BTreeSet::new();
    for l in logs.iter().chain(std::iter::once(&*own)) {
        days.extend(
            l.days
                .keys()
                .filter(|d| d.as_str() >= from && d.as_str() <= to)
                .cloned(),
        );
    }
    for d in &days {
        let e = own.cleared.entry(d.clone()).or_insert(now);
        *e = (*e).max(now);
        own.days.remove(d);
    }
    days.len()
}

// ---------------------------------------------------------------------------
// Sync-Objekte: je Gerät und Monat eines
// ---------------------------------------------------------------------------

pub fn object_id(device: &str, month: &str) -> String {
    let h = Sha256::digest(format!("{device}|{month}").as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("r_{}", &hex[..40])
}

/// Sync-Objekte eines Gerätestands (id, Wert), je Monat.
pub fn month_objects(log: &DeviceLog) -> Vec<(String, Value)> {
    let mut months: BTreeSet<&str> = BTreeSet::new();
    months.extend(log.days.keys().filter(|d| valid_day(d)).map(|d| &d[..7]));
    months.extend(log.cleared.keys().filter(|d| valid_day(d)).map(|d| &d[..7]));
    months
        .into_iter()
        .map(|m| {
            let days: BTreeMap<_, _> = log.days.iter().filter(|(d, _)| d.starts_with(m)).collect();
            let cleared: BTreeMap<_, _> = log
                .cleared
                .iter()
                .filter(|(d, _)| d.starts_with(m))
                .collect();
            (
                object_id(&log.device, m),
                json!({ "device": log.device, "month": m, "days": days, "cleared": cleared }),
            )
        })
        .collect()
}

/// Ein empfangenes Objekt prüfen und als Gerätestand (nur dieser Monat) liefern.
pub fn parse_object(id: &str, value: &Value) -> Result<DeviceLog, String> {
    let device = value
        .get("device")
        .and_then(Value::as_str)
        .filter(|d| valid_device(d))
        .ok_or("Lesestatistik: Gerät fehlt oder ungültig")?;
    let month = value
        .get("month")
        .and_then(Value::as_str)
        .filter(|m| valid_month(m))
        .ok_or("Lesestatistik: Monat fehlt oder ungültig")?;
    if object_id(device, month) != id {
        return Err("Lesestatistik: Kennung passt nicht zu Gerät und Monat".into());
    }
    let mut log: DeviceLog = serde_json::from_value(json!({
        "device": device,
        "days": value.get("days").cloned().unwrap_or(json!({})),
        "cleared": value.get("cleared").cloned().unwrap_or(json!({})),
    }))
    .map_err(|e| format!("Lesestatistik unlesbar: {e}"))?;
    log.days.retain(|d, _| valid_day(d) && d.starts_with(month));
    log.cleared
        .retain(|d, _| valid_day(d) && d.starts_with(month));
    Ok(log)
}

// ---------------------------------------------------------------------------
// Ablage
// ---------------------------------------------------------------------------

pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

fn log_path(root: &Path, device: &str) -> PathBuf {
    dir(root).join(format!("{device}.json"))
}

/// Eigene Gerätekennung; beim ersten Mal erzeugt.
pub fn own_device(root: &Path) -> Result<String, String> {
    let d = dir(root);
    std::fs::create_dir_all(&d).map_err(|e| format!("Lesestatistik-Ordner: {e}"))?;
    let p = d.join(DEVICE_FILE);
    if let Ok(raw) = std::fs::read_to_string(&p) {
        let id = raw.trim().to_string();
        if valid_device(&id) {
            return Ok(id);
        }
    }
    let mut bytes = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    crate::commands::pages::write_atomic(&p, id.as_bytes())?;
    Ok(id)
}

/// Stand eines Geräts; fehlt die Datei, ist er leer. Unlesbar → beiseitelegen.
pub fn load(root: &Path, device: &str) -> Result<DeviceLog, String> {
    let p = log_path(root, device);
    let raw = match std::fs::read_to_string(&p) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DeviceLog {
                version: 1,
                device: device.into(),
                ..Default::default()
            })
        }
        Err(e) => return Err(format!("Lesestatistik nicht lesbar: {e}")),
    };
    match serde_json::from_str::<DeviceLog>(&raw) {
        Ok(mut l) => {
            l.device = device.into();
            Ok(l)
        }
        Err(e) => {
            let aside = p.with_extension(format!("json.defekt-{}", now_ms()));
            std::fs::rename(&p, &aside).map_err(|r| {
                format!("Lesestatistik beschädigt ({e}), Sichern fehlgeschlagen: {r}")
            })?;
            log::warn!("Lesestatistik {device} beschädigt, beiseitegelegt");
            Ok(DeviceLog {
                version: 1,
                device: device.into(),
                ..Default::default()
            })
        }
    }
}

pub fn store(root: &Path, log: &DeviceLog) -> Result<(), String> {
    if !valid_device(&log.device) {
        return Err("Lesestatistik: ungültige Gerätekennung".into());
    }
    std::fs::create_dir_all(dir(root)).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec(log).map_err(|e| e.to_string())?;
    crate::commands::pages::write_atomic(&log_path(root, &log.device), &json)
}

/// Alle Gerätestände im Ordner.
pub fn load_all(root: &Path) -> Result<Vec<DeviceLog>, String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir(root)) else {
        return Ok(out);
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(dev) = name.strip_suffix(".json").filter(|d| valid_device(d)) {
            out.push(load(root, dev)?);
        }
    }
    out.sort_by(|a, b| a.device.cmp(&b.device));
    Ok(out)
}

/// Sync: Objekte aller Gerätestände (das Hub-Abbild bleibt so deckungsgleich).
pub fn sync_objects(root: &Path) -> Result<Vec<(String, Value)>, String> {
    Ok(load_all(root)?.iter().flat_map(month_objects).collect())
}

/// Sync: ein empfangenes Objekt einpflegen. Liefert, ob sich etwas änderte.
pub fn sync_apply(root: &Path, id: &str, value: &Value) -> Result<bool, String> {
    let incoming = parse_object(id, value)?;
    let mut target = load(root, &incoming.device)?;
    let before = target.clone();
    merge(&mut target, &incoming);
    let mut changed = target != before;
    if changed {
        store(root, &target)?;
    }
    // Grabsteine anderer Geräte gelten auch für die eigenen Eimer.
    let own = own_device(root)?;
    if own != incoming.device && !incoming.cleared.is_empty() {
        let mut mine = load(root, &own)?;
        if prune(&mut mine, &incoming.cleared) {
            store(root, &mine)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// KI-Einordnung (nur Buchmetadaten, nie Lesedaten)
// ---------------------------------------------------------------------------

/// Antwort der Einordnung. `category` ist ein Schlüssel aus der übergebenen
/// Liste oder `None`.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type, PartialEq)]
pub struct Classification {
    pub category: Option<String>,
    pub topics: Vec<String>,
    pub confidence: f64,
}

/// Eine wählbare Kategorie (`key` + Anzeigename) für die Anfrage.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type, PartialEq)]
pub struct CategoryChoice {
    pub key: String,
    pub label: String,
}

/// Was an das Modell geht: genau diese Felder (Codex A3), gekürzt.
#[derive(Serialize, Deserialize, Clone, Debug, specta::Type, PartialEq)]
pub struct ClassifyInput {
    pub title: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    /// Anfang des Buchtextes, höchstens 2 000 Zeichen.
    pub sample: Option<String>,
    pub categories: Vec<CategoryChoice>,
    /// Bereits verwendete Themen, damit gleiche Begriffe wiederkehren.
    pub known_topics: Vec<String>,
}

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn classify_prompt(input: &ClassifyInput) -> String {
    let cats: Vec<String> = input
        .categories
        .iter()
        .take(60)
        .map(|c| format!("- {}: {}", cut(&c.key, 60), cut(&c.label, 80)))
        .collect();
    let known: Vec<String> = input
        .known_topics
        .iter()
        .take(80)
        .map(|t| cut(t, 40))
        .collect();
    format!(
        "Ordne das Buch genau einer Kategorie zu und nenne 2 bis 4 kurze Themen \
(Schlagworte auf Deutsch, je höchstens drei Wörter). Verwende bereits bekannte \
Themen, wenn sie passen. Antworte NUR mit JSON: \
{{\"category\": \"<schlüssel oder null>\", \"topics\": [\"…\"], \"confidence\": 0.0}}\n\n\
Kategorien:\n{}\n\nBekannte Themen: {}\n\nTitel: {}\nAutor: {}\nKlappentext: {}\nTextanfang: {}",
        cats.join("\n"),
        if known.is_empty() {
            "–".into()
        } else {
            known.join(", ")
        },
        cut(&input.title, 300),
        cut(&input.authors.join(", "), 200),
        input
            .description
            .as_deref()
            .map(|d| cut(d, 1500))
            .unwrap_or_else(|| "–".into()),
        input
            .sample
            .as_deref()
            .map(|d| cut(d, 2000))
            .unwrap_or_else(|| "–".into()),
    )
}

/// Antwort lesen: erstes JSON-Objekt im Text; unbekannte Kategorie → `None`,
/// Themen bereinigt (höchstens 5, je 40 Zeichen), Sicherheit auf 0..1.
pub fn parse_classification(answer: &str, allowed: &[CategoryChoice]) -> Option<Classification> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    let v: Value = serde_json::from_str(answer.get(start..=end)?).ok()?;
    let category = v
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .and_then(|c| allowed.iter().find(|a| a.key == c || a.label == c))
        .map(|a| a.key.clone());
    let mut seen = BTreeSet::new();
    let topics: Vec<String> = v
        .get("topics")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|t| cut(t.trim(), 40))
                .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
                .take(5)
                .collect()
        })
        .unwrap_or_default();
    let confidence = v
        .get("confidence")
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    (category.is_some() || !topics.is_empty()).then_some(Classification {
        category,
        topics,
        confidence,
    })
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

mod commands {
    use super::*;
    use tauri::AppHandle;

    /// Zuwächse einbuchen (gebündelt von der Oberfläche).
    #[tauri::command]
    #[specta::specta]
    pub fn reading_log_add(app: AppHandle, deltas: Vec<ReadingDelta>) -> Result<u32, String> {
        if deltas.is_empty() || crate::settings::get_settings(&app).reading_stats_paused {
            return Ok(0);
        }
        let root = crate::library::sync_root(&app)?;
        let _guard = crate::library::sync_lock();
        let device = own_device(&root)?;
        let mut log = load(&root, &device)?;
        let tombs = tombstones(&load_all(&root)?);
        prune(&mut log, &tombs);
        let n = add(&mut log, &deltas, now_ms());
        store(&root, &log)?;
        Ok(n as u32)
    }

    /// Einstellungen der Lesestatistik auf einmal setzen.
    #[tauri::command]
    #[specta::specta]
    pub fn change_reading_stats_setting(
        app: AppHandle,
        goal_minutes: u32,
        goal_days: u8,
        sync: bool,
        ai: bool,
        paused: bool,
    ) -> Result<(), String> {
        let mut s = crate::settings::get_settings(&app);
        s.reading_goal_minutes = goal_minutes.min(600);
        s.reading_goal_days = if goal_days & 0x7f == 0 {
            0x7f
        } else {
            goal_days & 0x7f
        };
        s.reading_stats_sync = sync;
        s.reading_stats_ai = ai;
        s.reading_stats_paused = paused;
        crate::settings::write_settings(&app, s);
        Ok(())
    }

    /// Ein Buch einordnen lassen (Modell der Funktion „Vorlesen/Bibliothek“).
    #[tauri::command]
    #[specta::specta]
    pub async fn reading_classify(
        app: AppHandle,
        input: ClassifyInput,
    ) -> Result<Classification, String> {
        let all = crate::settings::get_settings(&app);
        if !all.reading_stats_ai {
            return Err("reading_classify:off".into());
        }
        let settings = all.for_function(crate::settings::FN_READ_ALOUD);
        let answer = crate::summarizer::ask_llm(&settings, classify_prompt(&input)).await?;
        parse_classification(&answer, &input.categories)
            .ok_or_else(|| "reading_classify:unreadable".into())
    }

    /// Auswertung aller Geräte.
    #[tauri::command]
    #[specta::specta]
    pub fn reading_log_rows(app: AppHandle) -> Result<Vec<ReadingRow>, String> {
        let root = crate::library::sync_root(&app)?;
        let _guard = crate::library::sync_lock();
        Ok(aggregate(&load_all(&root)?))
    }

    /// Tage `from..=to` auf allen Geräten löschen. Liefert die Zahl der Tage.
    #[tauri::command]
    #[specta::specta]
    pub fn reading_log_clear(app: AppHandle, from: String, to: String) -> Result<u32, String> {
        if !valid_day(&from) || !valid_day(&to) || from > to {
            return Err("Ungültiger Zeitraum".into());
        }
        let root = crate::library::sync_root(&app)?;
        let _guard = crate::library::sync_lock();
        let device = own_device(&root)?;
        let all = load_all(&root)?;
        let mut log = load(&root, &device)?;
        let n = clear_range(&mut log, &all, &from, &to, now_ms());
        store(&root, &log)?;
        Ok(n as u32)
    }
}
pub use commands::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: &str, book: &str, f: &str, sec: f64, chars: f64) -> ReadingDelta {
        ReadingDelta {
            day: day.into(),
            book_id: book.into(),
            format: f.into(),
            lang: "de".into(),
            sec,
            chars,
            audio_ms: 0.0,
        }
    }

    fn dev(id: &str) -> DeviceLog {
        DeviceLog {
            version: 1,
            device: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn einbuchen_summiert_und_verwirft_ungueltiges() {
        let mut l = dev("pc01");
        let n = add(
            &mut l,
            &[
                d("2026-11-06", "book_1", "lesen", 30.0, 900.0),
                d("2026-11-06", "book_1", "lesen", 12.4, 100.0),
                d("2026-11-06", "book_1", "tanzen", 5.0, 0.0),
                d("2026-13-01", "book_1", "lesen", 5.0, 0.0),
                d("2026-11-06", "../x", "lesen", 5.0, 0.0),
                d("2026-11-06", "book_1", "hoeren", f64::NAN, 0.0),
                d("2026-11-06", "book_1", "hoeren", 99_999.0, 0.0),
            ],
            1000,
        );
        assert_eq!(n, 3);
        let b = &l.days["2026-11-06"]["book_1|lesen|de"];
        assert_eq!((b.sec, b.chars, b.since), (42, 1000, 1000));
        assert_eq!(l.days["2026-11-06"]["book_1|hoeren|de"].sec, 3600);
    }

    #[test]
    fn zusammenfuehren_ist_kommutativ_und_idempotent() {
        let mut a = dev("pc01");
        add(
            &mut a,
            &[d("2026-11-05", "book_1", "lesen", 60.0, 10.0)],
            10,
        );
        let mut b = a.clone();
        add(&mut b, &[d("2026-11-05", "book_1", "lesen", 30.0, 5.0)], 20);
        add(
            &mut a,
            &[d("2026-11-06", "book_2", "hoeren", 10.0, 0.0)],
            30,
        );
        let mut ab = a.clone();
        merge(&mut ab, &b);
        let mut ba = b.clone();
        merge(&mut ba, &a);
        assert_eq!(ab, ba);
        let again = ab.clone();
        merge(&mut ab, &again);
        assert_eq!(ab, again);
        assert_eq!(ab.days["2026-11-05"]["book_1|lesen|de"].sec, 90);
    }

    #[test]
    fn geraete_werden_summiert_nicht_doppelt() {
        let mut pc = dev("pc01");
        let mut phone = dev("ph01");
        add(
            &mut pc,
            &[d("2026-11-06", "book_1", "lesen", 600.0, 0.0)],
            1,
        );
        add(
            &mut phone,
            &[d("2026-11-06", "book_1", "lesen", 300.0, 0.0)],
            1,
        );
        // Das Telefon kennt den PC-Stand doppelt (zweimal empfangen).
        let rows = aggregate(&[pc.clone(), phone.clone()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sec, 900.0);
        let mut pc2 = pc.clone();
        merge(&mut pc2, &pc);
        assert_eq!(aggregate(&[pc2, phone])[0].sec, 900.0);
    }

    #[test]
    fn grabstein_loescht_auf_allen_geraeten_und_neue_epoche_zaehlt() {
        let mut pc = dev("pc01");
        let mut phone = dev("ph01");
        add(
            &mut pc,
            &[d("2026-11-06", "book_1", "lesen", 600.0, 0.0)],
            100,
        );
        add(
            &mut phone,
            &[d("2026-11-06", "book_1", "hoeren", 300.0, 0.0)],
            100,
        );
        add(
            &mut phone,
            &[d("2026-11-07", "book_1", "hoeren", 50.0, 0.0)],
            100,
        );
        let n = clear_range(&mut pc, &[phone.clone()], "2026-11-01", "2026-11-06", 200);
        assert_eq!(n, 1);
        let rows = aggregate(&[pc.clone(), phone.clone()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].day, "2026-11-07");
        // Telefon erhält den Grabstein und beginnt danach neu.
        assert!(prune(&mut phone, &pc.cleared));
        add(
            &mut phone,
            &[d("2026-11-06", "book_1", "hoeren", 20.0, 0.0)],
            300,
        );
        let rows = aggregate(&[pc.clone(), phone.clone()]);
        let six: Vec<_> = rows.iter().filter(|r| r.day == "2026-11-06").collect();
        assert_eq!(six.len(), 1);
        assert_eq!(six[0].sec, 20.0);
        // Gleiche Uhrzeit wie der Grabstein: trotzdem neue Epoche.
        let mut pc3 = pc.clone();
        add(
            &mut pc3,
            &[d("2026-11-06", "book_2", "lesen", 5.0, 0.0)],
            200,
        );
        assert!(pc3.days["2026-11-06"]["book_2|lesen|de"].since > 200);
    }

    #[test]
    fn einordnung_antwort_lesen() {
        let cats = vec![
            CategoryChoice {
                key: "kommunikation".into(),
                label: "Kommunikation & Rhetorik".into(),
            },
            CategoryChoice {
                key: "fuehrung".into(),
                label: "Führung & Management".into(),
            },
        ];
        let a = "Hier: ```json\n{\"category\": \"kommunikation\", \"topics\": [\"Rhetorik\", \"rhetorik\", \" Stimme \"], \"confidence\": 1.7}\n```";
        let c = parse_classification(a, &cats).unwrap();
        assert_eq!(c.category.as_deref(), Some("kommunikation"));
        assert_eq!(c.topics, vec!["Rhetorik", "Stimme"]);
        assert_eq!(c.confidence, 1.0);
        // Anzeigename statt Schlüssel wird erkannt, Unbekanntes verworfen.
        let c = parse_classification(r#"{"category":"Führung & Management","topics":[]}"#, &cats)
            .unwrap();
        assert_eq!(c.category.as_deref(), Some("fuehrung"));
        assert!(parse_classification(r#"{"category":"Kochen","topics":[]}"#, &cats).is_none());
        assert!(parse_classification("kein json", &cats).is_none());
        let p = classify_prompt(&ClassifyInput {
            title: "T".into(),
            authors: vec![],
            description: None,
            sample: Some("x".repeat(5000)),
            categories: cats.clone(),
            known_topics: vec![],
        });
        assert!(p.contains("- fuehrung: Führung & Management"));
        assert!(p.len() < 4000);
    }

    #[test]
    fn sync_objekte_hin_und_zurueck() {
        let mut pc = dev("pc01");
        add(&mut pc, &[d("2026-10-31", "book_1", "lesen", 60.0, 0.0)], 1);
        add(
            &mut pc,
            &[d("2026-11-01", "book_1", "vorlesen", 60.0, 0.0)],
            1,
        );
        pc.cleared.insert("2026-11-02".into(), 5);
        let objs = month_objects(&pc);
        assert_eq!(objs.len(), 2);
        let mut back = dev("pc01");
        for (id, v) in &objs {
            let part = parse_object(id, v).unwrap();
            merge(&mut back, &part);
        }
        assert_eq!(back, pc);
        // Fremde Kennung oder Monat außerhalb werden abgewiesen bzw. gefiltert.
        assert!(parse_object("r_00", &objs[0].1).is_err());
        let mut v = objs[1].1.clone();
        v["days"]["2026-10-01"] = json!({ "book_9|lesen|de": { "sec": 1, "since": 1 } });
        let p = parse_object(&objs[1].0, &v).unwrap();
        assert!(!p.days.contains_key("2026-10-01"));
    }

    #[test]
    fn ablage_und_sync_anwenden() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let me = own_device(root).unwrap();
        assert_eq!(own_device(root).unwrap(), me);
        let mut mine = load(root, &me).unwrap();
        add(
            &mut mine,
            &[d("2026-11-06", "book_1", "lesen", 60.0, 0.0)],
            10,
        );
        store(root, &mine).unwrap();
        let mut other = dev("ph01");
        add(
            &mut other,
            &[d("2026-11-06", "book_1", "hoeren", 30.0, 0.0)],
            10,
        );
        other.cleared.insert("2026-11-06".into(), 20);
        for (id, v) in month_objects(&other) {
            assert!(sync_apply(root, &id, &v).unwrap());
            assert!(!sync_apply(root, &id, &v).unwrap());
        }
        // Grabstein des Telefons hat den eigenen Eimer entfernt.
        assert!(load(root, &me).unwrap().days.is_empty());
        assert!(aggregate(&load_all(root).unwrap()).is_empty());
        // Beschädigte Datei wird beiseitegelegt.
        std::fs::write(dir(root).join("kaputt1.json"), "{").unwrap();
        assert!(load(root, "kaputt1").unwrap().days.is_empty());
    }
}
