//! Sprachfassungen eines Buchs (lva#344), Schema 2.
//!
//! Alle Sprachfassungen stehen gleichwertig in [`LibraryBook::editions`], jede
//! mit stabiler Kennung. Die Hauptfassung ist nur ein Verweis
//! ([`LibraryBook::primary`]). Die Felder `text`, `audio` und
//! `language` des Buchs sind ihre **Arbeitskopie**: bestehende Abläufe (Hörbuch
//! anhängen, Transkription, Zusammenführen von Teilen) ändern weiter nur
//! diese Felder, und [`normalize`] überträgt sie beim Lesen und Schreiben in
//! die Hauptfassung. Altgeräte ohne Schema 2 sehen so weiter ein gültiges Buch.
//!
//! Spezifikation: `docs/superpowers/specs/2026-11-06-sprachfassungen.md`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{
    book_dir, book_locks, checked_id, core_title, cover_file, fresh_id, lock, now_ms, plain_name,
    read_book, surnames, write_book, AudioTrack, AudioVersion, LibraryBook, TextVersion, TRASH,
};

pub const SCHEMA: u32 = 2;

/// Eine Sprachfassung: Text und/oder Hörbuch in einer Sprache.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct Edition {
    /// Stabil: `ed_<buch>` (migriert) oder `ed_<ms>_<zufall>`.
    pub id: String,
    /// Sprachcode (`de`, `en`), leer = unbekannt.
    pub lang: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    /// Open-Library-Werk (`/works/OL…W`): belegt Werkgleichheit über Sprachen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioVersion>,
    /// Dauerhafte Fortschrittsreferenz des Hörbuchs; wird nie umgeschlüsselt.
    #[serde(default)]
    pub audio_key: String,
    /// `migration`, `import`, `merge` oder `split`.
    #[serde(default)]
    pub origin: String,
    /// Beim Zusammenlegen: Kennung des Quellbuchs (für „Abtrennen“).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_book: Option<String>,
    #[serde(default)]
    pub added_ms: f64,
    /// Letzte Änderung dieser Fassung (Sync-Merge je Kennung).
    #[serde(default)]
    pub updated: f64,
    #[serde(default)]
    pub device: String,
    /// Nur zur Anzeige: absolute Pfade der Hörbuchteile (leer = fehlt).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_paths: Vec<String>,
}

/// Zeitgestempelte Wahl einer Fassung. Ordnung (updated, device, edition)
/// lexikografisch: das größere Tripel gewinnt auf jedem Gerät gleich.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct Stamp {
    pub edition: String,
    pub updated: f64,
    #[serde(default)]
    pub device: String,
}

impl Stamp {
    /// Totale Ordnung der Register; die Oberfläche (PR 2) zeigt damit den Gewinner.
    #[allow(dead_code)]
    pub fn beats(&self, other: &Stamp) -> bool {
        (self.updated, &self.device, &self.edition) > (other.updated, &other.device, &other.edition)
    }
}

/// Stelle, mit der gewechselt wurde.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct ReadingPosition {
    /// Fortschrittsschlüssel der Zielfassung (`audio:…` oder `text:…`).
    pub key: String,
    /// `ms` oder `chars`.
    pub unit: String,
    pub offset: f64,
}

/// Zuletzt gewählte Sprachfassung, geräteübergreifend, samt Stelle.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct ReadingState {
    pub edition: String,
    pub updated: f64,
    #[serde(default)]
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ReadingPosition>,
}

/// Kapitelgruppe: zusammenhängende Kapitel (0-basiert) in `a` und `b`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct AlignGroup {
    pub a: Vec<u32>,
    pub b: Vec<u32>,
    pub confirmed: bool,
}

/// Kapitelzuordnung zweier Fassungen, lückenlos in Gruppen.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct Alignment {
    pub a: String,
    pub b: String,
    pub updated: f64,
    #[serde(default)]
    pub device: String,
    pub groups: Vec<AlignGroup>,
}

/// Gelöschte Sprachfassung: verhindert, dass der Sync sie zurückbringt.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default, PartialEq)]
pub struct EditionTombstone {
    pub id: String,
    pub removed_ms: f64,
}

/// Sprachcode aus freier Angabe: „Deutsch“, „de-DE“, „ger“, „German“ → `de`.
pub fn lang_code(raw: &str) -> String {
    let s = raw.trim().to_lowercase();
    let head = s.split(['-', '_']).next().unwrap_or_default();
    let code = match head {
        "de" | "deu" | "ger" | "deutsch" | "german" => "de",
        "en" | "eng" | "englisch" | "english" => "en",
        "fr" | "fra" | "fre" | "französisch" | "french" | "français" | "francais" => "fr",
        "es" | "spa" | "spanisch" | "spanish" | "español" | "espanol" => "es",
        "it" | "ita" | "italienisch" | "italian" | "italiano" => "it",
        "nl" | "nld" | "dut" | "niederländisch" | "dutch" | "nederlands" => "nl",
        "pt" | "por" | "portugiesisch" | "portuguese" | "português" => "pt",
        "pl" | "pol" | "polnisch" | "polish" | "polski" => "pl",
        "ru" | "rus" | "russisch" | "russian" => "ru",
        "sv" | "swe" | "schwedisch" | "swedish" => "sv",
        "da" | "dan" | "dänisch" | "danish" => "da",
        "tr" | "tur" | "türkisch" | "turkish" => "tr",
        other if other.len() == 2 && other.chars().all(|c| c.is_ascii_alphabetic()) => other,
        _ => "",
    };
    code.to_string()
}

fn rand_suffix() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    format!("{:04x}", (n ^ (n >> 16)) & 0xffff)
}

pub fn fresh_edition_id(taken: &BTreeSet<String>) -> String {
    loop {
        let id = format!("ed_{}_{}", now_ms() as i64, rand_suffix());
        if !taken.contains(&id) {
            return id;
        }
    }
}

/// Deterministische Kennung der migrierten Fassung: jedes Gerät, jeder Lauf
/// ergibt dieselbe.
pub fn migrated_id(book_id: &str) -> String {
    format!("ed_{book_id}")
}

/// Kennung der Hauptfassung.
/// Wirksame Hauptfassung: das Register, falls es auf eine lebende Fassung
/// zeigt, sonst die älteste (added_ms, Kennung). Das Register selbst wird
/// nie repariert, sonst hinge der gespeicherte Merge von der Reihenfolge ab.
pub fn primary_id(book: &LibraryBook) -> Option<&str> {
    book.primary
        .as_ref()
        .map(|p| p.edition.as_str())
        .filter(|p| book.editions.iter().any(|e| e.id == *p))
        .or_else(|| {
            book.editions
                .iter()
                .min_by(|a, b| a.added_ms.total_cmp(&b.added_ms).then(a.id.cmp(&b.id)))
                .map(|e| e.id.as_str())
        })
}

/// Fortschrittsschlüssel einer neuen Fassung.
pub fn new_audio_key(book_id: &str, edition_id: &str) -> String {
    format!("audio:{book_id}:{edition_id}")
}

/// Grabsteine anwenden, Hauptfassung und Auswahl gültig halten, fehlende
/// Fortschrittsschlüssel vergeben. Berührt die Arbeitskopie nicht.
fn tidy(book: &mut LibraryBook) {
    let dead: BTreeSet<String> = book
        .edition_tombstones
        .iter()
        .map(|t| t.id.clone())
        .collect();
    let keep: Vec<Edition> = book
        .editions
        .iter()
        .filter(|e| !dead.contains(&e.id))
        .cloned()
        .collect();
    book.editions = keep;
    let bid = book.id.clone();
    for e in book.editions.iter_mut() {
        if e.audio_key.is_empty() {
            e.audio_key = new_audio_key(&bid, &e.id);
        }
    }
    book.schema = SCHEMA;
}

/// Schema 1 → 2 und Arbeitskopie → Hauptfassung. Idempotent und
/// deterministisch (jedes Gerät migriert gleich).
pub fn normalize(book: &mut LibraryBook) {
    // Nur Schema 1 migrieren; ein Buch mit Grabsteinen war schon Schema 2.
    if book.editions.is_empty()
        && book.edition_tombstones.is_empty()
        && (book.text.is_some() || book.audio.is_some())
    {
        book.editions.push(Edition {
            id: migrated_id(&book.id),
            lang: book.language.as_deref().map(lang_code).unwrap_or_default(),
            title: book.title.clone(),
            subtitle: book.subtitle.clone(),
            isbn: book.isbn.clone(),
            work_key: book.work_key.clone(),
            text: book.text.clone(),
            audio: book.audio.clone(),
            audio_key: format!("audio:{}", book.id),
            origin: "migration".into(),
            origin_book: None,
            added_ms: book.added_ms,
            updated: book.added_ms,
            device: String::new(),
            audio_paths: Vec::new(),
        });
        // Deterministisch wie die Fassung: jedes Gerät setzt dasselbe Register.
        if book.primary.is_none() {
            book.primary = Some(Stamp {
                edition: migrated_id(&book.id),
                updated: 0.0,
                device: String::new(),
            });
        }
    }
    if book.editions.is_empty() {
        return;
    }
    tidy(book);
    let pid = primary_id(book).unwrap_or_default().to_string();
    let lang = book
        .language
        .as_deref()
        .map(lang_code)
        .filter(|l| !l.is_empty());
    let (text, audio) = (book.text.clone(), book.audio.clone());
    let title = Some(book.title.clone()).filter(|t| !t.trim().is_empty());
    let (subtitle, isbn, work_key) = (
        book.subtitle.clone(),
        book.isbn.clone(),
        book.work_key.clone(),
    );
    if let Some(p) = book.editions.iter_mut().find(|e| e.id == pid) {
        // Ein bestehender Ablauf hat die Arbeitskopie geändert (Inhalt oder
        // Angaben wie Titel/ISBN): Fassung nachziehen, sonst setzt der nächste
        // Empfang die Angaben aus der Fassung zurück.
        let changed = p.text != text
            || p.audio != audio
            || lang.as_ref().is_some_and(|l| *l != p.lang)
            // Leerer Fassungstitel erbt den Buchtitel (kein Wechsel).
            || title.as_ref().is_some_and(|t| !p.title.is_empty() && *t != p.title)
            || p.subtitle != subtitle
            || p.isbn != isbn
            || p.work_key != work_key;
        if changed {
            p.text = text;
            p.audio = audio;
            if let Some(l) = lang {
                p.lang = l;
            }
            if let Some(t) = title {
                p.title = t;
            }
            p.subtitle = subtitle;
            p.isbn = isbn;
            p.work_key = work_key;
            // Echt jünger als der bisherige Stand, auch in derselben Millisekunde.
            p.updated = next_stamp(Some(p.updated));
        }
    }
}

