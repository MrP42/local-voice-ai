//! C3 (Goal Lokaler Agent, AK5): die Politik des Agenten -- im Code, nicht im Prompt.
//!
//! `agent.route` laesst ein kleines Modell aus einer kurzen Liste EIN Werkzeug waehlen. Was
//! das Modell antwortet, ist Eingabe, nie Berechtigung. Dieses Modul ist die einzige Stelle,
//! die aus einer Wahl des Modells ein [`Verdict`] macht; es ist rein (kein I/O, keine Uhr,
//! keine Datenbank, kein Modell) und deshalb vollstaendig testbar. Das Muster ist der
//! *Action-Selector*: das Modell waehlt aus einer geschlossenen, vom Ablauf vorgegebenen
//! Menge und fuellt nur INHALT (Betreff, Text, Zeitangabe); Ziel, Empfaenger, Konto, Pfad und
//! Recht bestimmt der Code bzw. der Ablauf. Ausgefuehrt wird nie hier: der Schritt hinter
//! `agent.route` wirkt, mit seinem eigenen Recht ueber das Tor (Freigabe, Audit).
//!
//! # Was die Politik durchsetzt
//!
//! 1. **Whitelist.** Nur Werkzeuge aus dem Katalog dieses Moduls ([`catalog`]) UND aus der
//!    Liste des Schritts ([`Whitelist`]) kommen durch; der Vergleich ist genau (keine
//!    Gross-/Kleinschreibung, kein Leerraum, keine unsichtbaren Zeichen), jede andere
//!    Schreibweise ist "nicht freigegeben". Unbekannte Namen in der Liste des Ablaufs sind
//!    ein Konfigurationsfehler, nie ein stilles Weglassen.
//! 2. **Empfaenger.** Das Modell bestimmt nie, an wen etwas geht: `send_mail` kennt kein
//!    Empfaengerfeld, die Menge bildet der Code (B5, `recipients::resolve`) und wird hier nur
//!    als Liste uebergeben. Nennt das Modell trotzdem Empfaenger (`to`, `cc`, `bcc`, ...), muss
//!    jede Adresse in der Menge des Codes stehen -- sonst wird die GANZE Wahl verworfen, nichts
//!    wird gekuerzt oder "korrigiert". Was in der Menge steht, wird ignoriert: die Mail geht an
//!    die Menge, nicht an eine Auswahl des Modells.
//! 3. **Argumente.** Nur die Felder des Werkzeugs, nur Text, Steuer- und unsichtbare Zeichen
//!    entfernt, Laenge gedeckelt, Betreff einzeilig (kein Kopfzeilen-Einschleusen). Ein
//!    unbekanntes Feld ist ein Verstoss (das Schema des Servers erlaubt keines; wer eines
//!    liefert, ignoriert das Schema).
//! 4. **Datum.** Das Modell nennt eine Zeitangabe so, wie sie gesagt wurde; das Datum rechnet
//!    `dates::resolve` mit dem Bezugsdatum und `dates::validate` prueft es (nicht in der
//!    Vergangenheit, hoechstens zwei Jahre voraus). Ein vom Modell genanntes ISO-Datum gilt
//!    nur durch denselben Weg.
//! 5. **Obergrenze.** Je Lauf hoechstens `max_actions` (1..=[`HARD_MAX_ACTIONS`]) Wahlen eines
//!    Werkzeugs ueber alle `agent.route`-Schritte; die harte Grenze gilt auch bei einer
//!    groesseren Angabe im Ablauf ([`actions_used`] zaehlt aus dem Laufkontext).
//! 6. **Einschleusen.** Ein Kontext mit Aufforderungen an die KI ("Ignoriere alle Regeln ...",
//!    "SYSTEM: ...", Werkzeugbefehle) fuehrt zu `no_action` OHNE Modellaufruf
//!    ([`injection_signals`]); dieselben Muster in den Argumenten des Modells verwerfen die
//!    Wahl. Das ist eine Heuristik als zusaetzliche Schicht, nie der einzige Schutz: ein
//!    umformulierter Befehl wird nicht erkannt, scheitert dann aber an 1 bis 5 und am Tor.
//!
//! # Fehlerfaelle und ihre Absicherung
//!
//! | Fall | Verhalten | Test |
//! |------|-----------|------|
//! | Werkzeug ausserhalb der Liste (auch andere Schreibweise) | `ToolNotAllowed`, nie ausgefuehrt | `a_tool_outside_the_whitelist_*`, `the_name_is_matched_exactly_*` |
//! | Empfaenger nicht in der Menge des Codes | `RecipientNotAllowed`, ganze Wahl verworfen | `foreign_recipients_*`, `every_recipient_key_*` |
//! | Empfaenger bei einem Werkzeug ohne Empfaenger | `RecipientNotAllowed` | `recipients_on_a_tool_without_recipients_*` |
//! | Obergrenze erreicht | `LimitReached` (Modell wird nicht gefragt) | `the_limit_*`, `actions_used_*` |
//! | unbekanntes/fehlendes/falsch getyptes Argument | `UnexpectedArgument`/`MissingArgument`/`BadArguments` | `arguments_*` |
//! | Datum nicht aufloesbar / vergangen | Pflicht: `DateInvalid`; optional: verworfen mit Vermerk | `dates_*` |
//! | Steuerzeichen, Zeilenumbruch im Betreff, Riesentext | bereinigt/gedeckelt, kein Fehler | `text_*` |
//! | Einschleusen im Kontext oder in der Antwort | `InjectionSuspected` | `injection_*` |
//! | feindliche Antworten (Muell, Riesenobjekte, Typen) | nie Panik, nie `Run` mit fremden Feldern | `hostile_answers_*` |
//!
//! Der Echtzeit-Audiopfad ist nicht beteiligt (keine Allokation-Vorgaben hier: das Modul laeuft
//! nur auf dem Arbeiter-Thread der Engine).

