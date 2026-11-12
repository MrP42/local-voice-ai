//! G2 (#497): Hinweis an Teilnehmende einer Besprechung, mit Nachweis.
//!
//! Der Nutzer legt vor oder waehrend der Aufnahme einen vorbereiteten Text in
//! die Zwischenablage oder fuegt ihn auf ausdruecklichen Knopfdruck in das
//! fokussierte Chatfeld der Besprechungs-App (Teams, Webex) ein. Die App
//! sendet nie selbst. Dieses Modul ist die reine Logik: Textbausteine,
//! Platzhalter, Nachweis-Datensatz, Pflicht-Regel und die Zeile im
//! Protokollexport. Es kennt weder Tauri noch den Besprechungsspeicher und
//! laesst sich deshalb ohne Laufzeit pruefen. Der Anschluss an den Speicher
//! steht in `managers::meetings::notice`, die Befehle in
//! `commands::meeting_notice`.
//!
//! Der Nachweis liegt in `meetings.metadata_json["participant_notice"]` (eine
//! Liste, aelteste zuerst) -- dieselbe Ablage wie die uebrigen Angaben zur
//! Besprechung, keine zweite Einwilligungslogik. Die Einwilligung der
//! Beteiligten (`consent_confirmed_at`) bleibt, wie sie ist.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::managers::meeting_detect::catalog::{self, AppClass};

/// Schluessel in `meetings.metadata_json`.
pub const NOTICE_KEY: &str = "participant_notice";
/// So viele Nachweise merkt sich eine Besprechung hoechstens (die aeltesten
/// fallen weg).
pub const MAX_RECORDS: usize = 20;
/// Ein Nachweis vor dem Start gilt so lange (Sekunden) fuer die Aufnahme, die
/// aus demselben Startdialog beginnt: fuenf Minuten.
pub const PENDING_TTL_SECS: i64 = 5 * 60;
/// Fehlercode, wenn die Pflicht gilt und noch kein Hinweis vorliegt.
pub const CODE_NOTICE_REQUIRED: &str = "notice_required";
/// Fehlercode, wenn ein Nachweis vor dem Start zu einem Startdialog gehoeren
/// soll, den es nicht (mehr) gibt (geschlossen, ersetzt oder schon gestartet).
pub const CODE_NOTICE_DIALOG_GONE: &str = "notice_dialog_gone";
/// Obergrenzen fuer Eingaben aus den Einstellungen (Zeichen).
pub const MAX_TEMPLATE_CHARS: usize = 2_000;
pub const MAX_PURPOSE_CHARS: usize = 200;
pub const MAX_RETENTION_CHARS: usize = 300;
/// Laenge der Frist, in der der Nutzer das Chatfeld anklickt (Sekunden).
pub const PASTE_COUNTDOWN_DEFAULT_SECS: u32 = 5;
pub const PASTE_COUNTDOWN_MAX_SECS: u32 = 15;

/// Fuer welche Besprechungs-App der Text formuliert ist.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoticeApp {
    Teams,
    Webex,
    #[default]
    Neutral,
}

impl NoticeApp {
    /// Name der Textvariante in der Sprache der Exportzeile.
    fn label(self, lang: NoticeLang) -> &'static str {
        match (self, lang) {
            (NoticeApp::Teams, _) => "Teams",
            (NoticeApp::Webex, _) => "Webex",
            (NoticeApp::Neutral, NoticeLang::De) => "neutraler Text",
            (NoticeApp::Neutral, NoticeLang::En) => "neutral text",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLang {
    #[default]
    De,
    En,
}

impl NoticeLang {
    /// Name der Sprache des Hinweistextes, in der Sprache `in_lang`.
    fn label(self, in_lang: NoticeLang) -> &'static str {
        match (self, in_lang) {
            (NoticeLang::De, NoticeLang::De) => "Deutsch",
            (NoticeLang::En, NoticeLang::De) => "Englisch",
            (NoticeLang::De, NoticeLang::En) => "German",
            (NoticeLang::En, NoticeLang::En) => "English",
        }
    }

    /// Sprache der Exportzeile zu einer Besprechungssprache (`en`, `en-US` ...):
    /// Englisch nur bei einer englischen Besprechung, sonst Deutsch (die
    /// Ueberschriften des Exports sind deutsch).
    pub fn for_language(language: Option<&str>) -> Self {
        let code = language.unwrap_or("").trim().to_ascii_lowercase();
        if code == "en" || code.starts_with("en-") || code.starts_with("en_") {
            NoticeLang::En
        } else {
            NoticeLang::De
        }
    }
}

/// Auf welchem Weg der Hinweis die Teilnehmenden erreichen soll (oder
/// erreicht hat).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum NoticeWay {
    /// Text in die Zwischenablage gelegt; Einfuegen und Senden macht der Nutzer.
    Clipboard,
    /// Text auf Knopfdruck in das fokussierte Chatfeld eingefuegt (nie gesendet).
    Paste,
    /// Der Nutzer bestaetigt, die Teilnehmenden muendlich informiert zu haben.
    Oral,
}

impl NoticeWay {
    /// Wie stark der Nachweis ist: Einfuegen vor muendlicher Bestaetigung vor
    /// blossem Kopieren (bestimmt, welcher Eintrag in den Export kommt).
    fn strength(self) -> u8 {
        match self {
            NoticeWay::Paste => 3,
            NoticeWay::Oral => 2,
            NoticeWay::Clipboard => 1,
        }
    }
}

/// Ein vom Nutzer angepasster Textbaustein (ersetzt den mitgelieferten).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct NoticeTemplate {
    pub app: NoticeApp,
    pub lang: NoticeLang,
    pub text: String,
}

/// Einstellungen des Hinweises (`AppSettings::meeting_notice`).
///
/// Standard des persoenlichen Profils: an, nicht Pflicht. Die Pflicht gehoert
/// zum Regelwerk (Einstellungen > Regeln) und wirkt nur bei eingeschalteter
/// Funktion.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(default)]
pub struct MeetingNoticeSettings {
    pub enabled: bool,
    /// Die Aufnahme startet erst nach einem bereitgestellten Hinweis oder der
    /// Bestaetigung "muendlich informiert".
    pub required: bool,
    /// Eine Zeile im Protokollexport.
    pub export_line: bool,
    pub default_app: NoticeApp,
    pub default_lang: NoticeLang,
    /// Wert fuer `{purpose}`; leer = Standard der Sprache.
    pub purpose: String,
    /// Wert fuer `{retention}`; leer = der Satz entfaellt.
    pub retention: String,
    /// Angepasste Textbausteine (je App und Sprache hoechstens einer).
    pub templates: Vec<NoticeTemplate>,
}

impl Default for MeetingNoticeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            required: false,
            export_line: true,
            default_app: NoticeApp::default(),
            default_lang: NoticeLang::default(),
            purpose: String::new(),
            retention: String::new(),
            templates: Vec::new(),
        }
    }
}

