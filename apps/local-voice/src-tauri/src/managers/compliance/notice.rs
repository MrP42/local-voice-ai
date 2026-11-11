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

/// Schluessel in `meetings.metadata_json`.
pub const NOTICE_KEY: &str = "participant_notice";
/// So viele Nachweise merkt sich eine Besprechung hoechstens (die aeltesten
/// fallen weg).
pub const MAX_RECORDS: usize = 20;
/// Ein Nachweis vor dem Start gilt so lange fuer die naechste Aufnahme.
pub const PENDING_TTL_SECS: i64 = 30 * 60;
/// Fehlercode, wenn die Pflicht gilt und noch kein Hinweis vorliegt.
pub const CODE_NOTICE_REQUIRED: &str = "notice_required";
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
    fn label(self) -> &'static str {
        match self {
            NoticeApp::Teams => "Teams",
            NoticeApp::Webex => "Webex",
            NoticeApp::Neutral => "neutraler Text",
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
    fn label(self) -> &'static str {
        match self {
            NoticeLang::De => "Deutsch",
            NoticeLang::En => "Englisch",
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

/// Satz zur Verarbeitung: lokal oder mit Anbieter (`Some(Name)`).
pub fn processing_sentence(lang: NoticeLang, provider: Option<&str>) -> String {
    match (lang, provider.map(str::trim).filter(|p| !p.is_empty())) {
        (NoticeLang::De, None) => "Es werden keine Daten an Dritte übertragen.".to_string(),
        (NoticeLang::De, Some(p)) => format!("Für die Notizen wird der Text an {p} übermittelt."),
        (NoticeLang::En, None) => "No data is sent to third parties.".to_string(),
        (NoticeLang::En, Some(p)) => format!("For the notes, the text is sent to {p}."),
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
        provider: Option<&str>,
    ) -> Self {
        let purpose = settings.purpose.trim();
        Self {
            purpose: if purpose.is_empty() {
                default_purpose(lang).to_string()
            } else {
                purpose.to_string()
            },
            retention: settings.retention.trim().to_string(),
            processing: processing_sentence(lang, provider),
        }
    }

    fn get(&self, name: &str) -> Option<&str> {
        match name {
            "purpose" | "zweck" => Some(&self.purpose),
            "retention" | "aufbewahrung" => Some(&self.retention),
            "processing" | "verarbeitung" => Some(&self.processing),
            _ => None,
        }
    }
}

/// Setzt die Platzhalter ein (ein Durchgang: ein Wert, der selbst `{...}`
/// enthaelt, wird nicht noch einmal ersetzt; unbekannte Platzhalter bleiben
/// stehen) und raeumt die Leerraeume auf, die ein leerer Wert hinterlaesst.
pub fn render(template: &str, values: &NoticeValues) -> String {
    let mut out = String::with_capacity(template.len() + 128);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let close = after.find('}');
        let reopen = after.find('{');
        let end = match (close, reopen) {
            (Some(c), Some(o)) if o < c => None,
            (c, _) => c,
        };
        match end {
            Some(end) => {
                let name = after[..end].trim().to_lowercase();
                match values.get(&name) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(&after[..end]);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
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

/// Der fertige Text fuer `app` und `lang`.
pub fn text_for(
    settings: &MeetingNoticeSettings,
    app: NoticeApp,
    lang: NoticeLang,
    provider: Option<&str>,
) -> String {
    render(
        &template_for(settings, app, lang),
        &NoticeValues::from_settings(settings, lang, provider),
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
/// plus `rec`, hoechstens [`MAX_RECORDS`].
pub fn with_record(meta: Option<&Value>, rec: &NoticeRecord) -> Value {
    let mut records = records_in(meta);
    records.push(rec.clone());
    if records.len() > MAX_RECORDS {
        let drop = records.len() - MAX_RECORDS;
        records.drain(..drop);
    }
    serde_json::to_value(records).unwrap_or(Value::Null)
}

/// Der Eintrag, der in den Export kommt: der staerkste Weg, bei Gleichstand
/// der fruehste.
pub fn export_record(records: &[NoticeRecord]) -> Option<&NoticeRecord> {
    records
        .iter()
        .filter(|r| r.in_export)
        .min_by_key(|r| (std::cmp::Reverse(r.way.strength()), r.at))
}

/// Die eine Zeile im Protokollexport; `when` formatiert Unix-Sekunden als
/// Ortszeit ("2026-11-11 14:32"). Enthaelt nie den Hinweistext selbst.
pub fn export_line(records: &[NoticeRecord], when: impl Fn(i64) -> String) -> Option<String> {
    let r = export_record(records)?;
    let at = when(r.at);
    let how = match r.way {
        NoticeWay::Clipboard => format!(
            "am {at} bereitgestellt (in die Zwischenablage gelegt, {}, {})",
            r.app.label(),
            r.lang.label()
        ),
        NoticeWay::Paste => format!(
            "am {at} in den Chat eingefügt ({}, {})",
            r.app.label(),
            r.lang.label()
        ),
        NoticeWay::Oral => format!("am {at} mündlich bestätigt"),
    };
    Some(format!("Hinweis an Teilnehmende: {how}."))
}

// ---------------------------------------------------------------------------
// Nachweise vor dem Start
// ---------------------------------------------------------------------------

/// Nachweise, die vor dem Start einer Aufnahme entstehen (im Startdialog). Die
/// Besprechung gibt es noch nicht; der Recorder haengt sie nach erfolgreichem
/// Start an.
pub struct PendingNotices {
    inner: Mutex<Vec<NoticeRecord>>,
}

impl PendingNotices {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<NoticeRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn add(&self, rec: NoticeRecord) {
        let mut list = self.lock();
        list.push(rec);
        if list.len() > MAX_RECORDS {
            let drop = list.len() - MAX_RECORDS;
            list.drain(..drop);
        }
    }

    /// Die noch geltenden Nachweise (juenger als [`PENDING_TTL_SECS`]).
    pub fn fresh(&self, now: i64) -> Vec<NoticeRecord> {
        self.lock()
            .iter()
            .filter(|r| now - r.at < PENDING_TTL_SECS)
            .cloned()
            .collect()
    }

    /// Entnimmt die geltenden Nachweise; alles andere wird verworfen.
    pub fn take_fresh(&self, now: i64) -> Vec<NoticeRecord> {
        let mut list = self.lock();
        let fresh = list
            .iter()
            .filter(|r| now - r.at < PENDING_TTL_SECS)
            .cloned()
            .collect();
        list.clear();
        fresh
    }

    pub fn clear(&self) {
        self.lock().clear();
    }
}

impl Default for PendingNotices {
    fn default() -> Self {
        Self::new()
    }
}

/// Der Prozess teilt sich einen Satz Nachweise vor dem Start.
pub static PENDING: PendingNotices = PendingNotices::new();

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

// ---------------------------------------------------------------------------
// Einfuegen
// ---------------------------------------------------------------------------

/// Warum aus dem Einfuegen nur ein Kopieren wurde. Der Text liegt dann
/// nachweislich in der Zwischenablage.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum NoticeFallback {
    /// Kein anderes Fenster im Vordergrund gefunden.
    NoTarget,
    /// Im Vordergrund stand diese App selbst.
    OwnWindow,
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
        assert!(processing_sentence(NoticeLang::De, None).contains("übertragen"));
    }

    #[test]
    fn default_text_is_complete_and_has_no_open_placeholder() {
        let s = MeetingNoticeSettings::default();
        for app in [NoticeApp::Teams, NoticeApp::Webex, NoticeApp::Neutral] {
            for lang in [NoticeLang::De, NoticeLang::En] {
                let text = text_for(&s, app, lang, None);
                assert!(!text.contains('{') && !text.contains('}'), "{text}");
                assert!(!text.contains("  "), "doppelte Leerzeichen: {text}");
                assert!(!text.contains(". ."), "{text}");
            }
        }
        let de = text_for(&s, NoticeApp::Neutral, NoticeLang::De, None);
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
        let de = text_for(&s, NoticeApp::Teams, NoticeLang::De, Some("Bedrock"));
        assert!(de.contains("mitgeschrieben, für das Protokoll des Projekts Nordlicht."));
        assert!(de.contains("Die Aufnahme wird nach 30 Tagen gelöscht."));
        assert!(de.contains("Für die Notizen wird der Text an Bedrock übermittelt."));
        assert!(!de.contains("keine Daten an Dritte"));
        let en = text_for(&s, NoticeApp::Webex, NoticeLang::En, Some("Bedrock"));
        assert!(en.contains("the text is sent to Bedrock"));
    }

    #[test]
    fn an_empty_retention_leaves_no_gap_and_a_blank_purpose_falls_back() {
        let s = MeetingNoticeSettings {
            purpose: "   ".to_string(),
            ..Default::default()
        };
        let text = text_for(&s, NoticeApp::Neutral, NoticeLang::En, None);
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
            text_for(&s, NoticeApp::Webex, NoticeLang::De, None),
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
        let line = export_line(&[rec(1_000, NoticeWay::Clipboard)], stamp).unwrap();
        assert_eq!(
            line,
            "Hinweis an Teilnehmende: am T1000 bereitgestellt (in die Zwischenablage gelegt, Teams, Deutsch)."
        );
        assert!(!line.contains("Text"));
        let pasted = export_line(&[rec(2_000, NoticeWay::Paste)], stamp).unwrap();
        assert_eq!(
            pasted,
            "Hinweis an Teilnehmende: am T2000 in den Chat eingefügt (Teams, Deutsch)."
        );
        let oral = export_line(&[rec(3_000, NoticeWay::Oral)], stamp).unwrap();
        assert_eq!(
            oral,
            "Hinweis an Teilnehmende: am T3000 mündlich bestätigt."
        );
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
        assert_eq!(export_line(&[hidden.clone()], stamp), None);
        let shown = rec(20, NoticeWay::Clipboard);
        let line = export_line(&[hidden, shown], stamp).unwrap();
        assert!(
            line.contains("T20"),
            "nur der sichtbare Eintrag zaehlt: {line}"
        );
        assert_eq!(export_line(&[], stamp), None);
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

    #[test]
    fn pending_evidence_is_taken_once_and_expires() {
        let pending = PendingNotices::new();
        pending.add(rec(1_000, NoticeWay::Oral));
        assert_eq!(pending.fresh(1_000 + 60).len(), 1);
        assert_eq!(pending.take_fresh(1_000 + 60).len(), 1);
        assert!(pending.fresh(1_000 + 60).is_empty(), "entnommen ist weg");

        pending.add(rec(1_000, NoticeWay::Clipboard));
        let late = 1_000 + PENDING_TTL_SECS;
        assert!(pending.fresh(late).is_empty(), "abgelaufen");
        assert!(pending.take_fresh(late).is_empty());
    }

    #[test]
    fn pending_evidence_can_be_cleared_and_is_capped() {
        let pending = PendingNotices::new();
        for i in 0..(MAX_RECORDS as i64 + 3) {
            pending.add(rec(i, NoticeWay::Clipboard));
        }
        assert_eq!(pending.fresh(100).len(), MAX_RECORDS);
        pending.clear();
        assert!(pending.fresh(100).is_empty());
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