use std::collections::HashSet;

use chrono::NaiveDate;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Map, Value};

use super::dates;
use super::schema::{ToolChoice, ToolSpec, NO_ACTION};

/// Vorgabe fuer `max_actions`, wenn der Ablauf nichts nennt.
pub const DEFAULT_MAX_ACTIONS: u32 = 3;
/// Harte Obergrenze je Lauf, unabhaengig von der Angabe im Ablauf.
pub const HARD_MAX_ACTIONS: u32 = 10;
/// So viele Werkzeuge hoechstens in der Liste eines Schritts (kurze Liste = bessere Wahl).
pub const MAX_TOOLS: usize = 6;
/// Laengster Grund bei `no_action` (Zeichen).
pub const REASON_CHARS: usize = 200;
/// Laengster Name eines Werkzeugs/Arguments, der in Protokolle gelangt.
const NAME_CHARS: usize = 40;
/// Laengste Zeitangabe (Zeichen).
const PHRASE_CHARS: usize = 80;

// -- Katalog der Werkzeuge ---------------------------------------------------------------------

/// Wie ein Argument des Modells behandelt wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    /// Text, hoechstens `max` Zeichen; `multiline`: Zeilenumbrueche bleiben erhalten.
    Text { max: usize, multiline: bool },
    /// Zeitangabe wie gesagt; der Code rechnet das ISO-Datum und legt es unter `out` ab.
    Date { out: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub struct ParamDef {
    /// Schluessel, wie das Modell ihn liefert.
    pub key: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    pub description: &'static str,
}

/// Ein Werkzeug, das ein Schritt anbieten darf. Die Menge ist im Code geschlossen: ein Ablauf
/// waehlt daraus, er erfindet keine.
#[derive(Clone, Copy, Debug)]
pub struct ToolDef {
    pub name: &'static str,
    /// Der Baustein des Ablaufs, der das Werkzeug ausfuehrt (mit eigenem Recht am Tor).
    pub action: &'static str,
    pub description: &'static str,
    /// Geht eine Mail an andere? Dann bildet der Code die Empfaengermenge.
    pub sends_mail: bool,
    pub params: &'static [ParamDef],
}

static TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "notify_local",
        action: "notify.local",
        description: "Zeigt eine Mitteilung auf diesem Rechner (Erinnerung, Hinweis zu einer Frist).",
        sends_mail: false,
        params: &[
            ParamDef {
                key: "title",
                kind: ParamKind::Text {
                    max: 80,
                    multiline: false,
                },
                required: true,
                description: "kurzer Titel der Mitteilung",
            },
            ParamDef {
                key: "body",
                kind: ParamKind::Text {
                    max: 240,
                    multiline: false,
                },
                required: false,
                description: "Text der Mitteilung",
            },
            ParamDef {
                key: "due_phrase",
                kind: ParamKind::Date { out: "due_date" },
                required: false,
                description: "Frist, falls eine genannt ist, genau so wie gesagt (z. B. \"übermorgen\", \"Freitag nächster Woche\", \"15.10.\"); nicht umrechnen",
            },
        ],
    },
    ToolDef {
        name: "send_mail",
        action: "mail.send",
        description: "Sendet eine Mail an die vom Programm festgelegten Empfänger. Keine Adressen nennen.",
        sends_mail: true,
        params: &[
            ParamDef {
                key: "subject",
                kind: ParamKind::Text {
                    max: 150,
                    multiline: false,
                },
                required: true,
                description: "Betreff",
            },
            ParamDef {
                key: "body",
                kind: ParamKind::Text {
                    max: 4_000,
                    multiline: true,
                },
                required: false,
                description: "Text der Mail",
            },
        ],
    },
    ToolDef {
        name: "calendar_note",
        action: "calendar.note",
        description: "Schreibt eine kurze Notiz in den Termin des Auslösers.",
        sends_mail: false,
        params: &[ParamDef {
            key: "text",
            kind: ParamKind::Text {
                max: 2_000,
                multiline: true,
            },
            required: true,
            description: "Text der Notiz",
        }],
    },
];