impl MeetingNoticeSettings {
    /// Eingaben begrenzen und bereinigen, bevor sie gespeichert werden: Texte
    /// getrimmt und gekuerzt, leere Bausteine fallen weg, je App und Sprache
    /// gilt der letzte.
    pub fn sanitized(mut self) -> Self {
        self.purpose = cut(self.purpose.trim(), MAX_PURPOSE_CHARS);
        self.retention = cut(self.retention.trim(), MAX_RETENTION_CHARS);
        let mut kept: Vec<NoticeTemplate> = Vec::new();
        for template in self.templates.drain(..) {
            let text = cut(template.text.trim(), MAX_TEMPLATE_CHARS);
            if text.is_empty() {
                continue;
            }
            kept.retain(|t| !(t.app == template.app && t.lang == template.lang));
            kept.push(NoticeTemplate { text, ..template });
        }
        self.templates = kept;
        self
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

// ---------------------------------------------------------------------------
// Textbausteine
// ---------------------------------------------------------------------------

/// Mitgelieferter Textbaustein. Platzhalter: `{purpose}` (`{zweck}`),
/// `{retention}` (`{aufbewahrung}`), `{processing}` (`{verarbeitung}`).
pub fn builtin_template(app: NoticeApp, lang: NoticeLang) -> &'static str {
    match (app, lang) {
        (NoticeApp::Teams, NoticeLang::De) => {
            "Hinweis: Diese Besprechung wird mit Local Voice lokal auf meinem Rechner mitgeschrieben, {purpose}. {processing} {retention} Das ist unabhängig von einer Aufzeichnung durch Teams. Wenn Sie damit nicht einverstanden sind, schreiben Sie es bitte hier in den Chat, dann beende ich die Mitschrift."
        }
        (NoticeApp::Teams, NoticeLang::En) => {
            "Note: This meeting is being transcribed with Local Voice locally on my computer, {purpose}. {processing} {retention} This is independent of any recording by Teams. If you do not agree, please say so here in the chat and I will stop the transcription."
        }
        (NoticeApp::Webex, NoticeLang::De) => {
            "Hinweis: Diese Besprechung wird mit Local Voice lokal auf meinem Rechner mitgeschrieben, {purpose}. {processing} {retention} Das ist unabhängig von einer Aufzeichnung durch Webex. Wenn Sie damit nicht einverstanden sind, schreiben Sie es bitte hier in den Chat, dann beende ich die Mitschrift."
        }
        (NoticeApp::Webex, NoticeLang::En) => {
            "Note: This meeting is being transcribed with Local Voice locally on my computer, {purpose}. {processing} {retention} This is independent of any recording by Webex. If you do not agree, please say so here in the chat and I will stop the transcription."
        }
        (NoticeApp::Neutral, NoticeLang::De) => {
            "Hinweis: Diese Besprechung wird mit Local Voice lokal auf meinem Rechner mitgeschrieben, {purpose}. {processing} {retention} Wenn Sie damit nicht einverstanden sind, sagen Sie es bitte jetzt, dann beende ich die Mitschrift."
        }
        (NoticeApp::Neutral, NoticeLang::En) => {
            "Note: This meeting is being transcribed with Local Voice locally on my computer, {purpose}. {processing} {retention} If you do not agree, please say so now and I will stop the transcription."
        }
    }
}

fn default_purpose(lang: NoticeLang) -> &'static str {
    match lang {
        NoticeLang::De => "um Notizen und ein Protokoll zu erstellen",
        NoticeLang::En => "to create notes and minutes",
    }
}

/// Namen der Anbieter, die nicht auf diesem Rechner laufen, aus den fuer die
/// Besprechung genutzten Anbietern (`(Name, lokal?)`): in der Reihenfolge der
/// Eingabe, ohne Doppelte und ohne leere Namen.
pub fn external_names<'a>(used: impl IntoIterator<Item = (&'a str, bool)>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (label, local) in used {
        let label = label.trim();
        if local || label.is_empty() || names.iter().any(|n| n == label) {
            continue;
        }
        names.push(label.to_string());
    }
    names
}

/// Satz zur Verarbeitung. "Keine Daten an Dritte" steht nur da, wo ALLE fuer
/// die Besprechung genutzten Anbieter lokal sind (`external` leer); sonst nennt
/// der Satz die Anbieter, an die der Text der Notizen geht, ohne eine
/// Zusicherung.
pub fn processing_sentence(lang: NoticeLang, external: &[String]) -> String {
    let names: Vec<&str> = external
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .collect();
    let list = match names.split_last() {
        None => {
            return match lang {
                NoticeLang::De => "Es werden keine Daten an Dritte übertragen.".to_string(),
                NoticeLang::En => "No data is sent to third parties.".to_string(),
            }
        }
        Some((last, [])) => (*last).to_string(),
        Some((last, head)) => {
            let and = match lang {
                NoticeLang::De => "und",
                NoticeLang::En => "and",
            };
            format!("{} {and} {last}", head.join(", "))
        }
    };
    match lang {
        NoticeLang::De => format!("Für die Notizen wird der Text an {list} übermittelt."),
        NoticeLang::En => format!("For the notes, the text is sent to {list}."),
    }
}

/// Werte der Platzhalter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeValues {
    pub purpose: String,
    pub retention: String,
    pub processing: String,
}

impl NoticeValues {
    pub fn from_settings(
        settings: &MeetingNoticeSettings,
        lang: NoticeLang,
        external: &[String],
    ) -> Self {
        let purpose = settings.purpose.trim();
        Self {
            purpose: if purpose.is_empty() {
                default_purpose(lang).to_string()
            } else {
                purpose.to_string()
            },
            retention: settings.retention.trim().to_string(),
            processing: processing_sentence(lang, external),
        }
    }

    fn get(&self, name: &str) -> Option<&str> {
        match name.trim().to_lowercase().as_str() {
            "purpose" | "zweck" => Some(&self.purpose),
            "retention" | "aufbewahrung" => Some(&self.retention),
            "processing" | "verarbeitung" => Some(&self.processing),
            _ => None,
        }
    }
}

/// Ein Stueck des Bausteins: Text oder Platzhalter (`{name}` und `{{name}}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Text(&'a str),
    Slot { raw: &'a str, name: &'a str },
}

/// Laenge (Bytes) und Name des Platzhalters, der bei `text` beginnt (`text`
/// beginnt mit `{`). `{{name}}` und `{name}` sind beide gueltig; ein
/// geoeffnetes, nicht geschlossenes oder verschachteltes `{` ist keiner.
fn slot_at(text: &str) -> Option<(usize, &str)> {
    if let Some(body) = text.strip_prefix("{{") {
        let close = body.find("}}")?;
        let name = &body[..close];
        if !name.contains(['{', '}']) {
            return Some((2 + close + 2, name.trim()));
        }
        return None;
    }
    let body = text.strip_prefix('{')?;
    let close = body.find('}')?;
    if body[..close].contains('{') {
        return None;
    }
    Some((1 + close + 1, body[..close].trim()))
}

fn scan(template: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut from = 0;
    let mut text_from = 0;
    while let Some(offset) = template[from..].find('{') {
        let start = from + offset;
        match slot_at(&template[start..]) {
            Some((len, name)) => {
                if start > text_from {
                    tokens.push(Token::Text(&template[text_from..start]));
                }
                tokens.push(Token::Slot {
                    raw: &template[start..start + len],
                    name,
                });
                from = start + len;
                text_from = from;
            }
            None => from = start + 1,
        }
    }
    if text_from < template.len() {
        tokens.push(Token::Text(&template[text_from..]));
    }
    tokens
}

/// Platzhalter eines Bausteins, die es nicht gibt (so geschrieben, wie sie im
/// Text stehen, ohne Doppelte). Beim Einsetzen bleiben sie unveraendert
/// stehen; die Einstellungen weisen beim Speichern darauf hin.
pub fn unknown_placeholders(template: &str) -> Vec<String> {
    let probe = NoticeValues {
        purpose: String::new(),
        retention: String::new(),
        processing: String::new(),
    };
    let mut unknown: Vec<String> = Vec::new();
    for token in scan(template) {
        if let Token::Slot { raw, name } = token {
            if probe.get(name).is_none() && !unknown.iter().any(|u| u == raw) {
                unknown.push(raw.to_string());
            }
        }
    }
    unknown
}

/// Setzt die Platzhalter ein (`{name}` und `{{name}}`; ein Durchgang: ein Wert,
/// der selbst `{...}` enthaelt, wird nicht noch einmal ersetzt; unbekannte
/// Platzhalter bleiben stehen) und raeumt auf, was Werte hinterlassen: leere
/// Werte lassen keine Luecke, und endet ein Wert schon mit einem Satzzeichen
/// (`.`, `!`, `?`), folgt dem Platzhalter kein zweiter Punkt aus dem Baustein.
pub fn render(template: &str, values: &NoticeValues) -> String {
    let mut out = String::with_capacity(template.len() + 128);
    let mut value_ended_sentence = false;
    for token in scan(template) {
        match token {
            Token::Text(text) => {
                let skip_dot = value_ended_sentence
                    && text.starts_with('.')
                    && !text.starts_with("..");
                out.push_str(if skip_dot { &text[1..] } else { text });
                value_ended_sentence = false;
            }
            Token::Slot { raw, name } => match values.get(name) {
                Some(value) => {
                    out.push_str(value);
                    value_ended_sentence = value.trim_end().ends_with(['.', '!', '?']);
                }
                None => {
                    out.push_str(raw);
                    value_ended_sentence = false;
                }
            },
        }
    }
    tidy(&out)
}

