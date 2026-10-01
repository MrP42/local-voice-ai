//! Katalog der Ausloeser und Bausteine (B1): welche Arten es gibt, welche Felder
//! sie kennen, welche Rechte sie brauchen und was sie planen.
//!
//! Der Katalog ist die EINE Quelle fuer die Pruefung beim Speichern, den
//! Trockenlauf und das JSON-Schema (`schema/lva-workflow-1.schema.json`; ein Test
//! haelt beide gleich). Ein neuer Ausloeser oder Baustein beginnt mit einer Zeile hier.
//!
//! Die Bausteine B2 bis B6 ersetzen den Katalogbaustein (`SpecAction`, laeuft nicht,
//! plant aber vollstaendig) durch eine echte Umsetzung gleicher Kennung
//! (`ActionRegistry::register`). Schaetzwerte (`HeavyNeed`) sind Platzhalter, die
//! der echte Baustein aus dem gewaehlten Modell berechnet.

use serde_json::{json, Map, Value};

use crate::managers::integrations::model::Capability;
use crate::managers::integrations::store::valid_id;

use super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::expr;

#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    Text,
    /// Kennung einer Integration des Registers.
    Id,
    Int {
        min: i64,
        max: i64,
    },
    Bool,
    TextList,
    Choice(&'static [&'static str]),
    Any,
}

#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub name: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    /// Nur feste Werte, nie `{{...}}`: Empfaenger und Adressen duerfen nicht aus
    /// Trigger-Daten, Variablen oder Modellausgaben entstehen.
    pub literal: bool,
}

const fn field(name: &'static str, kind: FieldKind, required: bool) -> FieldSpec {
    FieldSpec {
        name,
        kind,
        required,
        literal: false,
    }
}

const fn literal(name: &'static str, kind: FieldKind, required: bool) -> FieldSpec {
    FieldSpec {
        name,
        kind,
        required,
        literal: true,
    }
}

// ---------------------------------------------------------------------------
// Ausloeser
// ---------------------------------------------------------------------------

pub struct TriggerSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub fields: &'static [FieldSpec],
    /// Felder, die der Ausloeser unter `trigger.<feld>` liefert (leer: frei).
    pub provides: &'static [&'static str],
    /// Beispieldaten fuer den Trockenlauf aus den Feldern des Ausloesers.
    pub sample: fn(&Map<String, Value>) -> Value,
}

fn sample_empty(_: &Map<String, Value>) -> Value {
    json!({})
}

fn sample_schedule(_: &Map<String, Value>) -> Value {
    json!({"scheduled_for": "2026-10-02T08:00:00Z"})
}

fn sample_calendar(p: &Map<String, Value>) -> Value {
    let cal = p
        .get("integration")
        .and_then(Value::as_str)
        .unwrap_or("cal-beispiel");
    json!({
        "calendar": cal,
        "event_id": "beispiel-termin-1",
        "title": "Beispieltermin",
        "start": "2026-10-02T10:00:00Z",
        "end": "2026-10-02T11:00:00Z",
        "attendees": [
            {"email": "ich@beispiel.invalid", "is_self": true},
            {"email": "kunde@beispiel.invalid", "is_self": false}
        ],
        "external_attendees": 1,
        "online": true,
        "meeting": {"title": "Beispieltermin"}
    })
}

fn sample_meeting(_: &Map<String, Value>) -> Value {
    json!({
        "meeting_id": "beispiel-besprechung-1",
        "title": "Beispielbesprechung",
        "stage": "minutes",
        "meeting": {"title": "Beispielbesprechung"}
    })
}

fn sample_folder(p: &Map<String, Value>) -> Value {
    let cal = p
        .get("integration")
        .and_then(Value::as_str)
        .unwrap_or("ordner-beispiel");
    json!({
        "integration": cal,
        "path": "C:/Eingang/Beispiel.wav",
        "name": "Beispiel.wav",
        "extension": "wav",
        "size": 1048576
    })
}