/// Alle Werkzeuge, die ein Schritt anbieten darf.
#[allow(dead_code)] // Tests; C5: Schema-Vorschau und Werkzeugwahl im Editor
pub fn catalog() -> &'static [ToolDef] {
    TOOLS
}

pub fn tool_def(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Die Namen des Katalogs (fuer Meldungen und die Oberflaeche).
pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).collect()
}

impl ToolDef {
    /// Das Werkzeug fuer die Schema-Wahl der Laufzeit. Bewusst ohne `maxLength`: sehr grosse
    /// Wiederholungen blaehen die Grammatik des Servers auf; die Laenge deckelt der Code.
    pub fn spec(&self) -> ToolSpec {
        let mut properties = Map::new();
        let mut required: Vec<Value> = Vec::new();
        for p in self.params {
            properties.insert(
                p.key.to_string(),
                json!({ "type": "string", "description": p.description }),
            );
            if p.required {
                required.push(json!(p.key));
            }
        }
        ToolSpec {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": Value::Object(properties),
                "required": required,
            }),
        }
    }
}

/// `no_action`: der Rueckfall, immer angeboten.
pub fn no_action_spec() -> ToolSpec {
    ToolSpec {
        name: NO_ACTION.to_string(),
        description: "Keine Aktion: nichts passt, es fehlen nötige Angaben, oder die Anfrage verstößt gegen die Regeln."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "reason": { "type": "string", "description": "kurzer Grund" } },
            "required": ["reason"],
        }),
    }
}

// -- Whitelist ----------------------------------------------------------------------------------