/// Zeitstempel für eine Registerschreibung: jetzt, aber echt jünger als `prev`.
fn next_stamp(prev: Option<f64>) -> f64 {
    prev.map_or_else(now_ms, |p| now_ms().max(p + 1.0))
}

/// Hauptfassung wechseln: die Arbeitskopie übernimmt Text, Hörbuch, Sprache,
/// Titel und ISBN der neuen Hauptfassung.
pub fn set_primary(book: &mut LibraryBook, edition_id: &str, device: &str) -> Result<(), String> {
    normalize(book);
    let e = book
        .editions
        .iter()
        .find(|e| e.id == edition_id)
        .cloned()
        .ok_or("Sprachfassung nicht gefunden.")?;
    // Registerschreibungen sind echt jünger als der bisherige Stand.
    let now = next_stamp(book.primary.as_ref().map(|p| p.updated));
    // Die bisherige Hauptfassung behält ihre Angaben.
    let old = primary_id(book).unwrap_or_default().to_string();
    let (title, subtitle, isbn, work_key) = (
        book.title.clone(),
        book.subtitle.clone(),
        book.isbn.clone(),
        book.work_key.clone(),
    );
    if let Some(o) = book.editions.iter_mut().find(|x| x.id == old) {
        o.title = title;
        o.subtitle = subtitle;
        o.isbn = isbn;
        o.work_key = work_key;
        o.updated = now.max(o.updated + 1.0);
        o.device = device.to_string();
    }
    book.primary = Some(Stamp {
        edition: e.id.clone(),
        updated: now,
        device: device.to_string(),
    });
    book.text = e.text;
    book.audio = e.audio;
    book.language = (!e.lang.is_empty()).then(|| e.lang.clone());
    if !e.title.trim().is_empty() {
        book.title = e.title;
    }
    book.subtitle = e.subtitle;
    book.isbn = e.isbn;
    book.work_key = e.work_key;
    normalize(book);
    Ok(())
}

/// Alle Hörbuchteile aller Fassungen (Dateinamen in `<buch>/audio/`).
pub fn all_tracks(book: &LibraryBook) -> Vec<AudioTrack> {
    let mut seen = BTreeSet::new();
    let primary = book.audio.iter();
    let others = book
        .editions
        .iter()
        .filter(|e| Some(e.id.as_str()) != primary_id(book))
        .filter_map(|e| e.audio.as_ref());
    primary
        .chain(others)
        .flat_map(super::tracks_of)
        .filter(|t| seen.insert(t.file.clone()))
        .collect()
}

pub fn validate(book: &LibraryBook) -> Result<(), String> {
    for e in &book.editions {
        if checked_id(&e.id).is_err() && !valid_edition_id(&e.id) {
            return Err("Ungültige Kennung einer Sprachfassung.".into());
        }
        if let Some(a) = &e.audio {
            if !plain_name(&a.file) && !a.file.is_empty()
                || a.tracks.iter().any(|t| !plain_name(&t.file))
            {
                return Err("Ungültiger Dateiname einer Sprachfassung.".into());
            }
        }
    }
    Ok(())
}