fn sample_youtube(p: &Map<String, Value>) -> Value {
    let channel = p
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or("UCbeispiel");
    json!({
        "channel_id": channel,
        "channel_title": "Beispielkanal",
        "video_id": "dQw4w9WgXcQ",
        "title": "Beispielvideo",
        "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "published": "2026-10-01T12:00:00Z"
    })
}

static TRIGGERS: &[TriggerSpec] = &[
    TriggerSpec {
        id: "manual",
        title: "Von Hand starten",
        fields: &[],
        provides: &[],
        sample: sample_empty,
    },
    TriggerSpec {
        id: "agent",
        title: "Durch einen Agenten starten",
        fields: &[],
        provides: &[],
        sample: sample_empty,
    },
    TriggerSpec {
        id: "schedule",
        title: "Zeitplan",
        fields: &[
            field("every", FieldKind::Choice(&["daily", "weekly"]), true),
            field("at", FieldKind::Text, true),
            field("weekdays", FieldKind::Any, false),
        ],
        provides: &["scheduled_for"],
        sample: sample_schedule,
    },
    TriggerSpec {
        id: "calendar.event_starting",
        title: "Termin beginnt",
        fields: &[
            field("integration", FieldKind::Id, true),
            field("lead_min", FieldKind::Int { min: 0, max: 120 }, false),
            field("only_meetings", FieldKind::Bool, false),
            field("title_contains", FieldKind::Text, false),
            field("min_attendees", FieldKind::Int { min: 1, max: 500 }, false),
        ],
        provides: &[
            "calendar",
            "event_id",
            "title",
            "start",
            "end",
            "attendees",
            "external_attendees",
            "online",
            "meeting",
        ],
        sample: sample_calendar,
    },
    TriggerSpec {
        id: "calendar.event_ended",
        title: "Termin endet",
        fields: &[
            field("integration", FieldKind::Id, true),
            field("only_meetings", FieldKind::Bool, false),
            field("title_contains", FieldKind::Text, false),
            field("min_attendees", FieldKind::Int { min: 1, max: 500 }, false),
        ],
        provides: &[
            "calendar",
            "event_id",
            "title",
            "start",
            "end",
            "attendees",
            "external_attendees",
            "online",
            "meeting",
        ],
        sample: sample_calendar,
    },
    TriggerSpec {
        id: "meeting.finished",
        title: "Besprechung fertig",
        fields: &[field(
            "stage",
            FieldKind::Choice(&["recording", "transcript", "notes", "minutes"]),
            true,
        )],
        provides: &["meeting_id", "title", "stage", "meeting"],
        sample: sample_meeting,
    },
    TriggerSpec {
        id: "folder.file_added",
        title: "Datei im Ordner",
        fields: &[
            field("integration", FieldKind::Id, true),
            field("subfolder", FieldKind::Text, false),
            field("extensions", FieldKind::TextList, false),
            field("stable_seconds", FieldKind::Int { min: 2, max: 300 }, false),
        ],
        provides: &["integration", "path", "name", "extension", "size"],
        sample: sample_folder,
    },
    TriggerSpec {
        id: "youtube.channel_new_video",
        title: "Neues Video im Kanal",
        fields: &[
            field("integration", FieldKind::Id, false),
            field("channel_id", FieldKind::Text, true),
            field("poll_minutes", FieldKind::Int { min: 15, max: 1440 }, false),
            // Beim ersten Abruf die neuesten n Videos verarbeiten (Vorgabe 0: alle vorhandenen
            // gelten als bekannt, es startet kein Lauf).
            field("backfill", FieldKind::Int { min: 0, max: 15 }, false),
        ],
        provides: &[
            "channel_id",
            "channel_title",
            "video_id",
            "title",
            "url",
            "published",
        ],
        sample: sample_youtube,
    },
];

pub fn triggers() -> &'static [TriggerSpec] {
    TRIGGERS
}

pub fn trigger_spec(id: &str) -> Option<&'static TriggerSpec> {
    TRIGGERS.iter().find(|t| t.id == id)
}