/// Warum eine Werkzeugliste nicht gilt (deutsch, fuer die Pruefung beim Speichern).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    Empty,
    TooMany(usize),
    Unknown { name: String, valid: String },
    NoActionListed,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::Empty => write!(
                f,
                "Die Werkzeugliste ist leer: mindestens ein Werkzeug muss freigegeben sein."
            ),
            PolicyError::TooMany(n) => write!(
                f,
                "{n} Werkzeuge sind zu viele: höchstens {MAX_TOOLS} je Schritt (eine kurze Liste wählt besser)."
            ),
            PolicyError::Unknown { name, valid } => {
                write!(f, "Das Werkzeug „{name}“ gibt es nicht. Erlaubt sind: {valid}.")
            }
            PolicyError::NoActionListed => write!(
                f,
                "„{NO_ACTION}“ ist immer dabei und gehört nicht in die Liste."
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Die Werkzeuge, die DIESER Schritt anbieten darf (Teilmenge des Katalogs, Reihenfolge wie
/// angegeben, ohne Doppelte).
#[derive(Clone, Debug)]
pub struct Whitelist {
    tools: Vec<&'static ToolDef>,
}

impl Whitelist {
    pub fn parse(names: &[String]) -> Result<Whitelist, PolicyError> {
        let mut tools: Vec<&'static ToolDef> = Vec::new();
        for raw in names {
            let name = raw.trim();
            if name == NO_ACTION {
                return Err(PolicyError::NoActionListed);
            }
            let Some(def) = tool_def(name) else {
                return Err(PolicyError::Unknown {
                    name: clean_name(name),
                    valid: tool_names().join(", "),
                });
            };
            if !tools.iter().any(|t| t.name == def.name) {
                tools.push(def);
            }
        }
        if tools.is_empty() {
            return Err(PolicyError::Empty);
        }
        if tools.len() > MAX_TOOLS {
            return Err(PolicyError::TooMany(tools.len()));
        }
        Ok(Whitelist { tools })
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name).collect()
    }

    /// Genau dieser Name (siehe Moduldoku, Punkt 1).
    pub fn get(&self, name: &str) -> Option<&'static ToolDef> {
        self.tools.iter().copied().find(|t| t.name == name)
    }

    #[allow(dead_code)] // Tests; C5 (Editor)
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn has_mail(&self) -> bool {
        self.tools.iter().any(|t| t.sends_mail)
    }

    /// Ohne die Werkzeuge, die Mail senden (wenn es keine Empfaenger gibt).
    pub fn without_mail(&self) -> Whitelist {
        Whitelist {
            tools: self
                .tools
                .iter()
                .copied()
                .filter(|t| !t.sends_mail)
                .collect(),
        }
    }

    /// Die Werkzeuge fuer die Schema-Wahl, `no_action` zuletzt.
    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut out: Vec<ToolSpec> = self.tools.iter().map(|t| t.spec()).collect();
        out.push(no_action_spec());
        out
    }
}

// -- Das Urteil -----------------------------------------------------------------------------------

/// Warum die Politik eine Wahl des Modells verwirft. `as_str` steht als `reason` im Ergebnis des
/// Schritts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalCode {
    ToolNotAllowed,
    UnexpectedArgument,
    MissingArgument,
    BadArguments,
    RecipientNotAllowed,
    NoRecipients,
    DateInvalid,
    LimitReached,
    InjectionSuspected,
}

impl RefusalCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::ToolNotAllowed => "tool_not_allowed",
            RefusalCode::UnexpectedArgument => "unexpected_argument",
            RefusalCode::MissingArgument => "missing_argument",
            RefusalCode::BadArguments => "bad_arguments",
            RefusalCode::RecipientNotAllowed => "recipient_not_allowed",
            RefusalCode::NoRecipients => "no_recipients",
            RefusalCode::DateInvalid => "date_invalid",
            RefusalCode::LimitReached => "limit_reached",
            RefusalCode::InjectionSuspected => "injection_suspected",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    /// Ein deutscher Satz fuers Laufprotokoll, ohne Text aus Kontext oder Antwort.
    pub detail: String,
    /// Der Name, den das Modell nannte (bereinigt, gekuerzt): nur zur Anzeige.
    pub requested_tool: Option<String>,
}