pub fn valid_edition_id(id: &str) -> bool {
    id.starts_with("ed_")
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Anzeigepfade der Hörbuchteile je Fassung setzen (wie `audio_paths`).
pub fn fill_paths(dir: &Path, book: &mut LibraryBook) {
    for e in book.editions.iter_mut() {
        e.audio_paths = e
            .audio
            .as_ref()
            .map(|a| {
                super::tracks_of(a)
                    .iter()
                    .map(|t| {
                        let p = dir.join("audio").join(&t.file);
                        if p.is_file() {
                            p.to_string_lossy().into_owned()
                        } else {
                            String::new()
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|v| v.iter().any(|p| !p.is_empty()))
            .unwrap_or_default();
    }
}

pub fn strip_paths(book: &mut LibraryBook) {
    for e in book.editions.iter_mut() {
        e.audio_paths = Vec::new();
    }
}

/// Teile unter neuem Namen `<editionId>__<name>` ablegen. Liefert die Fassung
/// mit den neuen Namen; `moved` sammelt (von, nach) für ein Zurückrollen.
fn adopt_audio(
    from_dir: &Path,
    to_dir: &Path,
    edition_id: &str,
    audio: &AudioVersion,
    moved: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<AudioVersion, String> {
    let mut out = audio.clone();
    std::fs::create_dir_all(to_dir.join("audio")).map_err(|e| e.to_string())?;
    let rename = |name: &str| -> String {
        if name.starts_with(&format!("{edition_id}__")) {
            name.to_string()
        } else {
            format!("{edition_id}__{name}")
        }
    };
    let mut names: Vec<String> = audio.tracks.iter().map(|t| t.file.clone()).collect();
    if !audio.file.is_empty() && !names.contains(&audio.file) {
        names.push(audio.file.clone());
    }
    for name in names {
        let from = from_dir.join("audio").join(&name);
        let to = to_dir.join("audio").join(rename(&name));
        if from.is_file() {
            if to.exists() {
                return Err(format!("Datei schon vorhanden: {}", to.display()));
            }
            std::fs::rename(&from, &to).map_err(|e| format!("Hörbuchteil verschieben: {e}"))?;
            moved.push((from, to));
        }
    }
    if !out.file.is_empty() {
        out.file = rename(&out.file);
    }
    for t in out.tracks.iter_mut() {
        t.file = rename(&t.file);
        t.transcript_job = None;
        t.transcript_jobs = Vec::new();
    }
    out.transcript_job = None;
    Ok(out)
}

/// Eigenes Cover einer Fassung, die nicht Hauptfassung ist: `covers/<ed>.*`.
pub fn edition_cover(dir: &Path, edition_id: &str) -> Option<PathBuf> {
    ["jpg", "png", "webp"]
        .iter()
        .map(|x| dir.join("covers").join(format!("{edition_id}.{x}")))
        .find(|p| p.is_file())
}

pub(crate) fn undo_moves(moved: &[(PathBuf, PathBuf)]) {
    for (from, to) in moved.iter().rev() {
        let _ = std::fs::rename(to, from);
    }
}

/// Bücher `sources` als Sprachfassungen in `target` übernehmen. Die Quellen
/// gehen samt Cover und `buch.json` in den Papierkorb; ihre Hörbuchteile
/// wandern umbenannt ins Ziel, ihre Vorlesen-Seiten und Fortschrittsschlüssel
/// bleiben unverändert.
pub fn join_in(root: &Path, target_id: &str, sources: &[String]) -> Result<LibraryBook, String> {
    if sources.iter().any(|s| s == target_id) {
        return Err("Ein Buch lässt sich nicht mit sich selbst zusammenlegen.".into());
    }
    let mut ids: Vec<&str> = vec![target_id];
    ids.extend(sources.iter().map(String::as_str));
    let locks = book_locks(&ids);
    {
        let _book_guards: Vec<_> = locks
            .iter()
            .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
            .collect();
        let _guard = lock();
        let tdir = book_dir(root, target_id)?;
        let mut target = read_book(&tdir).ok_or("Zielbuch nicht gefunden.")?;
        // Für die Rücknahme: Zielbuch roh, neu angelegte Cover.
        let original = std::fs::read(tdir.join("buch.json")).map_err(|e| e.to_string())?;
        let mut new_covers: Vec<std::path::PathBuf> = Vec::new();
        normalize(&mut target);
        let mut taken: BTreeSet<String> = target.editions.iter().map(|e| e.id.clone()).collect();
        let mut moved = Vec::new();
        let result = (|| -> Result<(), String> {
            for sid in sources {
                let sdir = book_dir(root, sid)?;
                let mut source = read_book(&sdir).ok_or("Quellbuch nicht gefunden.")?;
                normalize(&mut source);
                if source.editions.is_empty() {
                    return Err(format!("„{}“ hat weder Text noch Hörbuch.", source.title));
                }
                // Angaben der Quell-Hauptfassung sichern, bevor die Arbeitskopie wegfällt.
                let src_primary = primary_id(&source).map(str::to_string);
                // Alte → neue Kennung, damit Kapitelzuordnungen mitkommen.
                let mut renamed: BTreeMap<String, String> = BTreeMap::new();
                for mut e in source.editions.clone() {
                    let new_id = fresh_edition_id(&taken);
                    taken.insert(new_id.clone());
                    renamed.insert(e.id.clone(), new_id.clone());
                    if Some(&e.id) == src_primary.as_ref() {
                        e.title = source.title.clone();
                        e.subtitle = source.subtitle.clone();
                        e.isbn = source.isbn.clone();
                        e.work_key = source.work_key.clone();
                        if e.lang.is_empty() {
                            e.lang = source
                                .language
                                .as_deref()
                                .map(lang_code)
                                .unwrap_or_default();
                        }
                        if let Some(c) = cover_file(&sdir) {
                            let ext = c
                                .extension()
                                .map(|x| x.to_string_lossy().to_string())
                                .unwrap_or("jpg".into());
                            let covers = tdir.join("covers");
                            std::fs::create_dir_all(&covers).map_err(|e| e.to_string())?;
                            let to = covers.join(format!("{new_id}.{ext}"));
                            std::fs::copy(&c, &to).map_err(|e| format!("Cover sichern: {e}"))?;
                            new_covers.push(to);
                        }
                    }
                    // Eigenes Cover einer weiteren Fassung der Quelle mitnehmen.
                    if Some(&e.id) != src_primary.as_ref() {
                        if let Some(c) = edition_cover(&sdir, &e.id) {
                            let ext = c
                                .extension()
                                .map(|x| x.to_string_lossy().to_string())
                                .unwrap_or("jpg".into());
                            let covers = tdir.join("covers");
                            std::fs::create_dir_all(&covers).map_err(|e| e.to_string())?;
                            let to = covers.join(format!("{new_id}.{ext}"));
                            std::fs::copy(&c, &to).map_err(|e| format!("Cover sichern: {e}"))?;
                            new_covers.push(to);
                        }
                    }
                    if let Some(a) = &e.audio {
                        e.audio = Some(adopt_audio(&sdir, &tdir, &new_id, a, &mut moved)?);
                    }
                    e.id = new_id;
                    e.updated = now_ms();
                    e.origin = "merge".into();
                    e.origin_book = Some(source.id.clone());
                    if e.added_ms <= 0.0 {
                        e.added_ms = source.added_ms;
                    }
                    target.editions.push(e);
                }
                // Kapitelzuordnungen (samt Bestätigungen) unter neuen Kennungen.
                for al in &source.alignments {
                    if let (Some(a), Some(b)) = (renamed.get(&al.a), renamed.get(&al.b)) {
                        let mut al = al.clone();
                        al.a = a.clone();
                        al.b = b.clone();
                        target.alignments.push(canonical_alignment(al));
                    }
                }
                target.favorite |= source.favorite;
            }
            normalize(&mut target);
            write_book(&tdir, &target)?;
            // Unter denselben Sperren: niemand ändert die Quellen dazwischen.
            super::trash_locked(root, sources)
        })();
        if let Err(e) = result {
            // Ganz zurück: Ziel wie vorher, Dateien an ihren alten Ort.
            let _ = std::fs::write(tdir.join("buch.json"), &original);
            undo_moves(&moved);
            for c in &new_covers {
                let _ = std::fs::remove_file(c);
            }
            return Err(e);
        }
    }
    read_book(&book_dir(root, target_id)?).ok_or_else(|| "Buch unlesbar.".into())
}

/// Sprachfassung als eigenes Buch abtrennen. Ist sie die Hauptfassung, wird
/// `new_primary` Hauptfassung des verbleibenden Buchs (Pflicht).
/// Liefert (verbleibendes Buch, neues Buch).
pub fn split_in(
    root: &Path,
    id: &str,
    edition_id: &str,
    new_primary: Option<&str>,
) -> Result<(LibraryBook, LibraryBook), String> {
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let dir = book_dir(root, id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    normalize(&mut book);
    if book.editions.len() < 2 {
        return Err("Das Buch hat nur eine Sprachfassung.".into());
    }
    let mut moved = Vec::new();
    if primary_id(&book) == Some(edition_id) {
        let np = new_primary.ok_or("Bitte zuerst eine andere Hauptfassung wählen.")?;
        if np == edition_id {
            return Err("Bitte eine andere Hauptfassung wählen.".into());
        }
        set_primary(&mut book, np, "")?;
        // Cover folgt der Hauptfassung: das der abgetrennten nach covers/<ed>,
        // von dort nimmt das neue Buch es mit.
        swap_covers(&dir, edition_id, np, &mut moved)?;
    }
    let edition = book
        .editions
        .iter()
        .find(|e| e.id == edition_id)
        .cloned()
        .ok_or("Sprachfassung nicht gefunden.")
        .inspect_err(|_| undo_moves(&moved))?;

    // Ziel: das ursprüngliche Buch, falls seine Kennung frei ist.
    let new_id = edition
        .origin_book
        .as_deref()
        .filter(|o| checked_id(o).is_ok())
        .filter(|o| !root.join(o).exists())
        .map(str::to_string)
        .unwrap_or_else(|| fresh_id(root));
    let ndir = book_dir(root, &new_id).inspect_err(|_| undo_moves(&moved))?;
    // Ein Rest im Papierkorb (Cover, Metadaten) wird nicht gebraucht: die
    // Fassung trägt alles Nötige; der Papierkorb läuft normal ab.
    let result = (|| -> Result<LibraryBook, String> {
        std::fs::create_dir_all(&ndir).map_err(|e| e.to_string())?;
        let mut audio = edition.audio.clone();
        if let Some(a) = &edition.audio {
            let mut out = a.clone();
            std::fs::create_dir_all(ndir.join("audio")).map_err(|e| e.to_string())?;
            let strip = |name: &str| {
                name.strip_prefix(&format!("{}__", edition.id))
                    .unwrap_or(name)
                    .to_string()
            };
            for t in super::tracks_of(a) {
                let from = dir.join("audio").join(&t.file);
                let to = ndir.join("audio").join(strip(&t.file));
                if to.exists() {
                    return Err(format!("„{}“ gibt es im neuen Buch schon.", to.display()));
                }
                if from.is_file() {
                    std::fs::rename(&from, &to)
                        .map_err(|e| format!("Hörbuchteil verschieben: {e}"))?;
                    moved.push((from, to));
                }
            }
            out.file = strip(&out.file);
            for t in out.tracks.iter_mut() {
                t.file = strip(&t.file);
            }
            audio = Some(out);
        }
        if let Some(c) = ["jpg", "png", "webp"]
            .iter()
            .map(|x| dir.join("covers").join(format!("{}.{x}", edition.id)))
            .find(|p| p.is_file())
        {
            let ext = c
                .extension()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default();
            let to = ndir.join(format!("cover.{ext}"));
            std::fs::rename(&c, &to).map_err(|e| format!("Cover zurücklegen: {e}"))?;
            moved.push((c, to));
        }
        let fresh = LibraryBook {
            id: new_id.clone(),
            title: if edition.title.trim().is_empty() {
                book.title.clone()
            } else {
                edition.title.clone()
            },
            subtitle: edition.subtitle.clone(),
            authors: book.authors.clone(),
            language: (!edition.lang.is_empty()).then(|| edition.lang.clone()),
            isbn: edition.isbn.clone(),
            work_key: edition.work_key.clone(),
            cover_source: if cover_file(&ndir).is_some() {
                "file".into()
            } else {
                "generated".into()
            },
            text: edition.text.clone(),
            audio,
            added_ms: if edition.added_ms > 0.0 {
                edition.added_ms
            } else {
                now_ms()
            },
            ..Default::default()
        };
        let mut fresh = fresh;
        // Gleiche Fassungskennung behalten: Fortschritt `audio:<buch>:<ed>`
        // bleibt so auffindbar, bis die Oberfläche ihn umschreibt.
        fresh.editions = vec![Edition {
            updated: now_ms(),
            origin: "split".into(),
            origin_book: Some(book.id.clone()),
            audio_paths: Vec::new(),
            ..edition.clone()
        }];
        fresh.primary = Some(Stamp {
            edition: edition.id.clone(),
            updated: now_ms(),
            device: String::new(),
        });
        normalize(&mut fresh);
        write_book(&ndir, &fresh)?;
        book.editions.retain(|e| e.id != edition.id);
        book.alignments
            .retain(|a| a.a != edition.id && a.b != edition.id);
        // Grabstein: der Sync darf die Fassung nicht zurückbringen.
        book.edition_tombstones.push(EditionTombstone {
            id: edition.id.clone(),
            removed_ms: now_ms(),
        });
        normalize(&mut book);
        write_book(&dir, &book)?;
        Ok(fresh)
    })();
    match result {
        Ok(fresh) => {
            let kept = read_book(&dir).ok_or("Buch unlesbar.")?;
            let fresh = read_book(&ndir).unwrap_or(fresh);
            Ok((kept, fresh))
        }
        Err(e) => {
            // Dateien zurück, dann das neue Buch ganz weg (es war vorher
            // nicht da): sonst zeigte es auf Dateien, die nicht mehr da sind.
            undo_moves(&moved);
            let _ = std::fs::remove_dir_all(&ndir);
            Err(e)
        }
    }
}

/// Sprachfassung löschen: Grabstein im Buch, Dateien 30 Tage im Papierkorb
/// (`.papierkorb/<buch>~<fassung>/`). Haupt- oder aktive Fassung nur mit Ersatz.
pub fn remove_in(
    root: &Path,
    id: &str,
    edition_id: &str,
    new_primary: Option<&str>,
) -> Result<LibraryBook, String> {
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let dir = book_dir(root, id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    normalize(&mut book);
    if book.editions.len() < 2 {
        return Err("Die letzte Sprachfassung lässt sich nur mit dem Buch löschen.".into());
    }
    let mut moved = Vec::new();
    let is_primary = primary_id(&book) == Some(edition_id);
    let is_reading = book
        .reading
        .as_ref()
        .is_some_and(|r| r.edition == edition_id);
    if is_primary || is_reading {
        let np = new_primary
            .filter(|n| *n != edition_id)
            .ok_or("Bitte zuerst eine andere Sprachfassung als Ersatz wählen.")?;
        if is_primary {
            set_primary(&mut book, np, "")?;
            swap_covers(&dir, edition_id, np, &mut moved)?;
        }
        if is_reading {
            book.reading = Some(ReadingState {
                edition: np.to_string(),
                updated: next_stamp(book.reading.as_ref().map(|r| r.updated)),
                device: book
                    .reading
                    .as_ref()
                    .map(|r| r.device.clone())
                    .unwrap_or_default(),
                position: None,
            });
        }
    }
    let edition = book
        .editions
        .iter()
        .find(|e| e.id == edition_id)
        .cloned()
        .ok_or("Sprachfassung nicht gefunden.")
        .inspect_err(|_| undo_moves(&moved))?;
    let bin = root.join(TRASH).join(format!("{id}~{edition_id}"));
    if let Err(e) = std::fs::create_dir_all(bin.join("audio")) {
        undo_moves(&moved);
        return Err(e.to_string());
    }
    // Eigenes Cover der Fassung mit in den Papierkorb (für Wiederherstellen).
    if let Some(c) = ["jpg", "png", "webp"]
        .iter()
        .map(|x| dir.join("covers").join(format!("{edition_id}.{x}")))
        .find(|p| p.is_file())
    {
        let ext = c
            .extension()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        let to = bin.join(format!("cover.{ext}"));
        if let Err(e) = std::fs::rename(&c, &to) {
            undo_moves(&moved);
            return Err(format!("Entfernen fehlgeschlagen: {e}"));
        }
        moved.push((c, to));
    }
    if let Some(a) = &edition.audio {
        for t in super::tracks_of(a) {
            let from = dir.join("audio").join(&t.file);
            if from.is_file() {
                let to = bin.join("audio").join(&t.file);
                if let Err(e) = std::fs::rename(&from, &to) {
                    undo_moves(&moved);
                    return Err(format!("Entfernen fehlgeschlagen: {e}"));
                }
                moved.push((from, to));
            }
        }
    }
    // Ohne fassung.json ließe sich die Fassung nicht wiederherstellen:
    // Schreibfehler brechen ab und nehmen alle Verschiebungen zurück.
    let written = serde_json::to_vec_pretty(&edition)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(bin.join("fassung.json"), json).map_err(|e| e.to_string()))
        .and_then(|_| {
            std::fs::write(bin.join(super::TRASH_STAMP), format!("{}", now_ms() as i64))
                .map_err(|e| e.to_string())
        });
    if let Err(e) = written {
        undo_moves(&moved);
        let _ = std::fs::remove_dir_all(&bin);
        return Err(format!("Papierkorb schreiben: {e}"));
    }
    book.editions.retain(|e| e.id != edition_id);
    book.alignments
        .retain(|a| a.a != edition_id && a.b != edition_id);
    book.edition_tombstones.push(EditionTombstone {
        id: edition_id.to_string(),
        removed_ms: now_ms(),
    });
    normalize(&mut book);
    if let Err(e) = write_book(&dir, &book) {
        undo_moves(&moved);
        return Err(e);
    }
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Cover tauschen, wenn die Hauptfassung wechselt: `cover.*` gehört danach
/// zur neuen Hauptfassung, das bisherige wandert nach `covers/<alt>.*`.
pub fn swap_covers(
    dir: &Path,
    old_primary: &str,
    new_primary: &str,
    moved: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    if old_primary == new_primary || old_primary.is_empty() {
        return Ok(());
    }
    let start = moved.len();
    let covers = dir.join("covers");
    let incoming = ["jpg", "png", "webp"]
        .iter()
        .map(|x| covers.join(format!("{new_primary}.{x}")))
        .find(|p| p.is_file());
    std::fs::create_dir_all(&covers).map_err(|e| e.to_string())?;
    if let Some(cur) = cover_file(dir) {
        let ext = cur
            .extension()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or("jpg".into());
        let to = covers.join(format!("{old_primary}.{ext}"));
        std::fs::rename(&cur, &to).map_err(|e| format!("Cover beiseitelegen: {e}"))?;
        moved.push((cur, to));
    }
    if let Some(next) = incoming {
        let ext = next
            .extension()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or("jpg".into());
        let to = dir.join(format!("cover.{ext}"));
        if let Err(e) = std::fs::rename(&next, &to) {
            // Halber Tausch: den ersten Schritt sofort zurücknehmen.
            undo_moves(&moved[start..]);
            moved.truncate(start);
            return Err(format!("Cover übernehmen: {e}"));
        }
        moved.push((next, to));
    }
    Ok(())
}

/// Hauptfassung wechseln samt Cover, alles unter einer Sperre; scheitert
/// das Schreiben, kommen die Cover zurück.
pub fn set_primary_in(
    root: &Path,
    id: &str,
    edition_id: &str,
    device: &str,
) -> Result<LibraryBook, String> {
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let dir = book_dir(root, id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    let old = primary_id(&book).unwrap_or_default().to_string();
    set_primary(&mut book, edition_id, device)?;
    super::validate_book(&book)?;
    let mut moved = Vec::new();
    let result =
        swap_covers(&dir, &old, edition_id, &mut moved).and_then(|_| write_book(&dir, &book));
    if let Err(e) = result {
        undo_moves(&moved);
        return Err(e);
    }
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Buch unter Sperre lesen, ändern, schreiben.
pub fn update_in(
    root: &Path,
    id: &str,
    change: impl FnOnce(&mut LibraryBook) -> Result<(), String>,
) -> Result<LibraryBook, String> {
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let dir = book_dir(root, id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    change(&mut book)?;
    super::validate_book(&book)?;
    write_book(&dir, &book)?;
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

pub fn set_alignment(book: &mut LibraryBook, alignment: Alignment) -> Result<(), String> {
    if !valid_groups(&alignment.groups) {
        return Err("Kapitelzuordnung mit Lücke oder Doppelzuordnung.".into());
    }
    let alive = |id: &str| book.editions.iter().any(|e| e.id == id);
    if !alive(&alignment.a) || !alive(&alignment.b) || alignment.a == alignment.b {
        return Err("Sprachfassung nicht gefunden.".into());
    }
    let mut a = canonical_alignment(alignment);
    let same = |x: &Alignment| {
        let x = canonical_alignment(x.clone());
        (x.a, x.b) == (a.a.clone(), a.b.clone())
    };
    // Registerschreibung: echt jünger als der bisherige Stand dieses Paars.
    let prev = book
        .alignments
        .iter()
        .filter(|x| same(x))
        .map(|x| x.updated)
        .fold(None, |m: Option<f64>, u| Some(m.map_or(u, |m| m.max(u))));
    if let Some(p) = prev {
        a.updated = a.updated.max(p + 1.0);
    }
    book.alignments.retain(|x| !same(x));
    book.alignments.push(a);
    Ok(())
}

/// Auswahl setzen (Sprachwechsel). Der Zeitstempel der Oberfläche bleibt,
/// wird aber echt jünger als der bisherige Registerstand gemacht.
pub fn set_reading(book: &mut LibraryBook, mut reading: ReadingState) -> Result<(), String> {
    if !book.editions.iter().any(|e| e.id == reading.edition) {
        return Err("Sprachfassung nicht gefunden.".into());
    }
    if let Some(prev) = &book.reading {
        reading.updated = reading.updated.max(prev.updated + 1.0);
    }
    book.reading = Some(reading);
    Ok(())
}

/// Gelöschte Fassung aus `.papierkorb/<buch>~<fassung>/` zurückholen.
pub fn restore_in(
    root: &Path,
    id: &str,
    edition_id: &str,
    device: &str,
) -> Result<LibraryBook, String> {
    if !valid_edition_id(edition_id) {
        return Err("Ungültige Kennung einer Sprachfassung.".into());
    }
    let locks = book_locks(&[id]);
    let _book_guards: Vec<_> = locks
        .iter()
        .map(|l| l.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let _guard = lock();
    let dir = book_dir(root, id)?;
    let mut book = read_book(&dir).ok_or("Buch nicht gefunden.")?;
    let bin = root.join(TRASH).join(format!("{id}~{edition_id}"));
    let raw = std::fs::read(bin.join("fassung.json")).map_err(|_| "Nicht mehr im Papierkorb.")?;
    let old: Edition = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let back = restored(&old, now_ms(), device);
    if !valid_edition_id(&back.id) {
        return Err("Kennung zu lang.".into());
    }
    // Erst prüfen, dann verschieben: eine fremde Datei gleichen Namens im
    // Buch hält das Wiederherstellen an, der Papierkorb bleibt unberührt.
    let tracks = old.audio.as_ref().map(super::tracks_of).unwrap_or_default();
    let same = |a: &Path, b: &Path| std::fs::read(a).ok() == std::fs::read(b).ok();
    for t in &tracks {
        let (from, to) = (
            bin.join("audio").join(&t.file),
            dir.join("audio").join(&t.file),
        );
        if from.is_file() && to.exists() && !same(&from, &to) {
            return Err(format!("„{}“ gibt es im Buch schon.", t.file));
        }
    }
    let mut moved = Vec::new();
    let result = (|| -> Result<(), String> {
        for t in &tracks {
            let (from, to) = (
                bin.join("audio").join(&t.file),
                dir.join("audio").join(&t.file),
            );
            if from.is_file() && !to.exists() {
                std::fs::create_dir_all(dir.join("audio")).map_err(|e| e.to_string())?;
                std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                moved.push((from, to));
            }
        }
        if let Some(c) = cover_file(&bin) {
            let ext = c
                .extension()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default();
            std::fs::create_dir_all(dir.join("covers")).map_err(|e| e.to_string())?;
            let to = dir.join("covers").join(format!("{}.{ext}", back.id));
            std::fs::rename(&c, &to).map_err(|e| e.to_string())?;
            moved.push((c, to));
        }
        if !book.editions.iter().any(|e| e.id == back.id) {
            book.editions.push(back.clone());
        }
        super::validate_book(&book)?;
        write_book(&dir, &book)
    })();
    if let Err(e) = result {
        undo_moves(&moved);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&bin);
    read_book(&dir).ok_or_else(|| "Buch unlesbar.".into())
}

/// Vermutlich dieselbe Arbeit in zwei Sprachen.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct LanguagePair {
    pub a: String,
    pub b: String,
    /// Sicher: gleicher Werkschlüssel oder gleicher Kerntitel und Autor.
    pub sure: bool,
    /// `work_key`, `title_author` oder `author_only`.
    pub reason: String,
}

fn langs_of(b: &LibraryBook) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = b
        .editions
        .iter()
        .map(|e| e.lang.clone())
        .filter(|l| !l.is_empty())
        .collect();
    if let Some(l) = b
        .language
        .as_deref()
        .map(lang_code)
        .filter(|l| !l.is_empty())
    {
        out.insert(l);
    }
    out
}

fn work_keys(b: &LibraryBook) -> BTreeSet<String> {
    b.editions
        .iter()
        .filter_map(|e| e.work_key.clone())
        .chain(b.work_key.clone())
        .collect()
}

/// Paare von Büchern, die Sprachfassungen eines Werks sein könnten. ISBN zählt
/// nicht (Übersetzungen haben eigene). Nie automatisch zusammenlegen.
pub fn language_pairs(books: &[LibraryBook]) -> Vec<LanguagePair> {
    let mut out = Vec::new();
    for (i, a) in books.iter().enumerate() {
        for b in &books[i + 1..] {
            let (la, lb) = (langs_of(a), langs_of(b));
            if la.is_empty() || lb.is_empty() || !la.is_disjoint(&lb) {
                continue;
            }
            let (sa, sb) = (surnames(&a.authors), surnames(&b.authors));
            let same_author = !sa.is_empty() && sa.iter().any(|s| sb.contains(s));
            let (wa, wb) = (work_keys(a), work_keys(b));
            if !wa.is_empty() && !wa.is_disjoint(&wb) {
                out.push(pair(a, b, true, "work_key"));
                continue;
            }
            if !same_author {
                continue;
            }
            let ta = core_title(&a.title);
            if !ta.is_empty() && ta == core_title(&b.title) {
                out.push(pair(a, b, true, "title_author"));
                continue;
            }
            // Unsicher: je Sprache genau ein Buch dieses Autors.
            let unique = |lang: &BTreeSet<String>, me: &LibraryBook| {
                books
                    .iter()
                    .filter(|x| x.id != me.id)
                    .filter(|x| surnames(&x.authors).iter().any(|s| sa.contains(s)))
                    .all(|x| langs_of(x).is_disjoint(lang))
            };
            if unique(&la, a) && unique(&lb, b) {
                out.push(pair(a, b, false, "author_only"));
            }
        }
    }
    out
}

fn pair(a: &LibraryBook, b: &LibraryBook, sure: bool, reason: &str) -> LanguagePair {
    LanguagePair {
        a: a.id.clone(),
        b: b.id.clone(),
        sure,
        reason: reason.into(),
    }
}

// ---------------------------------------------------------------------------
// Sync-Merge (Spezifikation §5): zustandsbasiert, kommutativ, assoziativ,
// idempotent. Jeder Teil ist ein Register mit totaler Ordnung oder eine
// Grow-only-Menge; die Property-Tests unten belegen die Konvergenz.
// ---------------------------------------------------------------------------

/// Der Teil eines Buchs, den der Merge zusammenführt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditionState {
    pub editions: Vec<Edition>,
    pub tombstones: Vec<EditionTombstone>,
    pub primary: Option<Stamp>,
    pub reading: Option<ReadingState>,
    pub alignments: Vec<Alignment>,
}

/// FNV-1a über die kanonische JSON-Form: entscheidet Gleichstände bei gleicher
/// Version und verschiedenem Inhalt, auf allen Geräten gleich.
fn content_hash<T: Serialize>(v: &T) -> u64 {
    let raw = serde_json::to_vec(v).unwrap_or_default();
    raw.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

fn edition_key(e: &Edition) -> (u64, String, u64) {
    let mut bare = e.clone();
    bare.audio_paths = Vec::new();
    (f(e.updated), e.device.clone(), content_hash(&bare))
}

fn f(x: f64) -> u64 {
    // Ordnung positiver Zeitstempel bleibt in den Bits erhalten.
    x.max(0.0).to_bits()
}

fn reading_key(r: &ReadingState) -> (u64, String, String, u64) {
    (
        f(r.updated),
        r.device.clone(),
        r.edition.clone(),
        content_hash(&r.position),
    )
}

fn alignment_key(a: &Alignment) -> (u64, String, u64) {
    (f(a.updated), a.device.clone(), content_hash(&a.groups))
}

/// Paar ohne Richtung (`a` < `b`), Gruppen entsprechend gespiegelt.
fn canonical_alignment(mut a: Alignment) -> Alignment {
    if a.a > a.b {
        std::mem::swap(&mut a.a, &mut a.b);
        for g in a.groups.iter_mut() {
            std::mem::swap(&mut g.a, &mut g.b);
        }
    }
    a
}

/// Gruppen lückenlos, zusammenhängend, aufsteigend, jedes Kapitel genau einmal.
pub fn valid_groups(groups: &[AlignGroup]) -> bool {
    let (mut na, mut nb) = (0u32, 0u32);
    for g in groups {
        if g.a.is_empty() || g.b.is_empty() {
            return false;
        }
        for (i, c) in g.a.iter().enumerate() {
            if *c != na + i as u32 {
                return false;
            }
        }
        for (i, c) in g.b.iter().enumerate() {
            if *c != nb + i as u32 {
                return false;
            }
        }
        na += g.a.len() as u32;
        nb += g.b.len() as u32;
    }
    true
}

fn max_by_key<T: Clone, K: Ord>(a: Option<&T>, b: Option<&T>, key: impl Fn(&T) -> K) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if key(y) > key(x) {
            y.clone()
        } else {
            x.clone()
        }),
        (x, y) => x.or(y).cloned(),
    }
}

/// Zwei Stände zusammenführen. Reine Funktion.
pub fn merge_state(x: &EditionState, y: &EditionState) -> EditionState {
    // Grabsteine: Vereinigung, je Kennung der größte Zeitpunkt.
    let mut tomb: BTreeMap<String, f64> = BTreeMap::new();
    for t in x.tombstones.iter().chain(&y.tombstones) {
        let e = tomb.entry(t.id.clone()).or_insert(t.removed_ms);
        if t.removed_ms > *e {
            *e = t.removed_ms;
        }
    }
    // Fassungen: je Kennung das größere (updated, device, Inhalt).
    let mut eds: BTreeMap<String, Edition> = BTreeMap::new();
    for e in x.editions.iter().chain(&y.editions) {
        match eds.get(&e.id) {
            Some(cur) if edition_key(cur) >= edition_key(e) => {}
            _ => {
                eds.insert(e.id.clone(), e.clone());
            }
        }
    }
    eds.retain(|id, _| !tomb.contains_key(id));
    let editions: Vec<Edition> = eds.into_values().collect();
    // Hauptfassung: Register (updated, device, edition).
    let primary = max_by_key(x.primary.as_ref(), y.primary.as_ref(), |p| {
        (f(p.updated), p.device.clone(), p.edition.clone())
    });
    // Auswahl: Register; Stelle muss zur Fassung gehören.
    let reading = max_by_key(x.reading.as_ref(), y.reading.as_ref(), reading_key);
    // Zuordnungen: je Paar ein Register über die ganze Gruppenliste (kein
    // gruppenweiser Merge: Teilen/Verbinden bleibt so immer lückenlos).
    let mut al: BTreeMap<(String, String), Alignment> = BTreeMap::new();
    for a in x.alignments.iter().chain(&y.alignments) {
        let a = canonical_alignment(a.clone());
        if !valid_groups(&a.groups) {
            continue;
        }
        let k = (a.a.clone(), a.b.clone());
        match al.get(&k) {
            Some(cur) if alignment_key(cur) >= alignment_key(&a) => {}
            _ => {
                al.insert(k, a);
            }
        }
    }
    let alignments: Vec<Alignment> = al.into_values().collect();
    EditionState {
        tombstones: tomb
            .into_iter()
            .map(|(id, removed_ms)| EditionTombstone { id, removed_ms })
            .collect(),
        editions,
        primary,
        reading,
        alignments,
    }
}

/// Wirksame Auswahl (§5.2). Die Register bleiben im Stand roh erhalten, damit
/// der Merge assoziativ ist; gültig ist eine Auswahl nur, wenn ihre Fassung
/// lebt (sonst gilt die Hauptfassung), und ihre Stelle nur, wenn der Schlüssel
/// zu dieser Fassung gehört.
pub fn effective_reading(s: &EditionState) -> Option<ReadingState> {
    let mut r = s.reading.clone()?;
    if !s.editions.iter().any(|e| e.id == r.edition) {
        return None;
    }
    {
        if let Some(pos) = &r.position {
            let e = s.editions.iter().find(|e| e.id == r.edition);
            let owns = e.is_some_and(|e| {
                pos.key == e.audio_key
                    || e.text
                        .as_ref()
                        .is_some_and(|t| pos.key.starts_with(&format!("text:{}:", t.page_id)))
            });
            if !owns {
                r.position = None;
            }
        }
    }
    Some(r)
}

/// Wirksame Zuordnungen: nur zwischen lebenden Fassungen.
#[allow(dead_code)] // Oberfläche (PR 2): Dialog „Kapitel zuordnen“.
pub fn effective_alignments(s: &EditionState) -> Vec<Alignment> {
    let alive = |id: &str| s.editions.iter().any(|e| e.id == id);
    s.alignments
        .iter()
        .filter(|a| alive(&a.a) && alive(&a.b))
        .cloned()
        .collect()
}

/// Wirksame Hauptfassung: das Register, falls es auf eine lebende Fassung
/// zeigt, sonst die älteste (added_ms, Kennung).
pub fn effective_primary(s: &EditionState) -> Option<String> {
    s.primary
        .as_ref()
        .map(|p| p.edition.clone())
        .filter(|p| s.editions.iter().any(|e| &e.id == p))
        .or_else(|| {
            s.editions
                .iter()
                .min_by(|a, b| a.added_ms.total_cmp(&b.added_ms).then(a.id.cmp(&b.id)))
                .map(|e| e.id.clone())
        })
}

pub fn state_of(b: &LibraryBook) -> EditionState {
    EditionState {
        editions: b
            .editions
            .iter()
            .cloned()
            .map(|mut e| {
                e.audio_paths = Vec::new();
                e
            })
            .collect(),
        tombstones: b.edition_tombstones.clone(),
        primary: b.primary.clone(),
        reading: b.reading.clone(),
        alignments: b.alignments.clone(),
    }
}

/// Stand ins Buch übernehmen; Anzeige (Titel, Untertitel, ISBN, Sprache) und
/// Schema-1-Spiegel (`text`, `audio`, `language`) kommen aus der wirksamen
/// Hauptfassung.
pub fn apply_state(b: &mut LibraryBook, s: EditionState) {
    let primary = effective_primary(&s);
    b.editions = s.editions;
    b.edition_tombstones = s.tombstones;
    b.primary = s.primary;
    b.reading = s.reading;
    b.alignments = s.alignments;
    if let Some(e) = primary
        .and_then(|p| b.editions.iter().find(|e| e.id == p))
        .cloned()
    {
        b.text = e.text;
        b.audio = e.audio;
        b.language = (!e.lang.is_empty()).then(|| e.lang.clone());
        if !e.title.trim().is_empty() {
            b.title = e.title;
        }
        b.subtitle = e.subtitle;
        b.isbn = e.isbn;
        b.work_key = e.work_key;
    } else if !b.edition_tombstones.is_empty() {
        // Alle Fassungen sind anderswo gelöscht oder abgetrennt (gleichzeitig
        // auf zwei Geräten): leer bleiben statt aus der Arbeitskopie eine
        // gelöschte Fassung neu zu migrieren. Abgetrenntes lebt als eigenes
        // Buch, Gelöschtes im Papierkorb.
        b.text = None;
        b.audio = None;
    }
    b.schema = SCHEMA;
}

/// Empfang (Sync): `incoming` hat seine Metadaten (Autoren, Favorit,
/// Beschreibung) schon nach Buch-`updated` gewählt; hier kommt der
/// Fassungsteil nach [`merge_state`] dazu. Liefert die Stelle, auf die der
/// Fortschritt gehoben werden muss, wenn die fremde Auswahl gewonnen hat.
pub fn merge_incoming(
    local: Option<&LibraryBook>,
    incoming: &mut LibraryBook,
) -> Option<(ReadingPosition, f64)> {
    let mut theirs = incoming.clone();
    if let Some(l) = local {
        // Schema 2 ohne lebende Fassung (alle gelöscht) ist kein Altgerät.
        if theirs.schema < SCHEMA && theirs.editions.is_empty() && !l.editions.is_empty() {
            // Altgerät (Schema 1): kennt nur die Arbeitskopie der Hauptfassung.
            // Sie gilt als Änderung dieser Fassung, alles andere bleibt lokal.
            let mut legacy = l.clone();
            legacy.text = theirs.text.clone();
            legacy.audio = theirs.audio.clone();
            if theirs.language.is_some() {
                legacy.language = theirs.language.clone();
            }
            normalize(&mut legacy);
            theirs = legacy;
        }
    }
    normalize(&mut theirs);
    let theirs_state = state_of(&theirs);
    let merged = match local {
        Some(l) => {
            let mut mine = l.clone();
            normalize(&mut mine);
            merge_state(&state_of(&mine), &theirs_state)
        }
        None => merge_state(&theirs_state, &theirs_state),
    };
    let mine_reading = local.and_then(|l| {
        let mut m = l.clone();
        normalize(&mut m);
        effective_reading(&state_of(&m))
    });
    let lift = match (&effective_reading(&merged), &mine_reading) {
        (Some(r), Some(m)) if r == m => None,
        (Some(r), _) => r.position.clone().map(|p| (p, r.updated)),
        _ => None,
    };
    apply_state(incoming, merged);
    lift
}

/// Zu welcher Fassung gehören die flachen Dateiverweise (`files.text/cover`)
/// eines empfangenen Buchs? Dieselbe Regel wie in [`merge_incoming`]:
/// - Schema 2: die Hauptfassung des Senders;
/// - Schema 1 (Altgerät) und lokal mit Fassungen: die hiesige Hauptfassung,
///   denn dort landet seine Arbeitskopie;
/// - Schema 1 auf beiden Seiten: `ed_<buch>` (deterministische Migration).
pub fn sender_primary(local: Option<&LibraryBook>, incoming: &LibraryBook) -> Option<String> {
    let legacy = incoming.schema < SCHEMA && incoming.editions.is_empty();
    if legacy {
        if let Some(l) = local.filter(|l| !l.editions.is_empty()) {
            let mut l = l.clone();
            normalize(&mut l);
            return primary_id(&l).map(str::to_string);
        }
    }
    let mut b = incoming.clone();
    normalize(&mut b);
    primary_id(&b).map(str::to_string)
}

/// Gelöschte Fassung wiederherstellen (§7): neue, deterministische Kennung
/// `<alt>_r`, gleiche Referenzen (Fortschritt, Seite, Dateien). Zwei Geräte,
/// die dieselbe Fassung wiederherstellen, erzeugen dieselbe Kennung.
pub fn restored(e: &Edition, now: f64, device: &str) -> Edition {
    let mut r = e.clone();
    r.id = format!("{}_r", e.id);
    r.updated = now;
    r.device = device.to_string();
    r.origin = "restore".into();
    r.audio_paths = Vec::new();
    r
}

/// Fortschritt beim Empfang heben (Spezifikation §5.2): Datensatz `key` auf
/// `offset`, wenn er älter als die gewonnene Auswahl ist. Andere Felder
/// (gehörte Bereiche, `finished`) bleiben.
pub fn lift_progress(
    map: &mut std::collections::BTreeMap<String, String>,
    pos: &ReadingPosition,
    updated: f64,
) {
    let key = format!("progress.{}", pos.key);
    let mut rec: serde_json::Value = map
        .get(&key)
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| serde_json::json!({ "offset": 0, "length": 0, "heard": [], "read": [], "updated": 0 }));
    let older = rec.get("updated").and_then(|v| v.as_f64()).unwrap_or(0.0) < updated;
    if !older {
        return;
    }
    rec["offset"] = serde_json::json!(pos.offset);
    rec["updated"] = serde_json::json!(updated);
    if pos.unit == "ms" {
        rec["unit"] = serde_json::json!("ms");
    }
    if let Ok(raw) = serde_json::to_string(&rec) {
        map.insert(key, raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lv-editions-{}-{}",
            std::process::id(),
            now_ms() as i64 + rand_suffix().len() as i64
        ));
        let d = d.join(rand_suffix());
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, book: &LibraryBook, files: &[&str]) {
        let dir = root.join(&book.id);
        for f in files {
            std::fs::create_dir_all(dir.join("audio")).unwrap();
            std::fs::write(dir.join("audio").join(f), f.as_bytes()).unwrap();
        }
        write_book(&dir, book).unwrap();
    }

    fn audio(file: &str) -> AudioVersion {
        AudioVersion {
            file: file.into(),
            ..Default::default()
        }
    }

    fn base(id: &str, title: &str, lang: &str) -> LibraryBook {
        LibraryBook {
            id: id.into(),
            title: title.into(),
            authors: vec!["Anna Beispiel".into()],
            language: Some(lang.into()),
            cover_source: "generated".into(),
            added_ms: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn sprachcodes() {
        assert_eq!(lang_code("Deutsch"), "de");
        assert_eq!(lang_code("ger"), "de");
        assert_eq!(lang_code("en-GB"), "en");
        assert_eq!(lang_code("Français"), "fr");
        assert_eq!(lang_code("??"), "");
    }

    #[test]
    fn migration_ist_deterministisch_und_wiederholbar() {
        let mut b = base("book_7", "Effi", "Deutsch");
        b.text = Some(TextVersion {
            page_id: "page_1".into(),
            origin: "document".into(),
            source_name: None,
        });
        let mut c = b.clone();
        normalize(&mut b);
        normalize(&mut b);
        normalize(&mut c);
        assert_eq!(b, c);
        assert_eq!(b.schema, SCHEMA);
        assert_eq!(b.editions.len(), 1);
        assert_eq!(b.editions[0].id, "ed_book_7");
        assert_eq!(b.editions[0].lang, "de");
        assert_eq!(primary_id(&b), Some("ed_book_7"));
        assert_eq!(b.editions[0].audio_key, "audio:book_7");
        assert_eq!(b.editions[0].text.as_ref().unwrap().page_id, "page_1");
    }

    #[test]
    fn arbeitskopie_ist_quelle_der_hauptfassung() {
        let mut b = base("book_7", "Effi", "de");
        b.audio = Some(audio("a.mp3"));
        normalize(&mut b);
        // Bestehender Ablauf ändert nur die Arbeitskopie (z. B. Transkript).
        b.text = Some(TextVersion {
            page_id: "p".into(),
            origin: "transcript".into(),
            source_name: None,
        });
        normalize(&mut b);
        assert_eq!(b.editions[0].text.as_ref().unwrap().page_id, "p");
    }

    #[test]
    fn zusammenlegen_benennt_um_und_trennt_wieder() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("teil_001.mp3"));
        de.work_key = Some("/works/OL1W".into());
        put(&root, &de, &["teil_001.mp3"]);
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("teil_001.mp3"));
        en.isbn = Some("978-1".into());
        put(&root, &en, &["teil_001.mp3"]);
        std::fs::write(root.join("book_2").join("cover.jpg"), b"jpg").unwrap();

        let joined = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        assert_eq!(joined.editions.len(), 2);
        let e = joined.editions.iter().find(|e| e.lang == "en").unwrap();
        assert_eq!(e.title, "The Way");
        assert_eq!(e.isbn.as_deref(), Some("978-1"));
        assert_eq!(e.origin_book.as_deref(), Some("book_2"));
        assert_eq!(e.audio_key, "audio:book_2", "Fortschrittsschlüssel bleibt");
        let f = &e.audio.as_ref().unwrap().file;
        assert!(f.starts_with(&e.id) && f.ends_with("__teil_001.mp3"));
        assert!(root.join("book_1/audio").join(f).is_file());
        assert!(
            root.join("book_1/audio/teil_001.mp3").is_file(),
            "eigene Teile bleiben"
        );
        assert!(!root.join("book_2").exists(), "Quelle im Papierkorb");
        assert!(root.join(TRASH).join("book_2").is_dir());
        assert_eq!(joined.title, "Der Weg");
        assert_eq!(e.audio_paths.len(), 1);

        // Abtrennen: Kennung des Quellbuchs ist wieder frei.
        let eid = e.id.clone();
        let (kept, back) = split_in(&root, "book_1", &eid, None).unwrap();
        assert_eq!(kept.editions.len(), 1);
        assert!(
            kept.edition_tombstones.iter().any(|t| t.id == eid),
            "Grabstein im alten Buch"
        );
        assert_eq!(back.id, "book_2");
        assert_eq!(back.title, "The Way");
        assert_eq!(back.audio.as_ref().unwrap().file, "teil_001.mp3");
        assert_eq!(back.editions[0].audio_key, "audio:book_2");
        assert!(root.join("book_2/audio/teil_001.mp3").is_file());
        assert!(root.join("book_2/cover.jpg").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn cover_of(dir: &Path) -> Vec<u8> {
        std::fs::read(cover_file(dir).unwrap()).unwrap()
    }

    /// Codex R4: Cover folgt der Hauptfassung beim Wechsel, Abtrennen,
    /// Löschen und Wiederherstellen.
    #[test]
    fn cover_folgt_hauptfassung() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        std::fs::write(root.join("book_1/cover.jpg"), b"DE").unwrap();
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        std::fs::write(root.join("book_2/cover.jpg"), b"EN").unwrap();
        let joined = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        let dir = root.join("book_1");
        let de_id = primary_id(&joined).unwrap().to_string();
        let en_id = joined
            .editions
            .iter()
            .find(|e| e.id != de_id)
            .unwrap()
            .id
            .clone();
        assert_eq!(cover_of(&dir), b"DE");

        // Wechsel: EN vorne, DE beiseite.
        set_primary_in(&root, "book_1", &en_id, "pc").unwrap();
        assert_eq!(cover_of(&dir), b"EN");
        assert_eq!(
            std::fs::read(dir.join(format!("covers/{de_id}.jpg"))).unwrap(),
            b"DE"
        );

        // Löschen der Hauptfassung EN mit Ersatz DE: DE-Cover zurück vorne,
        // EN-Cover im Papierkorb, beim Wiederherstellen unter neuer Kennung.
        let after = remove_in(&root, "book_1", &en_id, Some(&de_id)).unwrap();
        assert_eq!(primary_id(&after), Some(de_id.as_str()));
        assert_eq!(cover_of(&dir), b"DE");
        let back = restore_in(&root, "book_1", &en_id, "pc").unwrap();
        let r_id = format!("{en_id}_r");
        assert!(back.editions.iter().any(|e| e.id == r_id));
        assert_eq!(
            std::fs::read(dir.join(format!("covers/{r_id}.jpg"))).unwrap(),
            b"EN"
        );

        // Abtrennen der Hauptfassung DE (Ersatz EN_r): neues Buch nimmt DE mit.
        let (kept, fresh) = split_in(&root, "book_1", &de_id, Some(&r_id)).unwrap();
        assert_eq!(primary_id(&kept), Some(r_id.as_str()));
        assert_eq!(cover_of(&dir), b"EN");
        assert_eq!(cover_of(&root.join(&fresh.id)), b"DE");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex R5/2: Zusammenlegen nimmt auch die Cover weiterer Fassungen mit.
    #[test]
    fn zusammenlegen_nimmt_alle_cover_mit() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        let mut fr = base("book_3", "Le Chemin", "fr");
        fr.audio = Some(audio("c.mp3"));
        put(&root, &fr, &["c.mp3"]);
        std::fs::write(root.join("book_3/cover.jpg"), b"FR").unwrap();
        std::fs::write(root.join("book_2/cover.png"), b"EN").unwrap();
        // book_2 hat schon zwei Fassungen: EN vorne, FR mit eigenem Cover.
        let two = join_in(&root, "book_2", &["book_3".into()]).unwrap();
        let fr_id = two
            .editions
            .iter()
            .find(|e| e.lang == "fr")
            .unwrap()
            .id
            .clone();
        assert_eq!(
            std::fs::read(root.join(format!("book_2/covers/{fr_id}.jpg"))).unwrap(),
            b"FR"
        );
        let all = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        assert_eq!(all.editions.len(), 3);
        let dir = root.join("book_1");
        for (lang, bytes, ext) in [("en", &b"EN"[..], "png"), ("fr", &b"FR"[..], "jpg")] {
            let id = &all.editions.iter().find(|e| e.lang == lang).unwrap().id;
            assert_eq!(
                std::fs::read(dir.join(format!("covers/{id}.{ext}"))).unwrap(),
                bytes,
                "{lang}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex R9: Zusammenlegen nimmt die Kapitelzuordnungen der Quelle mit,
    /// samt manueller Bestätigung, unter den neuen Kennungen.
    #[test]
    fn zusammenlegen_nimmt_kapitelzuordnung_mit() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        let mut fr = base("book_3", "Le Chemin", "fr");
        fr.audio = Some(audio("c.mp3"));
        put(&root, &fr, &["c.mp3"]);
        let mut two = join_in(&root, "book_2", &["book_3".into()]).unwrap();
        let id =
            |b: &LibraryBook, l: &str| b.editions.iter().find(|e| e.lang == l).unwrap().id.clone();
        let (en_id, fr_id) = (id(&two, "en"), id(&two, "fr"));
        let groups = vec![
            AlignGroup {
                a: vec![0],
                b: vec![0, 1],
                confirmed: true,
            },
            AlignGroup {
                a: vec![1],
                b: vec![2],
                confirmed: false,
            },
        ];
        set_alignment(
            &mut two,
            Alignment {
                a: en_id,
                b: fr_id,
                updated: 5.0,
                device: "pc".into(),
                groups: groups.clone(),
            },
        )
        .unwrap();
        write_book(&root.join("book_2"), &two).unwrap();
        let all = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        let (en_new, fr_new) = (id(&all, "en"), id(&all, "fr"));
        assert_eq!(all.alignments.len(), 1);
        let got = canonical_alignment(all.alignments[0].clone());
        let want = canonical_alignment(Alignment {
            a: en_new,
            b: fr_new,
            updated: 5.0,
            device: "pc".into(),
            groups,
        });
        assert_eq!((got.a, got.b, got.groups), (want.a, want.b, want.groups));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Codex R5/3: scheitert der zweite Schritt des Cover-Tauschs, ist der
    /// erste zurückgenommen.
    #[test]
    fn cover_tausch_halb_wird_zurueckgenommen() {
        let dir = tmp();
        std::fs::write(dir.join("cover.jpg"), b"ALT").unwrap();
        std::fs::create_dir_all(dir.join("covers")).unwrap();
        std::fs::write(dir.join("covers/ed_neu.png"), b"NEU").unwrap();
        std::fs::create_dir_all(dir.join("cover.png/blockiert")).unwrap();
        let mut moved = vec![(PathBuf::from("/x"), PathBuf::from("/y"))];
        assert!(swap_covers(&dir, "ed_alt", "ed_neu", &mut moved).is_err());
        assert_eq!(moved.len(), 1, "fremde Einträge bleiben");
        assert_eq!(std::fs::read(dir.join("cover.jpg")).unwrap(), b"ALT");
        assert!(dir.join("covers/ed_neu.png").is_file());
        assert!(!dir.join("covers/ed_alt.jpg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex R5/3+4: Wechsel der Hauptfassung beim Löschen; scheitert das
    /// Schreiben von fassung.json, sind Cover, Hörbuch und Buch unverändert.
    #[test]
    fn papierkorb_fehler_nimmt_alles_zurueck() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        std::fs::write(root.join("book_1/cover.jpg"), b"DE").unwrap();
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        std::fs::write(root.join("book_2/cover.jpg"), b"EN").unwrap();
        let joined = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        let p = primary_id(&joined).unwrap().to_string();
        let other = joined
            .editions
            .iter()
            .find(|e| e.id != p)
            .unwrap()
            .id
            .clone();
        let before = std::fs::read(root.join("book_1/buch.json")).unwrap();
        std::fs::create_dir_all(root.join(TRASH).join(format!("book_1~{p}/fassung.json/x")))
            .unwrap();
        assert!(remove_in(&root, "book_1", &p, Some(&other)).is_err());
        let dir = root.join("book_1");
        assert_eq!(std::fs::read(dir.join("buch.json")).unwrap(), before);
        assert_eq!(cover_of(&dir), b"DE");
        assert_eq!(
            std::fs::read(dir.join(format!("covers/{other}.jpg"))).unwrap(),
            b"EN"
        );
        assert!(dir.join("audio/a.mp3").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Fremde Datei gleichen Namens: Wiederherstellen hält an, Papierkorb bleibt.
    #[test]
    fn wiederherstellen_ueberschreibt_nichts() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        let joined = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        let p = primary_id(&joined).unwrap().to_string();
        let other = joined.editions.iter().find(|e| e.id != p).unwrap().clone();
        remove_in(&root, "book_1", &other.id, None).unwrap();
        let f = other.audio.unwrap().file;
        std::fs::write(root.join("book_1/audio").join(&f), b"fremd").unwrap();
        assert!(restore_in(&root, "book_1", &other.id, "pc").is_err());
        assert!(root
            .join(TRASH)
            .join(format!("book_1~{}", other.id))
            .join("audio")
            .join(&f)
            .is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hauptfassung_nur_mit_ersatz() {
        let root = tmp();
        let mut de = base("book_1", "Der Weg", "de");
        de.audio = Some(audio("a.mp3"));
        put(&root, &de, &["a.mp3"]);
        let mut en = base("book_2", "The Way", "en");
        en.audio = Some(audio("b.mp3"));
        put(&root, &en, &["b.mp3"]);
        let joined = join_in(&root, "book_1", &["book_2".into()]).unwrap();
        let p = primary_id(&joined).unwrap().to_string();
        let other = joined
            .editions
            .iter()
            .find(|e| e.id != p)
            .unwrap()
            .id
            .clone();
        assert!(remove_in(&root, "book_1", &p, None).is_err());
        assert!(split_in(&root, "book_1", &p, None).is_err());
        let after = remove_in(&root, "book_1", &p, Some(&other)).unwrap();
        assert_eq!(primary_id(&after), Some(other.as_str()));
        assert_eq!(after.title, "The Way");
        assert_eq!(after.language.as_deref(), Some("en"));
        assert_eq!(after.edition_tombstones.len(), 1);
        assert!(!root.join("book_1/audio/a.mp3").exists());
        assert!(root
            .join(TRASH)
            .join(format!("book_1~{p}/audio/a.mp3"))
            .is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hauptfassung_wechseln_behaelt_angaben() {
        let mut b = base("book_1", "Der Weg", "de");
        b.isbn = Some("978-de".into());
        b.audio = Some(audio("a.mp3"));
        normalize(&mut b);
        b.editions.push(Edition {
            id: "ed_x".into(),
            lang: "en".into(),
            title: "The Way".into(),
            audio: Some(audio("ed_x__b.mp3")),
            ..Default::default()
        });
        set_primary(&mut b, "ed_x", "pc").unwrap();
        assert_eq!(b.title, "The Way");
        assert_eq!(b.audio.as_ref().unwrap().file, "ed_x__b.mp3");
        set_primary(&mut b, "ed_book_1", "pc").unwrap();
        assert_eq!(b.title, "Der Weg");
        assert_eq!(b.isbn.as_deref(), Some("978-de"));
        assert_eq!(b.audio.as_ref().unwrap().file, "a.mp3");
    }

    #[test]
    fn paare_erkennen_ohne_isbn() {
        let mut a = base("book_1", "Garten im Wandel", "de");
        a.work_key = Some("/works/OL9W".into());
        let mut b = base("book_2", "The Changing Garden", "en");
        b.work_key = Some("/works/OL9W".into());
        b.authors = vec!["Ben Muster".into()];
        let c = base("book_3", "Das Archiv", "de");
        let d = base("book_4", "The Archive", "en");
        let mut e = base("book_5", "Effi Briest", "de");
        e.isbn = Some("1".into());
        let mut f = base("book_6", "Effi Briest", "en");
        f.isbn = Some("2".into());
        let pairs = language_pairs(&[a.clone(), b.clone()]);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].sure);
        // Gleicher Autor, Titel verschieden, je Sprache ein Buch → unsicher.
        let pairs = language_pairs(&[c.clone(), d.clone()]);
        assert_eq!(pairs.len(), 1);
        assert!(!pairs[0].sure);
        // Gleicher Kerntitel + Autor → sicher, obwohl ISBN verschieden.
        let pairs = language_pairs(&[e, f]);
        assert!(pairs[0].sure && pairs[0].reason == "title_author");
        // Gleiche Sprache → nie ein Paar.
        let mut g = d.clone();
        g.language = Some("de".into());
        assert!(language_pairs(&[c.clone(), g]).is_empty());
        // Zwei deutsche Bücher desselben Autors → kein unsicheres Paar.
        let mut h = c.clone();
        h.id = "book_9".into();
        h.title = "Anderes".into();
        assert!(language_pairs(&[c, d, h]).is_empty());
    }

    #[test]
    fn empfang_altgeraet_verliert_keine_fassungen() {
        let mut local = base("book_1", "Der Weg", "de");
        local.audio = Some(audio("a.mp3"));
        normalize(&mut local);
        local.editions.push(Edition {
            id: "ed_x".into(),
            lang: "en".into(),
            title: "The Way".into(),
            ..Default::default()
        });
        local.reading = Some(ReadingState {
            edition: "ed_x".into(),
            updated: 10.0,
            device: "pc".into(),
            position: None,
        });
        // Altes iPhone: nur Arbeitskopie, Favorit geändert.
        let mut incoming = base("book_1", "Der Weg", "de");
        incoming.audio = Some(audio("a.mp3"));
        incoming.favorite = true;
        merge_incoming(Some(&local), &mut incoming);
        assert_eq!(incoming.editions.len(), 2);
        assert!(incoming.favorite);
        assert_eq!(incoming.reading.as_ref().unwrap().edition, "ed_x");
    }

    #[test]
    fn empfang_neuere_auswahl_gewinnt_grabstein_bleibt() {
        let mut local = base("book_1", "Der Weg", "de");
        local.audio = Some(audio("a.mp3"));
        normalize(&mut local);
        let mut remote = local.clone();
        local.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 10.0,
            device: "pc".into(),
            position: None,
        });
        remote.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 20.0,
            device: "ios".into(),
            position: None,
        });
        local.edition_tombstones.push(EditionTombstone {
            id: "ed_old".into(),
            removed_ms: 5.0,
        });
        remote.editions.push(Edition {
            id: "ed_old".into(),
            lang: "en".into(),
            ..Default::default()
        });
        merge_incoming(Some(&local), &mut remote);
        assert_eq!(remote.reading.as_ref().unwrap().device, "ios");
        assert!(remote.editions.iter().all(|e| e.id != "ed_old"));
        // Gleicher Zeitstempel: totale Ordnung (updated, device, Inhaltshash),
        // das größere Tupel gewinnt – auf allen Geräten gleich, in beiden Richtungen.
        let mut l2 = local.clone();
        l2.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 30.0,
            device: "b".into(),
            position: None,
        });
        let mut r2 = local.clone();
        r2.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 30.0,
            device: "a".into(),
            position: None,
        });
        let mut l3 = l2.clone();
        merge_incoming(Some(&l2), &mut r2);
        assert_eq!(r2.reading.unwrap().device, "b");
        let mut r3 = local.clone();
        r3.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 30.0,
            device: "a".into(),
            position: None,
        });
        merge_incoming(Some(&r3), &mut l3);
        assert_eq!(l3.reading.unwrap().device, "b");
    }

    #[test]
    fn empfang_je_fassung_und_fortschritt_heben() {
        let mut local = base("book_1", "Der Weg", "de");
        local.audio = Some(audio("a.mp3"));
        normalize(&mut local);
        let mut remote = local.clone();
        // Lokal neue Fassung, fern geänderte Hauptfassung: beides bleibt.
        local.editions.push(Edition {
            id: "ed_l".into(),
            lang: "en".into(),
            updated: 5.0,
            ..Default::default()
        });
        remote.editions[0].title = "fern".into();
        remote.title = "fern".into(); // Schema-2-Sender spiegelt die Hauptfassung
        remote.editions[0].updated = 99.0e12;
        remote.reading = Some(ReadingState {
            edition: "ed_book_1".into(),
            updated: 50.0,
            device: "ios".into(),
            position: Some(ReadingPosition {
                key: "audio:book_1".into(),
                unit: "ms".into(),
                offset: 7000.0,
            }),
        });
        let lift = merge_incoming(Some(&local), &mut remote);
        assert_eq!(remote.editions.len(), 2);
        assert_eq!(
            remote
                .editions
                .iter()
                .find(|e| e.id == "ed_book_1")
                .unwrap()
                .title,
            "fern"
        );
        let (pos, at) = lift.expect("fremde Auswahl gewinnt");
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "progress.audio:book_1".to_string(),
            r#"{"offset":10,"length":0,"heard":[[0,10]],"read":[],"updated":40}"#.to_string(),
        );
        lift_progress(&mut map, &pos, at);
        let v: serde_json::Value = serde_json::from_str(&map["progress.audio:book_1"]).unwrap();
        assert_eq!(v["offset"], 7000.0);
        assert_eq!(v["heard"][0][1], 10, "gehörte Bereiche bleiben");
        // Neuerer lokaler Datensatz wird nicht überschrieben.
        lift_progress(&mut map, &pos, 10.0);
        let v: serde_json::Value = serde_json::from_str(&map["progress.audio:book_1"]).unwrap();
        assert_eq!(v["offset"], 7000.0);
    }

    #[test]
    fn altgeraet_aendert_nur_arbeitskopie() {
        let mut local = base("book_1", "Der Weg", "de");
        local.audio = Some(audio("a.mp3"));
        normalize(&mut local);
        local.editions.push(Edition {
            id: "ed_x".into(),
            lang: "en".into(),
            ..Default::default()
        });
        set_primary(&mut local, "ed_x", "pc").unwrap();
        set_primary(&mut local, "ed_book_1", "pc").unwrap();
        let mut old = base("book_1", "Der Weg", "de");
        old.audio = Some(audio("a.mp3"));
        old.text = Some(TextVersion {
            page_id: "neu".into(),
            origin: "transcript".into(),
            source_name: None,
        });
        assert!(merge_incoming(Some(&local), &mut old).is_none());
        assert_eq!(old.editions.len(), 2);
        assert_eq!(
            old.editions
                .iter()
                .find(|e| e.id == "ed_book_1")
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .page_id,
            "neu"
        );
    }

    #[test]
    fn tripel_ordnung() {
        let a = Stamp {
            edition: "ed_a".into(),
            updated: 5.0,
            device: "x".into(),
        };
        let b = Stamp {
            edition: "ed_b".into(),
            updated: 5.0,
            device: "x".into(),
        };
        assert!(b.beats(&a) && !a.beats(&b));
    }

    // ---- Property-Tests: Konvergenz des Merges --------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_state(r: &mut Rng) -> EditionState {
        let ids = ["ed_a", "ed_b", "ed_c", "ed_d"];
        let devs = ["", "pc", "ios"];
        let mut s = EditionState::default();
        for id in ids {
            if r.pick(3) > 0 {
                s.editions.push(Edition {
                    id: id.into(),
                    lang: ["de", "en"][r.pick(2) as usize].into(),
                    title: format!("T{}", r.pick(3)),
                    audio_key: format!("audio:b:{id}"),
                    added_ms: r.pick(3) as f64,
                    updated: r.pick(4) as f64,
                    device: devs[r.pick(3) as usize].into(),
                    ..Default::default()
                });
            }
            if r.pick(6) == 0 {
                s.tombstones.push(EditionTombstone {
                    id: id.into(),
                    removed_ms: r.pick(4) as f64,
                });
            }
        }
        if r.pick(2) == 0 {
            s.primary = Some(Stamp {
                edition: ids[r.pick(4) as usize].into(),
                updated: r.pick(4) as f64,
                device: devs[r.pick(3) as usize].into(),
            });
        }
        if r.pick(2) == 0 {
            let e = ids[r.pick(4) as usize];
            s.reading = Some(ReadingState {
                edition: e.into(),
                updated: r.pick(4) as f64,
                device: devs[r.pick(3) as usize].into(),
                position: (r.pick(2) == 0).then(|| ReadingPosition {
                    key: format!("audio:b:{}", ids[r.pick(4) as usize]),
                    unit: "ms".into(),
                    offset: r.pick(100) as f64,
                }),
            });
        }
        if r.pick(2) == 0 {
            let groups = match r.pick(3) {
                0 => vec![AlignGroup {
                    a: vec![0],
                    b: vec![0],
                    confirmed: true,
                }],
                1 => vec![AlignGroup {
                    a: vec![0],
                    b: vec![0, 1],
                    confirmed: false,
                }],
                _ => vec![AlignGroup {
                    a: vec![0, 1],
                    b: vec![0],
                    confirmed: true,
                }],
            };
            let (a, b) = if r.pick(2) == 0 {
                ("ed_a", "ed_b")
            } else {
                ("ed_b", "ed_a")
            };
            s.alignments.push(Alignment {
                a: a.into(),
                b: b.into(),
                updated: r.pick(4) as f64,
                device: devs[r.pick(3) as usize].into(),
                groups,
            });
        }
        s
    }

    /// Buch wie auf der Platte: schreiben, lesen, normalisieren.
    fn stored(b: &LibraryBook) -> LibraryBook {
        let mut back: LibraryBook =
            serde_json::from_str(&serde_json::to_string(b).unwrap()).unwrap();
        normalize(&mut back);
        back
    }

    fn book_of(s: &EditionState) -> LibraryBook {
        let mut b = LibraryBook {
            id: "b".into(),
            title: "Buch".into(),
            schema: SCHEMA,
            ..Default::default()
        };
        apply_state(&mut b, s.clone());
        stored(&b)
    }

    /// Empfang auf `local`: wie `apply_book` (merge_incoming, schreiben, lesen).
    fn receive(local: &LibraryBook, incoming: &LibraryBook) -> LibraryBook {
        let mut inc = incoming.clone();
        merge_incoming(Some(local), &mut inc);
        stored(&inc)
    }

    /// Codex R4: Konvergenz auch über Übernehmen, Schreiben und Lesen –
    /// das gespeicherte Register darf nicht von der Reihenfolge abhängen.
    #[test]
    fn gespeicherter_merge_konvergiert() {
        let mut r = Rng(0xD1B54A32D192ED03);
        for _ in 0..3000 {
            let (x, y, z) = (
                book_of(&random_state(&mut r)),
                book_of(&random_state(&mut r)),
                book_of(&random_state(&mut r)),
            );
            let a = receive(&receive(&x, &y), &z);
            let b = receive(&x, &receive(&y, &z));
            let c = receive(&receive(&z, &y), &x);
            assert_eq!(state_of(&a), state_of(&b), "Reihenfolge");
            assert_eq!(state_of(&a), state_of(&c), "Richtung");
            assert_eq!(state_of(&receive(&a, &a)), state_of(&a), "idempotent");
            assert_eq!(
                a.primary,
                merge_state(&merge_state(&state_of(&x), &state_of(&y)), &state_of(&z)).primary,
                "Register roh"
            );
        }
    }

    #[test]
    fn merge_kommutativ_assoziativ_idempotent() {
        let mut r = Rng(0x9E3779B97F4A7C15);
        for _ in 0..5000 {
            let (x, y, z) = (
                random_state(&mut r),
                random_state(&mut r),
                random_state(&mut r),
            );
            let xy = merge_state(&x, &y);
            assert_eq!(xy, merge_state(&y, &x), "kommutativ");
            assert_eq!(
                merge_state(&xy, &z),
                merge_state(&x, &merge_state(&y, &z)),
                "assoziativ"
            );
            assert_eq!(merge_state(&xy, &xy), xy, "idempotent");
            assert_eq!(merge_state(&x, &xy), xy, "Absorption");
            // Wirksame Sicht: Auswahl nur auf lebende Fassung, Stelle gehört dazu.
            if let Some(rd) = effective_reading(&xy) {
                let e = xy.editions.iter().find(|e| e.id == rd.edition).unwrap();
                if let Some(p) = rd.position {
                    assert_eq!(p.key, e.audio_key);
                }
            }
            for a in effective_alignments(&xy) {
                assert!(valid_groups(&a.groups));
                assert!(a.a < a.b);
            }
            for t in &xy.tombstones {
                assert!(
                    xy.editions.iter().all(|e| e.id != t.id),
                    "kein Wiederauftauchen"
                );
            }
        }
    }

    #[test]
    fn wiederherstellen_deterministisch() {
        let e = Edition {
            id: "ed_a".into(),
            audio_key: "audio:b".into(),
            ..Default::default()
        };
        let (r1, r2) = (restored(&e, 5.0, "pc"), restored(&e, 7.0, "ios"));
        assert_eq!(r1.id, r2.id);
        assert_eq!(r1.audio_key, "audio:b");
        // Gleichzeitig gelöscht (ed_a) und wiederhergestellt (ed_a_r): ed_a_r lebt.
        let x = EditionState {
            editions: vec![r1],
            tombstones: vec![EditionTombstone {
                id: "ed_a".into(),
                removed_ms: 1.0,
            }],
            ..Default::default()
        };
        let y = EditionState {
            editions: vec![e.clone(), r2],
            ..Default::default()
        };
        let m = merge_state(&x, &y);
        assert_eq!(m.editions.len(), 1);
        assert_eq!(m.editions[0].id, "ed_a_r");
    }

    #[test]
    fn gruppen_gueltig() {
        assert!(valid_groups(&[
            AlignGroup {
                a: vec![0],
                b: vec![0],
                confirmed: true
            },
            AlignGroup {
                a: vec![1],
                b: vec![1, 2],
                confirmed: true
            },
            AlignGroup {
                a: vec![2, 3],
                b: vec![3],
                confirmed: false
            },
        ]));
        assert!(
            !valid_groups(&[AlignGroup {
                a: vec![0],
                b: vec![1],
                confirmed: true
            }]),
            "Lücke"
        );
        assert!(
            !valid_groups(&[
                AlignGroup {
                    a: vec![0],
                    b: vec![0],
                    confirmed: true
                },
                AlignGroup {
                    a: vec![0],
                    b: vec![1],
                    confirmed: true
                },
            ]),
            "doppelt"
        );
    }
}