// ---------------------------------------------------------------------------
// Bausteine
// ---------------------------------------------------------------------------

/// Welches Recht ein Baustein braucht und woher seine Parameter kommen.
#[derive(Clone, Copy, Debug)]
pub enum NeedsSpec {
    None,
    Cap {
        capability: Capability,
        /// Parameter mit der Kennung der Integration.
        via: &'static str,
        /// Parameter, der als Ziel im Audit und in der Freigabe steht.
        target: Option<&'static str>,
    },
    /// Das Register kennt fuer diese Wirkung noch kein Recht: abgelehnt (fail closed).
    Unmodeled(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct ActionSpec {
    pub id: &'static str,
    pub title: &'static str,
    /// Geplante Wirkung; `{{p.<parameter>}}` setzt die (eingesetzten) Parameter ein.
    pub effect_text: &'static str,
    pub fields: &'static [FieldSpec],
    pub effect: EffectKind,
    pub heavy: Option<HeavyNeed>,
    pub needs: NeedsSpec,
}

const STT: HeavyNeed = HeavyNeed {
    ram_mb: 3_072,
    label: "Transkription",
};
const LLM: HeavyNeed = HeavyNeed {
    ram_mb: 6_144,
    label: "Sprachmodell",
};
const TTS: HeavyNeed = HeavyNeed {
    ram_mb: 2_048,
    label: "Sprachausgabe",
};

static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        id: "recording.start",
        title: "Aufnahme starten",
        effect_text: "Aufnahme starten, erst nach Bestätigung der Einwilligung im Hinweisfenster",
        fields: &[
            field("via", FieldKind::Id, true),
            field(
                "stop",
                FieldKind::Choice(&["event_end", "manual", "duration"]),
                false,
            ),
            field("max_minutes", FieldKind::Int { min: 1, max: 720 }, false),
            // Titel der Besprechung; Vorgabe: der Titel des Ausloesers (Termin, Besprechung).
            field("title", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::RecordingStart,
            via: "via",
            target: Some("title"),
        },
    },
    ActionSpec {
        id: "recording.stop",
        title: "Aufnahme beenden",
        effect_text: "Laufende Aufnahme beenden",
        fields: &[],
        effect: EffectKind::Idempotent,
        heavy: None,
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "meeting.import",
        title: "Datei importieren und transkribieren",
        effect_text: "Datei {{p.path}} aus {{p.via}} importieren und transkribieren",
        fields: &[
            field("via", FieldKind::Id, true),
            field("path", FieldKind::Text, true),
            field("title", FieldKind::Text, false),
            // Kennung des Projekts (Ordner), in das die Besprechung kommt.
            field("project", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: Some(STT),
        needs: NeedsSpec::Cap {
            capability: Capability::FilesRead,
            via: "via",
            target: Some("path"),
        },
    },
    ActionSpec {
        id: "meeting.notes",
        title: "KI-Notizen erzeugen",
        effect_text: "KI-Notizen zur Besprechung erzeugen",
        fields: &[
            // Kennung oder Titel einer Vorlage, `auto` (nach Inhalt) oder leer (Vorgabe).
            field("template", FieldKind::Text, false),
            // G5: Fassung des Transkripts und Sprache des Dokuments.
            field("variant_id", FieldKind::Text, false),
            field("output_language", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "meeting.minutes",
        title: "Protokoll erzeugen",
        effect_text: "Protokoll zur Besprechung erzeugen",
        fields: &[
            field("template", FieldKind::Text, false),
            field("variant_id", FieldKind::Text, false),
            field("output_language", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "text.summarize",
        title: "Zusammenfassen",
        effect_text: "Zusammenfassung erzeugen (Quelle: {{p.source}})",
        fields: &[
            // `minutes`, `notes`, `transcript` (der Besprechung des Laufs) oder ein Text.
            field("source", FieldKind::Text, true),
            // `kurz`, `mittel`, `lang` oder `management`.
            field("style", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "agent.extract",
        title: "Aufgaben, Fristen und Entscheidungen extrahieren",
        effect_text: "Aus dem Transkript To-dos, Fristen und Entscheidungen mit dem lokalen Sprachmodell ziehen (mit Belegen, ohne Aussenwirkung)",
        fields: &[
            // Welche Listen: `todos`, `deadlines`, `decisions` (ohne Angabe alle drei).
            field("kinds", FieldKind::TextList, false),
        ],
        // Liest nur: das Ergebnis steht im Laufprotokoll, beliebig wiederholbar.
        effect: EffectKind::Pure,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "agent.route",
        title: "Werkzeug wählen (lokaler Agent)",
        effect_text: "Das lokale Sprachmodell wählt aus den freigegebenen Werkzeugen höchstens eines; es führt nichts selbst aus, die folgenden Schritte wirken nur nach ihrer Bedingung und mit ihren Rechten",
        fields: &[
            // Alles Folgende bis auf `context` und `reference_date` sind FESTE Werte: Daten (Transkript,
            // Mail, Modellausgabe) bestimmen weder die Aufgabe noch die Werkzeuge, die Empfaengerregel,
            // die Obergrenze oder das Modell.
            literal("task", FieldKind::Text, true),
            // Daten fuer die Wahl (nicht vertrauenswuerdig, im Prompt als Daten markiert).
            field("context", FieldKind::Text, false),
            // Namen aus `agent::policy::catalog()`: `notify_local`, `send_mail`, `calendar_note`.
            literal("tools", FieldKind::TextList, true),
            // Empfaengerregel wie bei `mail.send`; Pflicht mit dem Werkzeug `send_mail`. Die Empfaenger
            // bildet der Code, das Modell nennt keine.
            literal(
                "recipients",
                FieldKind::Choice(&["me", "participants", "all", "internal", "list"]),
                false,
            ),
            literal("list", FieldKind::TextList, false),
            // Hoechstens so viele Werkzeugwahlen je Lauf (alle `agent.route`-Schritte zusammen); die
            // harte Grenze liegt bei 10.
            literal("max_actions", FieldKind::Int { min: 1, max: 10 }, false),
            // Router-Modell; ohne Angabe `llm-qwen3.5-9b-q4`. Nur ein geladenes Modell.
            literal("model", FieldKind::Text, false),
            // Bezug fuer relative Zeitangaben (ISO-Datum oder -Zeitpunkt); ohne Angabe heute.
            field("reference_date", FieldKind::Text, false),
        ],
        // Rechnet nur (Modellaufruf) und schreibt nichts ausser der Provenienz: beliebig wiederholbar.
        effect: EffectKind::Pure,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "export.document",
        title: "Dokument ablegen",
        effect_text: "Dokument als {{p.format}} in {{p.target}} ablegen",
        fields: &[
            field("format", FieldKind::Choice(&["docx", "pdf", "md"]), true),
            field("target", FieldKind::Id, true),
            field("name", FieldKind::Text, false),
            field("subfolder", FieldKind::Text, false),
            // Was abgelegt wird; ohne Angabe das Protokoll.
            field(
                "content",
                FieldKind::Choice(&["minutes", "notes", "all"]),
                false,
            ),
        ],
        effect: EffectKind::Idempotent,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::FilesWrite,
            via: "target",
            target: Some("name"),
        },
    },
    ActionSpec {
        id: "mail.send",
        title: "Mail senden",
        effect_text: "Mail über {{p.via}} an die Empfängerregel „{{p.to}}“ senden, Betreff „{{p.subject}}“",
        fields: &[
            // Kanal und Empfaengerregel sind feste Werte: Daten (Termin, Modellausgabe) bestimmen
            // nie, ueber welches Konto und an wen gesendet wird. Die Empfaenger bildet der
            // Baustein aus der Regel: `me` (nur ich), `participants` (die anderen Teilnehmenden
            // des Termins), `all` (Teilnehmende und ich), `internal` (Teilnehmende mit der
            // Domaene der eigenen Adressen), `list` (die feste Liste in `list`).
            literal("via", FieldKind::Id, true),
            literal(
                "to",
                FieldKind::Choice(&["me", "participants", "all", "internal", "list"]),
                true,
            ),
            literal("list", FieldKind::TextList, false),
            field("subject", FieldKind::Text, true),
            field("body", FieldKind::Text, false),
            // Pfad oder Liste von Pfaden; nur Dateien in den Ordner-Integrationen des Nutzers.
            field("attach", FieldKind::Any, false),
            // Entwuerfe anlegen ist noch nicht moeglich (Scope `Mail.ReadWrite`); `true` wird
            // beim Speichern abgelehnt. Die Freigabe mit Vorschau ersetzt den Entwurf.
            field("draft", FieldKind::Bool, false),
            // `true`: auch an andere als mich ohne Freigabe senden, wenn das Recht „erlaubt“
            // ist (E3: je Ablauf aenderbar). Ohne diese Angabe verlangt jede Mail an Dritte
            // eine Freigabe, auch bei „erlaubt“.
            literal("auto", FieldKind::Bool, false),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::MailSend,
            via: "via",
            target: Some("to"),
        },
    },
    ActionSpec {
        id: "calendar.note",
        title: "Notiz in den Termin schreiben",
        effect_text: "Notiz über {{p.via}} in den Termin schreiben",
        fields: &[
            literal("via", FieldKind::Id, true),
            // Kennung des Termins (`trigger.event_id`); ohne Angabe der Termin des Ausloesers.
            field("event", FieldKind::Text, false),
            field("text", FieldKind::Text, true),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::CalendarWrite,
            via: "via",
            target: Some("event"),
        },
    },
    ActionSpec {
        id: "obsidian.note",
        title: "Obsidian-Notiz schreiben",
        effect_text: "Notiz {{p.path}} in {{p.via}} schreiben",
        fields: &[
            field("via", FieldKind::Id, true),
            field("path", FieldKind::Text, true),
            field("content", FieldKind::Text, true),
            field("mode", FieldKind::Choice(&["create", "append"]), false),
            field("section", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::VaultWrite,
            via: "via",
            target: Some("path"),
        },
    },
    ActionSpec {
        id: "knowledge.search",
        title: "Wissen durchsuchen",
        effect_text: "Wissensbasis {{p.via}} nach „{{p.query}}“ durchsuchen",
        fields: &[
            field("via", FieldKind::Id, true),
            field("query", FieldKind::Text, true),
            field("limit", FieldKind::Int { min: 1, max: 50 }, false),
        ],
        effect: EffectKind::Pure,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::KnowledgeSearch,
            via: "via",
            target: Some("query"),
        },
    },
    ActionSpec {
        id: "knowledge.rate",
        title: "Relevanz bewerten",
        effect_text: "Relevanz des Inhalts nach dem Themenprofil bewerten",
        fields: &[
            field("source", FieldKind::Text, true),
            field("profile", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(LLM),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "knowledge.reconcile",
        title: "Mit der Wissensbasis abgleichen",
        effect_text: "Aussagen mit {{p.via}} abgleichen (neu / vorhanden / ergänzt / widerspricht) und Ergänzungen schreiben",
        fields: &[
            field("via", FieldKind::Id, true),
            field("source", FieldKind::Text, true),
            field("folder", FieldKind::Text, false),
            field("video_id", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: Some(LLM),
        needs: NeedsSpec::Cap {
            capability: Capability::VaultWrite,
            via: "via",
            target: Some("folder"),
        },
    },
    ActionSpec {
        id: "channel.report",
        title: "Kanal-Management-Summary",
        effect_text: "Management-Summary zum Kanal {{p.channel}} in {{p.via}} ablegen",
        fields: &[
            field("via", FieldKind::Id, true),
            field("channel", FieldKind::Text, true),
            field("source", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: Some(LLM),
        needs: NeedsSpec::Cap {
            capability: Capability::VaultWrite,
            via: "via",
            target: Some("channel"),
        },
    },
    ActionSpec {
        id: "youtube.add_source",
        title: "YouTube-Quelle anlegen",
        effect_text: "YouTube-Video {{p.url}} als Quelle über {{p.via}} anlegen",
        fields: &[
            field("via", FieldKind::Id, true),
            field("url", FieldKind::Text, true),
            // Kennung des Projekts (Ordner), in das die Besprechung kommt.
            field("project", FieldKind::Text, false),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::YoutubeAdd,
            via: "via",
            target: Some("url"),
        },
    },
    ActionSpec {
        id: "youtube.transcript",
        title: "YouTube-Transkript holen",
        effect_text: "Transkript des Videos {{p.video}} über {{p.via}} holen",
        fields: &[
            field("via", FieldKind::Id, true),
            field("video", FieldKind::Text, true),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(STT),
        needs: NeedsSpec::Cap {
            capability: Capability::MediaFetch,
            via: "via",
            target: Some("video"),
        },
    },
    ActionSpec {
        id: "notify.local",
        title: "Windows-Mitteilung",
        effect_text: "Mitteilung „{{p.title}}“ anzeigen",
        fields: &[
            field("title", FieldKind::Text, true),
            field("body", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: None,
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "tts.render",
        title: "Vorlesen",
        effect_text: "Text als Audiodatei sprechen lassen",
        fields: &[
            field("text", FieldKind::Text, true),
            field("voice", FieldKind::Text, false),
        ],
        effect: EffectKind::Idempotent,
        heavy: Some(TTS),
        needs: NeedsSpec::None,
    },
    ActionSpec {
        id: "webhook.post",
        title: "Webhook senden",
        effect_text: "Daten an den Webhook „{{p.via}}“ senden",
        fields: &[
            // Das Ziel ist eine Webhook-Integration des Registers (Art `webhook`); ihre Adresse
            // steht im Geheimnisspeicher, nie im Ablauf. Fest: Daten waehlen kein Ziel.
            literal("via", FieldKind::Id, true),
            field("body", FieldKind::Any, false),
        ],
        effect: EffectKind::External,
        heavy: None,
        needs: NeedsSpec::Cap {
            capability: Capability::WebhookPost,
            via: "via",
            target: Some("via"),
        },
    },
    ActionSpec {
        id: "wait",
        title: "Warten",
        effect_text: "{{p.minutes}} Minuten warten",
        fields: &[field("minutes", FieldKind::Int { min: 1, max: 10_080 }, true)],
        effect: EffectKind::Pure,
        heavy: None,
        needs: NeedsSpec::None,
    },
];

pub fn actions() -> &'static [ActionSpec] {
    ACTIONS
}

pub fn action_spec(id: &str) -> Option<&'static ActionSpec> {
    ACTIONS.iter().find(|a| a.id == id)
}

// ---------------------------------------------------------------------------
// Pruefung eines Feldwerts gegen seine Art
// ---------------------------------------------------------------------------

/// Enthaelt der Wert eine `{{...}}`-Stelle (er wird erst beim Lauf bestimmt)?
pub fn is_dynamic(v: &Value) -> bool {
    matches!(v, Value::String(s) if s.contains("{{"))
}

/// Prueft einen Wert gegen die Art des Felds. `allow_templates`: Werte mit
/// `{{...}}` gelten als dynamisch und werden erst beim Lauf geprueft.
pub fn check_field(spec: &FieldSpec, v: &Value, allow_templates: bool) -> Result<(), String> {
    if spec.literal && contains_template(v) {
        return Err("hier sind nur feste Werte erlaubt, keine {{…}}-Einsetzungen".to_string());
    }
    if allow_templates && is_dynamic(v) {
        return Ok(());
    }
    match spec.kind {
        FieldKind::Text => match v {
            Value::String(s) if s.chars().count() <= 4_000 => Ok(()),
            Value::String(_) => Err("Text zu lang (höchstens 4000 Zeichen)".to_string()),
            _ => Err("Text erwartet".to_string()),
        },
        FieldKind::Id => match v {
            Value::String(s) if valid_id(s) => Ok(()),
            Value::String(_) => Err(
                "Kennung erwartet (Buchstaben, Ziffern, - und _, höchstens 40 Zeichen)".to_string(),
            ),
            _ => Err("Kennung (Text) erwartet".to_string()),
        },
        FieldKind::Int { min, max } => match v.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(()),
            Some(_) => Err(format!("Zahl zwischen {min} und {max} erwartet")),
            None => Err("ganze Zahl erwartet".to_string()),
        },
        FieldKind::Bool => match v {
            Value::Bool(_) => Ok(()),
            _ => Err("Ja/Nein erwartet".to_string()),
        },
        FieldKind::TextList => match v {
            Value::Array(items) if items.len() <= 100 && items.iter().all(Value::is_string) => {
                Ok(())
            }
            Value::Array(items) if items.len() > 100 => Err("höchstens 100 Einträge".to_string()),
            _ => Err("Liste aus Texten erwartet".to_string()),
        },
        FieldKind::Choice(options) => match v {
            Value::String(s) if options.contains(&s.as_str()) => Ok(()),
            _ => Err(format!("eines von: {}", options.join(", "))),
        },
        FieldKind::Any => Ok(()),
    }
}

fn contains_template(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains("{{"),
        Value::Array(a) => a.iter().any(contains_template),
        Value::Object(m) => m.values().any(contains_template),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Katalogbaustein: plant und prueft, laeuft noch nicht
// ---------------------------------------------------------------------------

/// Ein Baustein, der nur aus seinem Katalogeintrag besteht. Er plant (Trockenlauf)
/// und nennt sein Recht, `run` meldet aber "noch nicht eingebaut".
pub struct SpecAction {
    spec: &'static ActionSpec,
}

impl SpecAction {
    pub fn new(spec: &'static ActionSpec) -> Self {
        Self { spec }
    }
}

fn param_text(params: &Value, name: &str) -> Option<String> {
    match params.get(name)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|i| i.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    }
}

/// Das Recht eines Katalogeintrags aus den (eingesetzten) Parametern.
pub fn needs_from_spec(spec: &ActionSpec, params: &Value) -> Result<Option<Needs>, NeedsError> {
    match spec.needs {
        NeedsSpec::None => Ok(None),
        NeedsSpec::Unmodeled(msg) => Err(NeedsError::Unmodeled(msg.to_string())),
        NeedsSpec::Cap {
            capability,
            via,
            target,
        } => {
            let id = param_text(params, via).ok_or_else(|| {
                NeedsError::Invalid(format!("Parameter „{via}“ (Kennung der Integration) fehlt"))
            })?;
            if !valid_id(&id) {
                return Err(NeedsError::Invalid(format!(
                    "Parameter „{via}“ ist keine gültige Integrations-Kennung"
                )));
            }
            Ok(Some(Needs {
                integration_id: id,
                capability,
                target: target.and_then(|t| param_text(params, t)),
            }))
        }
    }
}

/// Die geplante Wirkung eines Katalogeintrags (Anzeige, kein Ausfuehren).
pub fn describe_from_spec(spec: &ActionSpec, params: &Value) -> String {
    let ctx = json!({ "p": params });
    match expr::parse_template(spec.effect_text) {
        Ok(t) => t.render_display(&ctx),
        Err(_) => spec.title.to_string(),
    }
}

impl Action for SpecAction {
    fn id(&self) -> &str {
        self.spec.id
    }

    fn effect(&self) -> EffectKind {
        self.spec.effect
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        self.spec.heavy
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        describe_from_spec(self.spec, params)
    }

    fn run(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Result<StepOutput, StepError> {
        Err(StepError::NotAvailable(format!(
            "Der Baustein „{}“ ist in dieser Version noch nicht eingebaut.",
            self.spec.title
        )))
    }
}