/// Ein geprueftes Werkzeug mit bereinigten Argumenten: DAS und nichts anderes darf ein
/// folgender Schritt verwenden.
#[derive(Clone, Debug, PartialEq)]
pub struct Approved {
    pub tool: &'static str,
    /// Der Baustein des Ablaufs, der es ausfuehrt.
    pub action: &'static str,
    /// Alle Felder des Werkzeugs (fehlende optionale als leerer Text), dazu bei Zeitangaben das
    /// vom Code gerechnete Datum. Nichts davon bestimmt Ziel, Empfaenger oder Pfad.
    pub arguments: Map<String, Value>,
    /// Bei Mail-Werkzeugen die Empfaengermenge des Codes (eine Auswahl des Modells gibt es nicht).
    pub recipients: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Durch die Politik; der folgende Schritt fuehrt aus (mit seinem Recht am Tor).
    Run(Approved),
    /// Das Modell hat `no_action` gewaehlt (Grund bereinigt).
    NoAction { reason: String },
    /// Die Politik verwirft die Wahl: keine Aussenwirkung.
    Refused(Refusal),
}

impl Verdict {
    pub fn is_run(&self) -> bool {
        matches!(self, Verdict::Run(_))
    }
}

/// Was die Pruefung braucht.
pub struct VetInput<'a> {
    pub whitelist: &'a Whitelist,
    /// Die Empfaengermenge, die der Code gebildet hat (leer, wenn es keine gibt).
    pub recipients: &'a [String],
    /// Bezug fuer relative Zeitangaben ("heute" des Schritts oder Datum der Besprechung).
    pub reference: NaiveDate,
    /// Wahlen eines Werkzeugs in diesem Lauf bisher.
    pub used_actions: u32,
    pub max_actions: u32,
}

/// Wirksame Obergrenze: die Angabe des Ablaufs, nie unter 1 und nie ueber der harten Grenze.
pub fn effective_limit(max_actions: u32) -> u32 {
    max_actions.clamp(1, HARD_MAX_ACTIONS)
}

/// Wie viele Werkzeuge haben `agent.route`-Schritte in diesem Lauf schon gewaehlt? Gezaehlt
/// wird aus dem Laufkontext (`steps.<id>`), der aus dem Journal der Engine entsteht.
pub fn actions_used(context: &Value) -> u32 {
    context
        .get("steps")
        .and_then(Value::as_object)
        .map(|steps| {
            steps
                .values()
                .filter(|s| {
                    s.get("agent_route") == Some(&Value::Bool(true))
                        && s.get("outcome").and_then(Value::as_str) == Some("tool")
                })
                .count() as u32
        })
        .unwrap_or(0)
}