/// Mehrfache Leerzeichen und Leerzeilen zusammenziehen, Raender abschneiden.
fn tidy(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").split('\n') {
        let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Der Baustein, der gilt: die Anpassung des Nutzers, sonst der mitgelieferte.
pub fn template_for(settings: &MeetingNoticeSettings, app: NoticeApp, lang: NoticeLang) -> String {
    settings
        .templates
        .iter()
        .rev()
        .find(|t| t.app == app && t.lang == lang && !t.text.trim().is_empty())
        .map(|t| t.text.clone())
        .unwrap_or_else(|| builtin_template(app, lang).to_string())
}

/// Der fertige Text fuer `app` und `lang`. `external`: die genutzten Anbieter,
/// die nicht lokal laufen (siehe [`external_names`]).
pub fn text_for(
    settings: &MeetingNoticeSettings,
    app: NoticeApp,
    lang: NoticeLang,
    external: &[String],
) -> String {
    render(
        &template_for(settings, app, lang),
        &NoticeValues::from_settings(settings, lang, external),
    )
}

// ---------------------------------------------------------------------------
// Nachweis
// ---------------------------------------------------------------------------

/// Ein Nachweis: wann, welcher Text, auf welchem Weg.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(default)]
pub struct NoticeRecord {
    /// Unix-Sekunden.
    pub at: i64,
    pub way: NoticeWay,
    pub app: NoticeApp,
    pub lang: NoticeLang,
    /// Der verwendete Text (bei `oral` leer).
    pub text: String,
    /// Kommt die Zeile in den Protokollexport? Entschieden beim Anlegen
    /// (Einstellung `export_line`).
    pub in_export: bool,
    /// Nur bei `paste`: die Besprechungs-App bzw. der Browser, in dem das
    /// Einfuegen ausgeloest wurde (Name aus dem Katalog, nie ein
    /// Fenstertitel: Titel koennen Personendaten enthalten).
    pub target: Option<String>,
}

impl Default for NoticeRecord {
    fn default() -> Self {
        Self {
            at: 0,
            way: NoticeWay::Clipboard,
            app: NoticeApp::default(),
            lang: NoticeLang::default(),
            text: String::new(),
            in_export: true,
            target: None,
        }
    }
}

/// Die Nachweise einer Besprechung aus `metadata_json` (aelteste zuerst).
/// Unlesbare Eintraege fallen weg, statt alles zu verwerfen.
pub fn records_in(meta: Option<&Value>) -> Vec<NoticeRecord> {
    meta.and_then(|m| m.get(NOTICE_KEY))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| serde_json::from_value::<NoticeRecord>(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Der neue Wert fuer `metadata_json[NOTICE_KEY]`: die bisherigen Nachweise
/// plus `added`, hoechstens [`MAX_RECORDS`].
pub fn with_records(meta: Option<&Value>, added: &[NoticeRecord]) -> Value {
    let mut records = records_in(meta);
    records.extend(added.iter().cloned());
    if records.len() > MAX_RECORDS {
        let drop = records.len() - MAX_RECORDS;
        records.drain(..drop);
    }
    serde_json::to_value(records).unwrap_or(Value::Null)
}

/// Wie [`with_records`] fuer einen einzelnen Nachweis.
pub fn with_record(meta: Option<&Value>, rec: &NoticeRecord) -> Value {
    with_records(meta, std::slice::from_ref(rec))
}

/// Der Eintrag, der in den Export kommt: der staerkste Weg, bei Gleichstand
/// der fruehste.
pub fn export_record(records: &[NoticeRecord]) -> Option<&NoticeRecord> {
    records
        .iter()
        .filter(|r| r.in_export)
        .min_by_key(|r| (std::cmp::Reverse(r.way.strength()), r.at))
}

/// Die eine Zeile im Protokollexport in der Sprache `lang` des Exports; `when`
/// formatiert Unix-Sekunden als Ortszeit MIT Zeitzone ("2026-11-11 14:32
/// (UTC+01:00)"). Enthaelt nie den Hinweistext selbst. Beim Einfuegen steht
/// nur, dass es in der genannten App ausgeloest wurde: ob der Text im Chatfeld
/// ankam, kann die App nicht sehen.
pub fn export_line(
    records: &[NoticeRecord],
    lang: NoticeLang,
    when: impl Fn(i64) -> String,
) -> Option<String> {
    let r = export_record(records)?;
    let at = when(r.at);
    let text_app = r.app.label(lang);
    let text_lang = r.lang.label(lang);
    let target = r.target.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let line = match (lang, r.way) {
        (NoticeLang::De, NoticeWay::Clipboard) => format!(
            "Hinweis an Teilnehmende: am {at} bereitgestellt (in die Zwischenablage gelegt, {text_app}, {text_lang})."
        ),
        (NoticeLang::De, NoticeWay::Paste) => match target {
            Some(target) => format!(
                "Hinweis an Teilnehmende: am {at} Einfügen ausgelöst in {target} (Text: {text_app}, {text_lang})."
            ),
            None => format!(
                "Hinweis an Teilnehmende: am {at} Einfügen ausgelöst (Text: {text_app}, {text_lang})."
            ),
        },
        (NoticeLang::De, NoticeWay::Oral) => {
            format!("Hinweis an Teilnehmende: am {at} mündlich bestätigt.")
        }
        (NoticeLang::En, NoticeWay::Clipboard) => format!(
            "Notice to participants: provided on {at} (copied to the clipboard, {text_app}, {text_lang})."
        ),
        (NoticeLang::En, NoticeWay::Paste) => match target {
            Some(target) => format!(
                "Notice to participants: paste triggered in {target} on {at} (text: {text_app}, {text_lang})."
            ),
            None => format!(
                "Notice to participants: paste triggered on {at} (text: {text_app}, {text_lang})."
            ),
        },
        (NoticeLang::En, NoticeWay::Oral) => {
            format!("Notice to participants: confirmed as given orally on {at}.")
        }
    };
    Some(line)
}

// ---------------------------------------------------------------------------
// Nachweise vor dem Start
// ---------------------------------------------------------------------------

/// Ein offener Startdialog und die Nachweise, die in ihm entstanden sind.
struct PendingDialog {
    token: String,
    records: Vec<NoticeRecord>,
}

/// Nachweise, die vor dem Start einer Aufnahme entstehen (im Startdialog). Die
/// Besprechung gibt es noch nicht; der Recorder haengt sie nach erfolgreichem
/// Start an.
///
/// Der Nachweis gehoert zu GENAU EINEM Startdialog: der Dialog bekommt beim
/// Oeffnen ein zufaelliges Token ([`PendingNotices::open`]), jeder Nachweis und
/// der Start tragen es. Starts ohne Token (Hinweisfenster, Ablauf, Agent,
/// Termin) sehen nie einen Nachweis und beruehren ihn nicht. Oeffnet ein
/// zweiter Dialog, verfaellt der erste samt Nachweis; Schliessen verwirft ihn;
/// ein erfolgreicher Start verbraucht ihn.
pub struct PendingNotices {
    inner: Mutex<Option<PendingDialog>>,
}

impl PendingNotices {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Die Sperre haelt nur Daten: eine Vergiftung (Panic eines anderen Threads)
    /// darf nie dazu fuehren, dass kein Start mehr moeglich ist.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PendingDialog>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn matches(dialog: &PendingDialog, token: &str) -> bool {
        !token.is_empty() && dialog.token == token
    }

    /// Ein Startdialog oeffnet mit `token`: ein frueherer Dialog und sein
    /// Nachweis verfallen.
    pub fn open(&self, token: String) {
        *self.lock() = Some(PendingDialog {
            token,
            records: Vec::new(),
        });
    }

    /// Gibt es den Dialog `token` noch?
    pub fn is_open(&self, token: &str) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|d| Self::matches(d, token))
    }

    /// Haengt einen Nachweis an den Dialog `token`; `notice_dialog_gone`, wenn
    /// es ihn nicht (mehr) gibt.
    pub fn add(&self, token: &str, rec: NoticeRecord) -> Result<(), String> {
        let mut guard = self.lock();
        match guard.as_mut() {
            Some(dialog) if Self::matches(dialog, token) => {
                dialog.records.push(rec);
                if dialog.records.len() > MAX_RECORDS {
                    let drop = dialog.records.len() - MAX_RECORDS;
                    dialog.records.drain(..drop);
                }
                Ok(())
            }
            _ => Err(CODE_NOTICE_DIALOG_GONE.to_string()),
        }
    }

    /// Die noch geltenden Nachweise (juenger als [`PENDING_TTL_SECS`]) des
    /// Dialogs `token`. Ohne Token oder mit einem fremden: keine.
    pub fn fresh(&self, token: Option<&str>, now: i64) -> Vec<NoticeRecord> {
        let Some(token) = token else {
            return Vec::new();
        };
        match self.lock().as_ref() {
            Some(dialog) if Self::matches(dialog, token) => dialog
                .records
                .iter()
                .filter(|r| now - r.at < PENDING_TTL_SECS)
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Verwirft den Dialog `token` samt Nachweis (Schliessen, Abbrechen,
    /// erfolgreicher Start). Ein anderer, spaeter geoeffneter Dialog bleibt.
    pub fn discard(&self, token: &str) {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|d| Self::matches(d, token)) {
            *guard = None;
        }
    }
}

impl Default for PendingNotices {
    fn default() -> Self {
        Self::new()
    }
}

/// Der Prozess hat hoechstens einen offenen Startdialog.
pub static PENDING: PendingNotices = PendingNotices::new();

/// Wohin ein neuer Nachweis gehoert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTarget<'a> {
    /// An die laufende Besprechung `id`.
    Meeting(&'a str),
    /// An den offenen Startdialog mit diesem Token (die Besprechung gibt es
    /// noch nicht).
    Dialog(&'a str),
}

/// Ziel eines Nachweises: die Besprechung, sonst der offene Startdialog;
/// `notice_dialog_gone`, wenn weder eine Besprechung genannt ist noch das Token
/// zu einem offenen Dialog passt (geschlossen, ersetzt, schon gestartet).
pub fn resolve_target<'a>(
    pending: &PendingNotices,
    meeting_id: Option<&'a str>,
    dialog_token: Option<&'a str>,
) -> Result<NoticeTarget<'a>, String> {
    match (meeting_id, dialog_token) {
        (Some(id), _) => Ok(NoticeTarget::Meeting(id)),
        (None, Some(token)) if pending.is_open(token) => Ok(NoticeTarget::Dialog(token)),
        (None, _) => Err(CODE_NOTICE_DIALOG_GONE.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Pflicht
// ---------------------------------------------------------------------------

/// Darf eine Aufnahme starten? Gilt die Pflicht (Funktion an UND Pflicht an),
/// braucht sie einen geltenden Nachweis (Zwischenablage, Einfuegen oder
/// "muendlich informiert"). Sonst `notice_required`.
pub fn start_gate(settings: &MeetingNoticeSettings, fresh_evidence: usize) -> Result<(), String> {
    if settings.enabled && settings.required && fresh_evidence == 0 {
        Err(CODE_NOTICE_REQUIRED.to_string())
    } else {
        Ok(())
    }
}

/// Was ein Start an Nachweisen mitbringt: die geltenden Nachweise des
/// Startdialogs `token` zum Zeitpunkt der Pruefung (ein Schnappschuss: laeuft
/// die Gueltigkeit waehrend des Starts ab, geht der Nachweis nicht verloren),
/// oder `notice_required`, wenn die Pflicht gilt und keiner vorliegt. Ein Start
/// ohne Token bringt nie einen mit.
pub fn evidence_for_start(
    pending: &PendingNotices,
    settings: &MeetingNoticeSettings,
    token: Option<&str>,
    now: i64,
) -> Result<Vec<NoticeRecord>, String> {
    let fresh = pending.fresh(token, now);
    start_gate(settings, fresh.len())?;
    Ok(fresh)
}

// ---------------------------------------------------------------------------
// Einfuegen
// ---------------------------------------------------------------------------

/// Warum aus dem Einfuegen nur ein Kopieren wurde. Der Text liegt dann
/// ABSICHTLICH in der Zwischenablage (damit der Nutzer selbst einfuegen kann);
/// die Meldung der Oberflaeche sagt das.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum NoticeFallback {
    /// Kein anderes Fenster im Vordergrund gefunden.
    NoTarget,
    /// Im Vordergrund stand diese App selbst.
    OwnWindow,
    /// Im Vordergrund stand keine Besprechungs-App und kein Browser.
    NotMeetingApp,
    /// Der Fokus wechselte vor oder waehrend des Einfuegens.
    FocusChanged,
    /// Das Zielfenster laeuft mit hoeheren Rechten.
    TargetElevated,
    /// Der Tastendruck selbst scheiterte.
    InjectionFailed,
    /// macOS: keine Berechtigung fuer Tastendruecke.
    AccessibilityDenied,
}

/// Entscheidung nach Ablauf der Frist: gibt es ein fremdes Fenster im
/// Vordergrund (Prozess `foreground_pid`), in das eingefuegt werden darf?
pub fn paste_target_check(
    foreground_pid: Option<u32>,
    own_pid: u32,
) -> Result<u32, NoticeFallback> {
    match foreground_pid {
        None => Err(NoticeFallback::NoTarget),
        Some(pid) if pid == own_pid => Err(NoticeFallback::OwnWindow),
        Some(pid) => Ok(pid),
    }
}

/// Browser, die der Katalog der Besprechungs-Apps nicht kennt (Besprechungen
/// laufen auch im Browser).
const EXTRA_BROWSERS: &[(&str, &str)] = &[
    ("vivaldi.exe", "Vivaldi"),
    ("chromium.exe", "Chromium"),
    ("arc.exe", "Arc"),
];

/// Name der Besprechungs-App bzw. des Browsers zu einem Prozessnamen
/// (`ms-teams.exe`); `None` fuer alles andere. Quelle ist der Katalog der
/// Besprechungs-Erkennung (`meeting_detect::catalog`) plus [`EXTRA_BROWSERS`].
/// Rein: es gibt nur den Namen der Programmdatei her, nie einen Fenstertitel.
pub fn meeting_app_name(process: &str) -> Option<String> {
    let name = process.trim().to_ascii_lowercase();
    // Ein Pfadtrenner im Namen waere fuer den Katalog ein anderer Schluessel.
    if name.is_empty() || name.contains(['#', '\\', '/']) {
        return None;
    }
    match catalog::classify(&format!("np:{name}")) {
        AppClass::MeetingApp(label) | AppClass::Browser(label) => Some(label.to_string()),
        AppClass::Ignored | AppClass::Other => EXTRA_BROWSERS
            .iter()
            .find(|(exe, _)| *exe == name)
            .map(|(_, label)| (*label).to_string()),
    }
}

/// Darf in das Fenster im Vordergrund eingefuegt werden? Es muss ein fremdes
/// Fenster sein ([`paste_target_check`]) und zu einer Besprechungs-App oder
/// einem Browser gehoeren ([`meeting_app_name`]). `process_name` liefert den
/// Namen der Programmdatei zum Prozess; laesst er sich nicht lesen, gilt das
/// Fenster als unbekannt (fail closed). Ergebnis: der App-Name fuer den
/// Nachweis.
pub fn paste_target_decision(
    foreground_pid: Option<u32>,
    own_pid: u32,
    process_name: impl FnOnce(u32) -> Option<String>,
) -> Result<String, NoticeFallback> {
    let pid = paste_target_check(foreground_pid, own_pid)?;
    process_name(pid)
        .and_then(|name| meeting_app_name(&name))
        .ok_or(NoticeFallback::NotMeetingApp)
}

/// Frist in Sekunden begrenzen (0 wird zur Vorgabe).
pub fn clamp_countdown(secs: u32) -> u32 {
    if secs == 0 {
        PASTE_COUNTDOWN_DEFAULT_SECS
    } else {
        secs.min(PASTE_COUNTDOWN_MAX_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(at: i64, way: NoticeWay) -> NoticeRecord {
        NoticeRecord {
            at,
            way,
            app: NoticeApp::Teams,
            lang: NoticeLang::De,
            text: "Text".to_string(),
            in_export: true,
            target: None,
        }
    }

    fn pasted_in(at: i64, target: &str) -> NoticeRecord {
        NoticeRecord {
            target: Some(target.to_string()),
            ..rec(at, NoticeWay::Paste)
        }
    }

    fn stamp(secs: i64) -> String {
        format!("T{secs}")
    }

    // ---- Textbausteine und Platzhalter ------------------------------------

    #[test]
    fn every_app_and_language_has_a_builtin_text_with_all_three_placeholders() {
        for app in [NoticeApp::Teams, NoticeApp::Webex, NoticeApp::Neutral] {
            for lang in [NoticeLang::De, NoticeLang::En] {
                let t = builtin_template(app, lang);
                for p in ["{purpose}", "{processing}", "{retention}"] {
                    assert!(t.contains(p), "{app:?}/{lang:?} ohne {p}");
                }
                assert!(t.contains("Local Voice"), "{app:?}/{lang:?}");
                assert!(!t.contains("WAI"), "sichtbare Namen ohne WAI");
            }
        }
        assert!(builtin_template(NoticeApp::Teams, NoticeLang::De).contains("Teams"));
        assert!(builtin_template(NoticeApp::Webex, NoticeLang::En).contains("Webex"));
        assert!(!builtin_template(NoticeApp::Neutral, NoticeLang::De).contains("Teams"));
    }

    #[test]
    fn the_german_builtin_texts_use_real_umlauts() {
        let text = builtin_template(NoticeApp::Teams, NoticeLang::De);
        assert!(text.contains("unabhängig"));
        assert!(!text.contains("unabhaengig"));
        assert!(processing_sentence(NoticeLang::De, &[]).contains("übertragen"));
    }

    #[test]
    fn default_text_is_complete_and_has_no_open_placeholder() {
        let s = MeetingNoticeSettings::default();
        for app in [NoticeApp::Teams, NoticeApp::Webex, NoticeApp::Neutral] {
            for lang in [NoticeLang::De, NoticeLang::En] {
                let text = text_for(&s, app, lang, &[]);
                assert!(!text.contains('{') && !text.contains('}'), "{text}");
                assert!(!text.contains("  "), "doppelte Leerzeichen: {text}");
                assert!(!text.contains(". ."), "{text}");
            }
        }
        let de = text_for(&s, NoticeApp::Neutral, NoticeLang::De, &[]);
        assert_eq!(
            de,
            "Hinweis: Diese Besprechung wird mit Local Voice lokal auf meinem Rechner mitgeschrieben, um Notizen und ein Protokoll zu erstellen. Es werden keine Daten an Dritte übertragen. Wenn Sie damit nicht einverstanden sind, sagen Sie es bitte jetzt, dann beende ich die Mitschrift."
        );
    }

    #[test]
    fn purpose_retention_and_provider_fill_their_placeholders() {
        let s = MeetingNoticeSettings {
            purpose: "für das Protokoll des Projekts Nordlicht".to_string(),
            retention: "Die Aufnahme wird nach 30 Tagen gelöscht.".to_string(),
            ..Default::default()
        };
        let bedrock = vec!["Bedrock".to_string()];
        let de = text_for(&s, NoticeApp::Teams, NoticeLang::De, &bedrock);
        assert!(de.contains("mitgeschrieben, für das Protokoll des Projekts Nordlicht."));
        assert!(de.contains("Die Aufnahme wird nach 30 Tagen gelöscht."));
        assert!(de.contains("Für die Notizen wird der Text an Bedrock übermittelt."));
        assert!(!de.contains("keine Daten an Dritte"));
        let en = text_for(&s, NoticeApp::Webex, NoticeLang::En, &bedrock);
        assert!(en.contains("the text is sent to Bedrock"));
    }

    #[test]
    fn an_empty_retention_leaves_no_gap_and_a_blank_purpose_falls_back() {
        let s = MeetingNoticeSettings {
            purpose: "   ".to_string(),
            ..Default::default()
        };
        let text = text_for(&s, NoticeApp::Neutral, NoticeLang::En, &[]);
        assert!(text.contains(
            "transcribed with Local Voice locally on my computer, to create notes and minutes."
        ));
        assert!(!text.contains("  "));
    }

    #[test]
    fn german_placeholder_names_work_and_are_case_insensitive() {
        let values = NoticeValues {
            purpose: "Z".to_string(),
            retention: "A".to_string(),
            processing: "V".to_string(),
        };
        assert_eq!(
            render("{zweck}|{Aufbewahrung}|{ VERARBEITUNG }", &values),
            "Z|A|V"
        );
    }

    #[test]
    fn unknown_or_broken_placeholders_stay_and_values_are_not_expanded_twice() {
        let values = NoticeValues {
            purpose: "{retention}".to_string(),
            retention: "R".to_string(),
            processing: String::new(),
        };
        assert_eq!(render("{name} {purpose}", &values), "{name} {retention}");
        assert_eq!(render("offen {purpose", &values), "offen {purpose");
        assert_eq!(render("{ x {retention}", &values), "{ x R");
        assert_eq!(render("nur } zu", &values), "nur } zu");
        assert_eq!(render("", &values), "");
    }

    #[test]
    fn tidy_collapses_gaps_and_blank_lines() {
        let values = NoticeValues {
            purpose: String::new(),
            retention: String::new(),
            processing: String::new(),
        };
        assert_eq!(render("A  {retention}  B\n\n\n\nC \n", &values), "A B\n\nC");
    }

    fn values(purpose: &str) -> NoticeValues {
        NoticeValues {
            purpose: purpose.to_string(),
            retention: "R".to_string(),
            processing: "V".to_string(),
        }
    }

    /// Befund 11: `{{purpose}}` (so schreibt man es in i18next) laesst keine
    /// ueberzaehligen Klammern stehen.
    #[test]
    fn double_braces_are_placeholders_too_and_leave_no_stray_braces() {
        let v = values("Z");
        assert_eq!(render("a {{purpose}} b", &v), "a Z b");
        assert_eq!(render("{{ zweck }}|{{Retention}}", &v), "Z|R");
        assert_eq!(render("{purpose}|{{purpose}}", &v), "Z|Z");
        let out = render("Zweck: {{purpose}}. {{processing}}", &v);
        assert!(!out.contains('{') && !out.contains('}'), "{out}");
        assert_eq!(out, "Zweck: Z. V");
        // Unbekannt bleibt, wie geschrieben.
        assert_eq!(render("{{name}} {name}", &v), "{{name}} {name}");
        // Halb offen: der Rest bleibt wie er ist, der Platzhalter dahinter gilt.
        assert_eq!(render("{{purpose} x", &v), "{Z x");
        assert_eq!(render("{{purpose", &v), "{{purpose");
    }

    /// Befund 11: endet der Wert schon mit einem Satzzeichen, folgt kein zweiter
    /// Punkt aus dem Baustein.
    #[test]
    fn a_value_ending_in_a_full_stop_does_not_double_the_period() {
        assert_eq!(render("mit {purpose}. Danach", &values("Ziel.")), "mit Ziel. Danach");
        assert_eq!(render("mit {purpose}.", &values("Ziel!")), "mit Ziel!");
        assert_eq!(render("mit {purpose}.", &values("Ziel?")), "mit Ziel?");
        assert_eq!(render("mit {purpose}.", &values("Ziel")), "mit Ziel.");
        assert_eq!(render("mit {purpose}.", &values("Ziel. ")), "mit Ziel.");
        assert_eq!(
            render("mit {purpose}...", &values("Ziel.")),
            "mit Ziel....",
            "ein Auslassungszeichen bleibt, wie der Nutzer es schrieb"
        );
        assert_eq!(
            render("{purpose} Ende.", &values("Ziel.")),
            "Ziel. Ende.",
            "nur ein Punkt DIREKT hinter dem Platzhalter faellt weg"
        );
        let s = MeetingNoticeSettings {
            purpose: "um Notizen zu machen.".to_string(),
            ..Default::default()
        };
        let text = text_for(&s, NoticeApp::Neutral, NoticeLang::De, &[]);
        assert!(!text.contains(".."), "{text}");
        assert!(text.contains("zu machen. Es werden keine Daten"), "{text}");
    }

    /// Befund 11: unbekannte Platzhalter werden gemeldet, nicht still stehen
    /// gelassen.
    #[test]
    fn unknown_placeholders_are_reported_once_as_written() {
        assert!(unknown_placeholders("{purpose} {zweck} {{retention}} {processing}").is_empty());
        assert_eq!(
            unknown_placeholders("A {name} B {{Datum}} C {name} D {purpose}"),
            vec!["{name}".to_string(), "{{Datum}}".to_string()]
        );
        assert_eq!(unknown_placeholders("leer {} da"), vec!["{}".to_string()]);
        assert_eq!(
            unknown_placeholders("offen {purpose und } zu"),
            vec!["{purpose und }".to_string()],
            "ein geschlossenes Klammerpaar mit fremdem Inhalt ist ein unbekannter Platzhalter"
        );
        assert!(unknown_placeholders("offen {purpose").is_empty());
        assert!(unknown_placeholders("").is_empty());
        for app in [NoticeApp::Teams, NoticeApp::Webex, NoticeApp::Neutral] {
            for lang in [NoticeLang::De, NoticeLang::En] {
                assert!(
                    unknown_placeholders(builtin_template(app, lang)).is_empty(),
                    "{app:?}/{lang:?}"
                );
            }
        }
    }

    /// Befund 11: "keine Daten an Dritte" nur, wenn ALLE genutzten Anbieter lokal sind.
    #[test]
    fn the_third_party_claim_needs_every_used_provider_to_be_local() {
        let none = external_names([("Lokal", true), ("Mein Rechner", true)]);
        assert!(none.is_empty());
        assert!(processing_sentence(NoticeLang::De, &none).contains("keine Daten an Dritte"));

        let one = external_names([("Lokal", true), ("Bedrock", false)]);
        assert_eq!(one, vec!["Bedrock".to_string()]);
        let de = processing_sentence(NoticeLang::De, &one);
        assert_eq!(de, "Für die Notizen wird der Text an Bedrock übermittelt.");
        assert!(!de.contains("keine Daten an Dritte"));

        let two = external_names([
            ("OpenAI", false),
            (" Bedrock ", false),
            ("OpenAI", false),
            ("", false),
            ("Lokal", true),
        ]);
        assert_eq!(two, vec!["OpenAI".to_string(), "Bedrock".to_string()]);
        assert_eq!(
            processing_sentence(NoticeLang::De, &two),
            "Für die Notizen wird der Text an OpenAI und Bedrock übermittelt."
        );
        assert_eq!(
            processing_sentence(NoticeLang::En, &two),
            "For the notes, the text is sent to OpenAI and Bedrock."
        );
        let three: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            processing_sentence(NoticeLang::En, &three),
            "For the notes, the text is sent to A, B and C."
        );
        assert_eq!(
            processing_sentence(NoticeLang::De, &three),
            "Für die Notizen wird der Text an A, B und C übermittelt."
        );
        assert!(processing_sentence(NoticeLang::En, &[]).contains("No data is sent to third parties"));
        let blank = vec!["  ".to_string()];
        assert!(
            processing_sentence(NoticeLang::De, &blank).contains("keine Daten an Dritte"),
            "ein leerer Name ist kein Anbieter"
        );
    }

    #[test]
    fn a_user_template_replaces_the_builtin_only_for_its_app_and_language() {
        let s = MeetingNoticeSettings {
            templates: vec![NoticeTemplate {
                app: NoticeApp::Webex,
                lang: NoticeLang::De,
                text: "Eigener Text: {zweck}".to_string(),
            }],
            ..Default::default()
        };
        assert_eq!(
            text_for(&s, NoticeApp::Webex, NoticeLang::De, &[]),
            "Eigener Text: um Notizen und ein Protokoll zu erstellen"
        );
        assert_eq!(
            template_for(&s, NoticeApp::Webex, NoticeLang::En),
            builtin_template(NoticeApp::Webex, NoticeLang::En)
        );
        assert_eq!(
            template_for(&s, NoticeApp::Teams, NoticeLang::De),
            builtin_template(NoticeApp::Teams, NoticeLang::De)
        );
    }

    #[test]
    fn a_blank_user_template_falls_back_to_the_builtin() {
        let s = MeetingNoticeSettings {
            templates: vec![NoticeTemplate {
                app: NoticeApp::Teams,
                lang: NoticeLang::De,
                text: "  \n ".to_string(),
            }],
            ..Default::default()
        };
        assert_eq!(
            template_for(&s, NoticeApp::Teams, NoticeLang::De),
            builtin_template(NoticeApp::Teams, NoticeLang::De)
        );
    }

    #[test]
    fn sanitizing_trims_caps_dedupes_and_drops_empty_templates() {
        let long = "x".repeat(MAX_TEMPLATE_CHARS + 50);
        let s = MeetingNoticeSettings {
            purpose: format!("  {}  ", "p".repeat(500)),
            retention: "  r  ".to_string(),
            templates: vec![
                NoticeTemplate {
                    app: NoticeApp::Teams,
                    lang: NoticeLang::De,
                    text: "alt".to_string(),
                },
                NoticeTemplate {
                    app: NoticeApp::Teams,
                    lang: NoticeLang::De,
                    text: long,
                },
                NoticeTemplate {
                    app: NoticeApp::Webex,
                    lang: NoticeLang::En,
                    text: "   ".to_string(),
                },
            ],
            ..Default::default()
        }
        .sanitized();
        assert_eq!(s.purpose.chars().count(), MAX_PURPOSE_CHARS);
        assert_eq!(s.retention, "r");
        assert_eq!(s.templates.len(), 1, "doppelt und leer fallen weg");
        assert_eq!(s.templates[0].text.chars().count(), MAX_TEMPLATE_CHARS);
    }

    #[test]
    fn defaults_are_on_and_not_required_and_old_settings_files_load() {
        let d = MeetingNoticeSettings::default();
        assert!(d.enabled && !d.required && d.export_line);
        let parsed: MeetingNoticeSettings =
            serde_json::from_value(json!({ "required": true })).unwrap();
        assert!(parsed.required && parsed.enabled && parsed.export_line);
        assert_eq!(parsed.default_app, NoticeApp::Neutral);
        assert_eq!(parsed.default_lang, NoticeLang::De);
    }

    // ---- Nachweis-Speicherung ---------------------------------------------

    #[test]
    fn a_record_is_stored_next_to_other_metadata_and_read_back() {
        let meta = json!({ "timeline": { "basis": "qpc" } });
        let value = with_record(Some(&meta), &rec(100, NoticeWay::Clipboard));
        let mut next = meta.clone();
        next[NOTICE_KEY] = value;
        assert_eq!(next["timeline"]["basis"], "qpc");
        let back = records_in(Some(&next));
        assert_eq!(back, vec![rec(100, NoticeWay::Clipboard)]);
        assert_eq!(next[NOTICE_KEY][0]["way"], "clipboard");
        assert_eq!(next[NOTICE_KEY][0]["app"], "teams");
        assert_eq!(next[NOTICE_KEY][0]["lang"], "de");
    }

    #[test]
    fn records_accumulate_oldest_first_and_are_capped() {
        let mut meta: Option<Value> = None;
        for i in 0..(MAX_RECORDS as i64 + 5) {
            let value = with_record(meta.as_ref(), &rec(i, NoticeWay::Clipboard));
            meta = Some(json!({ NOTICE_KEY: value }));
        }
        let all = records_in(meta.as_ref());
        assert_eq!(all.len(), MAX_RECORDS);
        assert_eq!(all.first().unwrap().at, 5, "die aeltesten fielen weg");
        assert_eq!(all.last().unwrap().at, MAX_RECORDS as i64 + 4);
    }

    #[test]
    fn unreadable_entries_are_skipped_not_fatal() {
        let meta = json!({ NOTICE_KEY: [ {"at": 5, "way": "oral"}, "kaputt", {"at": "x"} ] });
        let list = records_in(Some(&meta));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].way, NoticeWay::Oral);
        assert!(list[0].in_export, "fehlendes Feld: Standard an");
        assert!(records_in(None).is_empty());
        assert!(records_in(Some(&json!({ NOTICE_KEY: "kein array" }))).is_empty());
        assert!(records_in(Some(&json!("kein objekt"))).is_empty());
    }

    // ---- Zeile im Export ---------------------------------------------------

    #[test]
    fn export_line_names_when_and_how_but_never_the_text() {
        let de = NoticeLang::De;
        let line = export_line(&[rec(1_000, NoticeWay::Clipboard)], de, stamp).unwrap();
        assert_eq!(
            line,
            "Hinweis an Teilnehmende: am T1000 bereitgestellt (in die Zwischenablage gelegt, Teams, Deutsch)."
        );
        assert!(!line.contains("Text"));
        let oral = export_line(&[rec(3_000, NoticeWay::Oral)], de, stamp).unwrap();
        assert_eq!(
            oral,
            "Hinweis an Teilnehmende: am T3000 mündlich bestätigt."
        );
    }

    /// Befund 3: das Einfuegen ist nur AUSGELOEST, nicht nachgewiesen
    /// "eingefuegt"; genannt wird die App (Katalogname), nie ein Fenstertitel.
    #[test]
    fn a_paste_is_worded_as_triggered_in_the_named_app_not_as_inserted() {
        let de = NoticeLang::De;
        let line = export_line(&[pasted_in(2_000, "Microsoft Teams")], de, stamp).unwrap();
        assert_eq!(
            line,
            "Hinweis an Teilnehmende: am T2000 Einfügen ausgelöst in Microsoft Teams (Text: Teams, Deutsch)."
        );
        assert!(!line.contains("eingefügt"), "{line}");
        let legacy = export_line(&[rec(2_000, NoticeWay::Paste)], de, stamp).unwrap();
        assert_eq!(
            legacy,
            "Hinweis an Teilnehmende: am T2000 Einfügen ausgelöst (Text: Teams, Deutsch)."
        );
        let blank = NoticeRecord {
            target: Some("  ".to_string()),
            ..rec(2_000, NoticeWay::Paste)
        };
        assert_eq!(export_line(&[blank], de, stamp).unwrap(), legacy);
    }

    /// Befund 9: die Zeile folgt der Sprache des Exports.
    #[test]
    fn the_export_line_follows_the_language_of_the_export() {
        let en = NoticeLang::En;
        let neutral = NoticeRecord {
            app: NoticeApp::Neutral,
            ..rec(1_000, NoticeWay::Clipboard)
        };
        assert_eq!(
            export_line(&[neutral], en, stamp).unwrap(),
            "Notice to participants: provided on T1000 (copied to the clipboard, neutral text, German)."
        );
        let english_text = NoticeRecord {
            lang: NoticeLang::En,
            ..pasted_in(2_000, "Chrome")
        };
        assert_eq!(
            export_line(&[english_text], en, stamp).unwrap(),
            "Notice to participants: paste triggered in Chrome on T2000 (text: Teams, English)."
        );
        assert_eq!(
            export_line(&[rec(3_000, NoticeWay::Oral)], en, stamp).unwrap(),
            "Notice to participants: confirmed as given orally on T3000."
        );
        let no_target = export_line(&[rec(2_000, NoticeWay::Paste)], en, stamp).unwrap();
        assert_eq!(
            no_target,
            "Notice to participants: paste triggered on T2000 (text: Teams, German)."
        );
    }

    #[test]
    fn the_language_of_the_export_comes_from_the_meeting_language() {
        assert_eq!(NoticeLang::for_language(Some("en")), NoticeLang::En);
        assert_eq!(NoticeLang::for_language(Some("EN-us")), NoticeLang::En);
        assert_eq!(NoticeLang::for_language(Some("en_GB")), NoticeLang::En);
        assert_eq!(NoticeLang::for_language(Some("de")), NoticeLang::De);
        assert_eq!(NoticeLang::for_language(Some("fr")), NoticeLang::De);
        assert_eq!(NoticeLang::for_language(Some("")), NoticeLang::De);
        assert_eq!(NoticeLang::for_language(None), NoticeLang::De);
    }

    /// Die Zeitangabe stammt vom Aufrufer; der Nachweis (target) laeuft durch
    /// die Ablage in `metadata_json` und kommt unveraendert zurueck.
    #[test]
    fn the_target_app_survives_the_metadata_round_trip_and_old_records_load() {
        let meta = json!({});
        let value = with_record(Some(&meta), &pasted_in(5, "Webex"));
        let back = records_in(Some(&json!({ NOTICE_KEY: value.clone() })));
        assert_eq!(back[0].target.as_deref(), Some("Webex"));
        assert_eq!(value[0]["target"], "Webex");
        let old = json!({ NOTICE_KEY: [ {"at": 5, "way": "paste", "app": "teams", "lang": "de", "text": "x", "in_export": true} ] });
        let list = records_in(Some(&old));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].target, None, "aelterer Nachweis ohne App");
    }

    #[test]
    fn the_export_prefers_the_strongest_way_then_the_earliest() {
        let list = [
            rec(10, NoticeWay::Clipboard),
            rec(20, NoticeWay::Oral),
            rec(40, NoticeWay::Paste),
            rec(30, NoticeWay::Paste),
        ];
        assert_eq!(export_record(&list).unwrap().at, 30);
        let weak = [rec(50, NoticeWay::Clipboard), rec(20, NoticeWay::Clipboard)];
        assert_eq!(export_record(&weak).unwrap().at, 20);
    }

    #[test]
    fn records_marked_not_for_export_give_no_line() {
        let mut hidden = rec(10, NoticeWay::Paste);
        hidden.in_export = false;
        assert_eq!(export_line(&[hidden.clone()], NoticeLang::De, stamp), None);
        let shown = rec(20, NoticeWay::Clipboard);
        let line = export_line(&[hidden, shown], NoticeLang::De, stamp).unwrap();
        assert!(
            line.contains("T20"),
            "nur der sichtbare Eintrag zaehlt: {line}"
        );
        assert_eq!(export_line(&[], NoticeLang::De, stamp), None);
    }

    // ---- Pflicht -----------------------------------------------------------

    #[test]
    fn the_personal_default_never_blocks_a_start() {
        assert_eq!(start_gate(&MeetingNoticeSettings::default(), 0), Ok(()));
    }

    #[test]
    fn required_mode_blocks_until_there_is_evidence() {
        let s = MeetingNoticeSettings {
            required: true,
            ..Default::default()
        };
        assert_eq!(start_gate(&s, 0), Err(CODE_NOTICE_REQUIRED.to_string()));
        assert_eq!(start_gate(&s, 1), Ok(()));
    }

    #[test]
    fn required_mode_without_the_feature_does_not_block() {
        let s = MeetingNoticeSettings {
            enabled: false,
            required: true,
            ..Default::default()
        };
        assert_eq!(start_gate(&s, 0), Ok(()));
    }

    // ---- Nachweise vor dem Start ------------------------------------------

    fn required() -> MeetingNoticeSettings {
        MeetingNoticeSettings {
            required: true,
            ..Default::default()
        }
    }

    #[test]
    fn pending_evidence_belongs_to_its_dialog_and_expires_after_five_minutes() {
        assert_eq!(PENDING_TTL_SECS, 300);
        let pending = PendingNotices::new();
        pending.open("T1".to_string());
        pending.add("T1", rec(1_000, NoticeWay::Oral)).unwrap();
        assert_eq!(pending.fresh(Some("T1"), 1_000 + 299).len(), 1);
        assert!(
            pending.fresh(Some("T1"), 1_000 + PENDING_TTL_SECS).is_empty(),
            "nach fuenf Minuten abgelaufen"
        );
    }

    #[test]
    fn pending_evidence_is_capped_and_an_unknown_dialog_cannot_add() {
        let pending = PendingNotices::new();
        assert_eq!(
            pending.add("T1", rec(1, NoticeWay::Oral)),
            Err(CODE_NOTICE_DIALOG_GONE.to_string()),
            "kein Dialog offen"
        );
        pending.open("T1".to_string());
        for i in 0..(MAX_RECORDS as i64 + 3) {
            pending.add("T1", rec(i, NoticeWay::Clipboard)).unwrap();
        }
        assert_eq!(pending.fresh(Some("T1"), 100).len(), MAX_RECORDS);
        assert_eq!(
            pending.add("fremd", rec(1, NoticeWay::Oral)),
            Err(CODE_NOTICE_DIALOG_GONE.to_string())
        );
        assert_eq!(pending.fresh(Some("T1"), 100).len(), MAX_RECORDS);
    }

    #[test]
    fn a_new_evidence_goes_to_the_meeting_or_to_the_open_dialog_only() {
        let pending = PendingNotices::new();
        assert_eq!(
            resolve_target(&pending, Some("m1"), None),
            Ok(NoticeTarget::Meeting("m1"))
        );
        assert_eq!(
            resolve_target(&pending, Some("m1"), Some("egal")),
            Ok(NoticeTarget::Meeting("m1"))
        );
        let gone = Err(CODE_NOTICE_DIALOG_GONE.to_string());
        assert_eq!(resolve_target(&pending, None, None), gone);
        assert_eq!(resolve_target(&pending, None, Some("T")), gone);
        pending.open("T".to_string());
        assert_eq!(
            resolve_target(&pending, None, Some("T")),
            Ok(NoticeTarget::Dialog("T"))
        );
        assert_eq!(resolve_target(&pending, None, Some("fremd")), gone);
        pending.discard("T");
        assert_eq!(resolve_target(&pending, None, Some("T")), gone);
    }

    #[test]
    fn an_empty_token_never_matches() {
        let pending = PendingNotices::new();
        pending.open(String::new());
        assert!(!pending.is_open(""));
        assert!(pending.add("", rec(1, NoticeWay::Oral)).is_err());
        assert!(pending.fresh(Some(""), 1).is_empty());
    }

    /// Befund 1: Abbruch verwirft den Nachweis; ein spaeter geoeffneter Dialog
    /// bleibt vom Schliessen des alten unberuehrt.
    #[test]
    fn closing_the_dialog_discards_its_evidence_and_only_its_own() {
        let pending = PendingNotices::new();
        pending.open("alt".to_string());
        pending.add("alt", rec(1_000, NoticeWay::Clipboard)).unwrap();
        pending.discard("alt");
        assert!(pending.fresh(Some("alt"), 1_010).is_empty(), "Abbruch verwirft");
        assert!(!pending.is_open("alt"));
        assert_eq!(
            evidence_for_start(&pending, &required(), Some("alt"), 1_010),
            Err(CODE_NOTICE_REQUIRED.to_string())
        );

        pending.open("neu".to_string());
        pending.add("neu", rec(1_020, NoticeWay::Oral)).unwrap();
        pending.discard("alt");
        assert_eq!(
            pending.fresh(Some("neu"), 1_030).len(),
            1,
            "das spaete Schliessen des alten Dialogs trifft den neuen nicht"
        );
    }

    /// Befund 1: ein zweiter Dialog verwirft den ersten samt Nachweis.
    #[test]
    fn opening_a_second_dialog_replaces_the_first() {
        let pending = PendingNotices::new();
        pending.open("eins".to_string());
        pending.add("eins", rec(1_000, NoticeWay::Oral)).unwrap();
        pending.open("zwei".to_string());
        assert!(pending.fresh(Some("eins"), 1_001).is_empty());
        assert!(pending.fresh(Some("zwei"), 1_001).is_empty(), "leer geoeffnet");
        assert_eq!(
            pending.add("eins", rec(1_002, NoticeWay::Oral)),
            Err(CODE_NOTICE_DIALOG_GONE.to_string())
        );
    }

    /// Befund 1: ein Start ohne Token (Hinweisfenster, Ablauf, Agent, Termin)
    /// scheitert bei Pflicht, auch wenn im Startdialog ein Nachweis liegt, und
    /// beruehrt diesen Nachweis nicht.
    #[test]
    fn a_start_without_a_token_never_sees_the_evidence_of_an_open_dialog() {
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", rec(1_000, NoticeWay::Clipboard)).unwrap();
        assert_eq!(
            evidence_for_start(&pending, &required(), None, 1_001),
            Err(CODE_NOTICE_REQUIRED.to_string())
        );
        assert_eq!(
            evidence_for_start(&pending, &required(), Some("fremd"), 1_001),
            Err(CODE_NOTICE_REQUIRED.to_string()),
            "ein fremdes Token auch nicht"
        );
        let ok = evidence_for_start(&pending, &required(), Some("T"), 1_001).unwrap();
        assert_eq!(ok.len(), 1, "der Nachweis des Dialogs war unberuehrt");
        // Ohne Pflicht startet ein Start ohne Token wie bisher, ohne Nachweis.
        let free = evidence_for_start(&pending, &MeetingNoticeSettings::default(), None, 1_001);
        assert_eq!(free, Ok(Vec::new()));
    }

    /// Befund 1: nach erfolgreichem Start ist der Nachweis verbraucht; eine
    /// zweite Aufnahme bekommt ihn nicht (weder mit dem alten Token noch aus
    /// einem neuen Dialog).
    #[test]
    fn a_consumed_dialog_gives_a_second_recording_no_evidence() {
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", rec(1_000, NoticeWay::Clipboard)).unwrap();
        let first = evidence_for_start(&pending, &required(), Some("T"), 1_010).unwrap();
        assert_eq!(first.len(), 1);
        pending.discard("T"); // was der Recorder nach dem erfolgreichen Start tut
        assert_eq!(
            evidence_for_start(&pending, &required(), Some("T"), 1_020),
            Err(CODE_NOTICE_REQUIRED.to_string()),
            "dasselbe Token ein zweites Mal"
        );
        pending.open("T2".to_string());
        assert_eq!(
            evidence_for_start(&pending, &required(), Some("T2"), 1_030),
            Err(CODE_NOTICE_REQUIRED.to_string()),
            "ein neuer Dialog beginnt leer"
        );
    }

    /// Abgelaufener Nachweis zaehlt nicht; ein beim Pruefen noch gueltiger
    /// bleibt als Schnappschuss, auch wenn der Start laenger dauert.
    #[test]
    fn expired_evidence_does_not_pass_the_gate() {
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", rec(1_000, NoticeWay::Oral)).unwrap();
        let late = 1_000 + PENDING_TTL_SECS;
        assert_eq!(
            evidence_for_start(&pending, &required(), Some("T"), late),
            Err(CODE_NOTICE_REQUIRED.to_string())
        );
        let snapshot =
            evidence_for_start(&pending, &required(), Some("T"), late - 1).unwrap();
        assert_eq!(snapshot.len(), 1, "der Schnappschuss ist unabhaengig von der Uhr danach");
    }

    #[test]
    fn a_poisoned_lock_does_not_block_later_starts() {
        static P: PendingNotices = PendingNotices::new();
        let _ = std::thread::spawn(|| {
            let _guard = P.lock();
            panic!("absichtlich");
        })
        .join();
        P.open("T".to_string());
        assert!(P.add("T", rec(1, NoticeWay::Oral)).is_ok());
        assert_eq!(P.fresh(Some("T"), 2).len(), 1);
    }

    // ---- Einfuegen -----------------------------------------------------------

    #[test]
    fn pasting_needs_a_foreign_foreground_window() {
        assert_eq!(paste_target_check(None, 10), Err(NoticeFallback::NoTarget));
        assert_eq!(
            paste_target_check(Some(10), 10),
            Err(NoticeFallback::OwnWindow)
        );
        assert_eq!(paste_target_check(Some(77), 10), Ok(77));
    }

    /// Befund 2: nur Besprechungs-Apps und Browser sind Ziele.
    #[test]
    fn meeting_apps_and_browsers_are_targets_and_nothing_else() {
        assert_eq!(
            meeting_app_name("ms-teams.exe").as_deref(),
            Some("Microsoft Teams")
        );
        assert_eq!(
            meeting_app_name("Teams.exe").as_deref(),
            Some("Microsoft Teams"),
            "Gross-/Kleinschreibung egal"
        );
        assert_eq!(meeting_app_name("CiscoCollabHost.exe").as_deref(), Some("Webex"));
        assert_eq!(meeting_app_name("Zoom.exe").as_deref(), Some("Zoom"));
        assert_eq!(meeting_app_name("chrome.exe").as_deref(), Some("Chrome"));
        assert_eq!(meeting_app_name("msedge.exe").as_deref(), Some("Edge"));
        assert_eq!(meeting_app_name("firefox.exe").as_deref(), Some("Firefox"));
        assert_eq!(meeting_app_name("vivaldi.exe").as_deref(), Some("Vivaldi"));
        assert_eq!(meeting_app_name("chromium.exe").as_deref(), Some("Chromium"));
        for other in [
            "notepad.exe",
            "WINWORD.EXE",
            "explorer.exe",
            "cmd.exe",
            "code.exe",
            "msedgewebview2.exe",
            "local-voice-ai.exe",
            "",
            "   ",
            "teams",
        ] {
            assert_eq!(meeting_app_name(other), None, "{other}");
        }
    }

    #[test]
    fn a_path_separator_in_the_name_cannot_pose_as_a_meeting_app() {
        assert_eq!(meeting_app_name("evil#teams.exe"), None);
        assert_eq!(meeting_app_name(r"C:\x\teams.exe"), None);
        assert_eq!(meeting_app_name("a/zoom.exe"), None);
    }

    #[test]
    fn the_target_decision_checks_window_then_program() {
        let name = |pid: u32| match pid {
            10 => Some("local-voice-ai.exe".to_string()),
            20 => Some("ms-teams.exe".to_string()),
            30 => Some("notepad.exe".to_string()),
            _ => None,
        };
        assert_eq!(
            paste_target_decision(None, 10, name),
            Err(NoticeFallback::NoTarget)
        );
        assert_eq!(
            paste_target_decision(Some(10), 10, name),
            Err(NoticeFallback::OwnWindow),
            "das eigene Fenster, auch wenn der Name bekannt waere"
        );
        assert_eq!(
            paste_target_decision(Some(20), 10, name),
            Ok("Microsoft Teams".to_string())
        );
        assert_eq!(
            paste_target_decision(Some(30), 10, name),
            Err(NoticeFallback::NotMeetingApp)
        );
        assert_eq!(
            paste_target_decision(Some(99), 10, name),
            Err(NoticeFallback::NotMeetingApp),
            "Name nicht lesbar: fail closed"
        );
    }

    #[test]
    fn the_program_name_is_only_asked_for_a_foreign_window() {
        let ask = |_pid: u32| -> Option<String> { panic!("nicht gefragt") };
        assert_eq!(
            paste_target_decision(Some(5), 5, ask),
            Err(NoticeFallback::OwnWindow)
        );
        assert_eq!(
            paste_target_decision(None, 5, ask),
            Err(NoticeFallback::NoTarget)
        );
    }

    #[test]
    fn the_countdown_is_bounded() {
        assert_eq!(clamp_countdown(0), PASTE_COUNTDOWN_DEFAULT_SECS);
        assert_eq!(clamp_countdown(3), 3);
        assert_eq!(clamp_countdown(900), PASTE_COUNTDOWN_MAX_SECS);
    }

    #[test]
    fn enums_serialize_in_snake_case_for_the_settings_file_and_the_ui() {
        assert_eq!(serde_json::to_value(NoticeWay::Paste).unwrap(), "paste");
        assert_eq!(
            serde_json::to_value(NoticeFallback::OwnWindow).unwrap(),
            "own_window"
        );
        assert_eq!(serde_json::to_value(NoticeApp::Webex).unwrap(), "webex");
        assert_eq!(serde_json::to_value(NoticeLang::En).unwrap(), "en");
    }
}