/// Pruefung der Wahl des Modells (siehe Moduldoku). Reihenfolge: `no_action` -> Whitelist ->
/// Obergrenze -> Empfaenger und Felder -> Datum -> Einschleusen.
pub fn vet(choice: &ToolChoice, input: &VetInput<'_>) -> Verdict {
    if choice.tool == NO_ACTION {
        let reason = choice
            .arguments
            .get("reason")
            .and_then(Value::as_str)
            .map(|r| sanitize_text(r, REASON_CHARS, false).0)
            .unwrap_or_default();
        return Verdict::NoAction { reason };
    }
    let requested = clean_name(&choice.tool);
    let refuse = |code: RefusalCode, detail: String| {
        Verdict::Refused(Refusal {
            code,
            detail,
            requested_tool: Some(requested.clone()),
        })
    };

    // 1. Whitelist.
    let Some(def) = input.whitelist.get(&choice.tool) else {
        return refuse(
            RefusalCode::ToolNotAllowed,
            format!("Das Werkzeug „{requested}“ ist in diesem Schritt nicht freigegeben."),
        );
    };

    // 5. Obergrenze (vor allem Weiteren: ein erreichtes Limit braucht keine Argumente).
    let limit = effective_limit(input.max_actions);
    if input.used_actions >= limit {
        return refuse(
            RefusalCode::LimitReached,
            format!("Die Obergrenze von {limit} Aktionen je Lauf ist erreicht."),
        );
    }

    if def.sends_mail && input.recipients.is_empty() {
        return refuse(
            RefusalCode::NoRecipients,
            "Für die Mail gibt es keine Empfänger: die Regel des Schritts ergibt keine Adresse."
                .to_string(),
        );
    }

    // 3. Argumente.
    let Some(args) = choice.arguments.as_object() else {
        return refuse(
            RefusalCode::BadArguments,
            "Die Argumente sind kein Objekt.".to_string(),
        );
    };
    let mut out = Map::new();
    let mut notes: Vec<String> = Vec::new();
    let mut seen: HashSet<&'static str> = HashSet::new();
    let mut texts: Vec<String> = Vec::new();

    for (key, value) in args {
        // 2. Empfaenger: jede Angabe des Modells muss in der Menge des Codes stehen.
        if is_recipient_key(key) {
            if !recipients_within(value, def, input.recipients) {
                return refuse(
                    RefusalCode::RecipientNotAllowed,
                    "Das Modell nannte einen Empfänger, der nicht zur Menge gehört, die das Programm festlegt."
                        .to_string(),
                );
            }
            if !is_blank(value) {
                notes.push(
                    "Empfängerangabe des Modells ignoriert: die Empfänger bestimmt das Programm."
                        .to_string(),
                );
            }
            continue;
        }
        let Some(param) = def.params.iter().find(|p| p.key == key.as_str()) else {
            return refuse(
                RefusalCode::UnexpectedArgument,
                format!(
                    "Das Modell nannte das Argument „{}“, das dieses Werkzeug nicht kennt.",
                    clean_name(key)
                ),
            );
        };
        seen.insert(param.key);
        let raw = match value {
            Value::Null => "",
            Value::String(s) => s.as_str(),
            _ => {
                return refuse(
                    RefusalCode::BadArguments,
                    format!("Das Argument „{}“ ist kein Text.", param.key),
                )
            }
        };
        match param.kind {
            ParamKind::Text { max, multiline } => {
                let (text, truncated) = sanitize_text(raw, max, multiline);
                if truncated {
                    notes.push(format!("„{}“ wurde auf {max} Zeichen gekürzt.", param.key));
                }
                if param.required && text.is_empty() {
                    return refuse(
                        RefusalCode::MissingArgument,
                        format!("Das Pflichtargument „{}“ ist leer.", param.key),
                    );
                }
                texts.push(text.clone());
                out.insert(param.key.to_string(), Value::String(text));
            }
            ParamKind::Date { out: out_key } => {
                let (phrase, _) = sanitize_text(raw, PHRASE_CHARS, false);
                let mut iso = String::new();
                if !phrase.is_empty() {
                    match resolve_date(&phrase, input.reference) {
                        Ok(date) => iso = date.format("%Y-%m-%d").to_string(),
                        Err(code) => {
                            if param.required {
                                return refuse(
                                    RefusalCode::DateInvalid,
                                    format!(
                                        "Die Zeitangabe in „{}“ ließ sich nicht in ein gültiges Datum übersetzen ({code}).",
                                        param.key
                                    ),
                                );
                            }
                            notes.push(format!(
                                "Zeitangabe in „{}“ verworfen: nicht eindeutig oder ungültig ({code}).",
                                param.key
                            ));
                        }
                    }
                } else if param.required {
                    return refuse(
                        RefusalCode::MissingArgument,
                        format!("Das Pflichtargument „{}“ fehlt.", param.key),
                    );
                }
                out.insert(param.key.to_string(), Value::String(phrase));
                out.insert(out_key.to_string(), Value::String(iso));
                seen.insert(out_key);
            }
        }
    }

    // Fehlende Felder: Pflicht -> Verstoss, sonst leerer Text (Vorlagen im Ablauf scheitern nie an
    // einem fehlenden Pfad).
    for param in def.params {
        if seen.contains(param.key) {
            continue;
        }
        if param.required {
            return refuse(
                RefusalCode::MissingArgument,
                format!("Das Pflichtargument „{}“ fehlt.", param.key),
            );
        }
        out.insert(param.key.to_string(), Value::String(String::new()));
        if let ParamKind::Date { out: out_key } = param.kind {
            out.insert(out_key.to_string(), Value::String(String::new()));
        }
    }

    // 6. Dieselben Muster in der Antwort: das Modell gibt Aufforderungen aus dem Kontext weiter.
    let joined = texts.join("\n");
    if !injection_signals(&joined).is_empty() {
        return refuse(
            RefusalCode::InjectionSuspected,
            "Die Argumente enthalten eine Aufforderung an die KI; die Wahl wird verworfen."
                .to_string(),
        );
    }

    Verdict::Run(Approved {
        tool: def.name,
        action: def.action,
        arguments: out,
        recipients: if def.sends_mail {
            input.recipients.to_vec()
        } else {
            Vec::new()
        },
        notes,
    })
}

// -- Empfaenger ----------------------------------------------------------------------------------

const RECIPIENT_KEYS: &[&str] = &[
    "to",
    "cc",
    "bcc",
    "recipient",
    "recipients",
    "receiver",
    "receivers",
    "empfaenger",
    "empfänger",
    "email",
    "emails",
    "e-mail",
    "address",
    "addresses",
    "adresse",
    "adressen",
    "an",
    "mailto",
    "reply_to",
    "replyto",
    "sender",
    "from",
    "absender",
];

fn is_recipient_key(key: &str) -> bool {
    let k = key.trim().to_lowercase();
    RECIPIENT_KEYS.contains(&k.as_str())
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(items) => items.iter().all(is_blank),
        _ => false,
    }
}

/// Alle Adressen in `value` stehen in `allowed` (ohne Gross-/Kleinschreibung). Nichts genannt =
/// in Ordnung. Fuer ein Werkzeug ohne Mail gibt es keine Menge: jede Angabe ist ein Verstoss.
fn recipients_within(value: &Value, def: &ToolDef, allowed: &[String]) -> bool {
    if is_blank(value) {
        return true;
    }
    if !def.sends_mail || allowed.is_empty() {
        return false;
    }
    let allowed: HashSet<String> = allowed.iter().map(|a| a.trim().to_lowercase()).collect();
    let mut named: Vec<String> = Vec::new();
    match value {
        Value::String(s) => named.extend(split_addresses(s)),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) => named.extend(split_addresses(s)),
                    Value::Null => {}
                    _ => return false,
                }
            }
        }
        _ => return false,
    }
    named.iter().all(|a| allowed.contains(a))
}

fn split_addresses(text: &str) -> Vec<String> {
    text.split([',', ';', '\n', ' '])
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect()
}

// -- Datum ---------------------------------------------------------------------------------------------

fn resolve_date(phrase: &str, reference: NaiveDate) -> Result<NaiveDate, &'static str> {
    let resolved = dates::resolve(phrase, reference).ok_or("date_unresolved")?;
    dates::validate(resolved.date, reference).map_err(dates::DateIssue::code)
}

// -- Text -----------------------------------------------------------------------------------------------

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// Ein Name, wie er in Protokolle darf: `[A-Za-z0-9_.-]`, hoechstens [`NAME_CHARS`] Zeichen,
/// alles andere als `?`.
pub fn clean_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out: String = trimmed
        .chars()
        .take(NAME_CHARS)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '?'
            }
        })
        .collect();
    if trimmed.chars().count() > NAME_CHARS {
        out.push('…');
    }
    if out.is_empty() {
        out.push_str("(leer)");
    }
    out
}

/// Text des Modells fuer die Weitergabe: Steuer- und unsichtbare Zeichen entfernt, einzeilig
/// oder mit Zeilenumbruechen, getrimmt, auf `max` Zeichen gedeckelt (mit `…`). Der Rueckgabewert
/// sagt, ob gekuerzt wurde. Die Eingabe wird vor der Arbeit begrenzt (feindliche Riesentexte).
pub fn sanitize_text(raw: &str, max: usize, multiline: bool) -> (String, bool) {
    let mut s = String::new();
    let mut over = false;
    for (i, c) in raw.chars().enumerate() {
        if i >= max.saturating_mul(4).saturating_add(64) {
            over = true;
            break;
        }
        match c {
            '\r' => {}
            '\n' if multiline => s.push('\n'),
            '\n' | '\t' => s.push(' '),
            c if c.is_control() || is_invisible(c) => {}
            c => s.push(c),
        }
    }
    let flat = if multiline {
        s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
    } else {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let flat = flat.trim().to_string();
    if over || flat.chars().count() > max {
        let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        (cut, true)
    } else {
        (flat, false)
    }
}

// -- Einschleusen --------------------------------------------------------------------------------------

/// Kleinschreibung, Umlaute und ss gleichgesetzt, unsichtbare Zeichen weg, Leerraum zusammengefasst.
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if is_invisible(c) => {}
            c if c.is_whitespace() || c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("fester Ausdruck")
}

static SIGNALS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        (
            "ignore_rules",
            re(
                r"\b(ignorier|missacht|vergiss|umgeh|ueberschreib)\w*\s+(\w+\s+){0,4}(regeln|anweisungen|instruktionen|vorgaben|befehle|sicherheitsregeln|einschraenkungen|richtlinien)\b",
            ),
        ),
        (
            "ignore_rules_en",
            re(
                r"\b(ignore|disregard|forget|override|bypass)\b\s+(\w+\s+){0,4}(instructions|rules|guidelines|prompts?|directives|restrictions)\b",
            ),
        ),
        (
            "ignore_previous_en",
            re(r"\b(ignore|disregard)\s+(all\s+|any\s+|the\s+)?(previous|prior|above|earlier)\b"),
        ),
        (
            "system_prompt",
            re(r"\b(system\s*prompt|systemprompt|developer\s+(message|mode))\b"),
        ),
        (
            "role_marker",
            re(
                r"(\[\[\s*(assistant|system|user)\s*:|<\s*/?\s*(system|assistant)\s*>|#{2,}\s*(system|instruction))",
            ),
        ),
        (
            "new_authority",
            re(
                r"\b(neue|geaenderte)\s+(anweisung|regel|instruktion)\w*\s+(vom|von|des|der)\s+(administrator|admin|chef|it|entwickler|system)",
            ),
        ),
        (
            "role_override",
            re(
                r"\b(du\s+bist\s+(jetzt|ab\s+sofort|nun)|du\s+darfst\s+jetzt|ab\s+sofort\s+(darfst|musst|sollst)|you\s+are\s+now)\b",
            ),
        ),
        (
            "address_ai",
            re(
                r"\b(an\s+den\s+ki[\s-]*assistenten|hey\s+assistent|liebe[rs]?\s+(ki|assistent)|dear\s+(ai|assistant)|an\s+die\s+ki|note\s+to\s+(the\s+)?(ai|assistant))\b",
            ),
        ),
        (
            "tool_command",
            re(
                r"\b((antworte|waehle|nutze|verwende)\s+nur\s+(mit\s+)?(dem\s+)?(werkzeug|tool)|(rufe|ruf|call|invoke)\s+(jetzt\s+|sofort\s+|now\s+)?[a-z]+_[a-z_]+(\s+auf)?)\b",
            ),
        ),
    ]
});

/// Welche Muster einer Aufforderung an die KI stehen in `text`? Leer = keine erkannt. Die Namen
/// (`ignore_rules`, `tool_command`, ...) sind stabil und stehen im Ergebnis des Schritts; der
/// Text selbst nie.
pub fn injection_signals(text: &str) -> Vec<&'static str> {
    let folded = fold(text);
    SIGNALS
        .iter()
        .filter(|(_, pattern)| pattern.is_match(&folded))
        .map(|(id, _)| *id)
        .collect()
}

#[cfg(test)]
mod tests;
