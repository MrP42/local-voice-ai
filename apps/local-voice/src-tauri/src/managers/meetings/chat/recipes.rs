//! Recipes (M4 §7, F13): gespeicherte Fragen mit Variablen `{{name}}`.
//!
//! Variablen wirken auf Text UND Suche: `person`, `folder`, `date_from`,
//! `date_to` und `meeting` setzen zusaetzlich den Filter des Chats. Eine
//! fehlende Pflichtvariable ist ein Fehler VOR jedem LLM-Aufruf. Die
//! mitgelieferten Recipes (`builtin:<key>`) stehen im Code; die Kopien in
//! `chat_recipes` dienen nur Liste und Duplizieren.

use std::collections::HashMap;

use chrono::{NaiveDate, TimeZone};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{ChatError, ChatScope, ScopeFilter, CODE_RECIPE_INVALID};
use crate::managers::meetings::search::index::BUILTIN_PREFIX;
use crate::managers::meetings::store::MeetingStore;

pub const MAX_PROMPT_CHARS: usize = 2_000;
pub const MAX_VARIABLES: usize = 6;
const MAX_LABEL_CHARS: usize = 60;
const MAX_VALUE_CHARS: usize = 200;

static PLACEHOLDER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\{\s*([^{}]*?)\s*\}\}").expect("Platzhalter-Regex"));
static VAR_NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z_]{1,24}$").expect("Name-Regex"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecipeScope {
    Meeting,
    Global,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecipeVarKind {
    Text,
    Person,
    Folder,
    DateFrom,
    DateTo,
    Meeting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecipeVar {
    /// `[a-z_]{1,24}`, im Prompt als `{{name}}`.
    pub name: String,
    pub label: String,
    pub kind: RecipeVarKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecipeSpec {
    /// Immer 1.
    pub version: u32,
    pub prompt: String,
    #[serde(default)]
    pub variables: Vec<RecipeVar>,
    pub scope: RecipeScope,
    /// Auch waehrend der Aufnahme anbietbar.
    #[serde(default)]
    pub live_ok: bool,
}

/// Recipe fuer die UI (`chat_recipes_list`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecipeItem {
    pub id: String,
    pub title: String,
    pub builtin: bool,
    pub spec: RecipeSpec,
    pub updated_at: i64,
}

/// Grund, warum ein Recipe nicht laeuft (`recipe_invalid:<grund>`). Enthaelt
/// hoechstens den Variablennamen, nie einen eingegebenen Wert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeError(pub String);

impl RecipeError {
    fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{CODE_RECIPE_INVALID}:{}", self.0)
    }
}

impl From<RecipeError> for ChatError {
    fn from(e: RecipeError) -> Self {
        ChatError::new(CODE_RECIPE_INVALID, e.0)
    }
}

/// Namen der Platzhalter im Prompt, in Reihenfolge, ohne Doppelte.
pub fn placeholders(prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for caps in PLACEHOLDER.captures_iter(prompt) {
        let name = caps[1].to_string();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

fn parse_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok()
}

/// Prueft Form und Grenzen: Version 1, Prompt 1..=2 000 Zeichen, hoechstens
/// 6 Variablen mit gueltigem, eindeutigem Namen und Label, jeder Platzhalter
/// ist deklariert, Datums-Vorgaben sind lesbar.
pub fn validate_spec(spec: &RecipeSpec) -> Result<(), RecipeError> {
    if spec.version != 1 {
        return Err(RecipeError::new("version"));
    }
    let prompt_chars = spec.prompt.trim().chars().count();
    if prompt_chars == 0 {
        return Err(RecipeError::new("prompt_empty"));
    }
    if prompt_chars > MAX_PROMPT_CHARS {
        return Err(RecipeError::new("prompt_too_long"));
    }
    if spec.variables.len() > MAX_VARIABLES {
        return Err(RecipeError::new("too_many_variables"));
    }
    let mut names: Vec<&str> = Vec::new();
    for var in &spec.variables {
        if !VAR_NAME.is_match(&var.name) {
            return Err(RecipeError::new("variable_name"));
        }
        if names.contains(&var.name.as_str()) {
            return Err(RecipeError::new(format!("duplicate_variable:{}", var.name)));
        }
        names.push(&var.name);
        let label = var.label.trim().chars().count();
        if label == 0 || label > MAX_LABEL_CHARS {
            return Err(RecipeError::new(format!("variable_label:{}", var.name)));
        }
        if let Some(default) = &var.default {
            if default.chars().count() > MAX_VALUE_CHARS {
                return Err(RecipeError::new(format!("default_too_long:{}", var.name)));
            }
            let is_date = matches!(var.kind, RecipeVarKind::DateFrom | RecipeVarKind::DateTo);
            if is_date && !default.trim().is_empty() && parse_date(default).is_none() {
                return Err(RecipeError::new(format!("invalid_date:{}", var.name)));
            }
        }
    }
    for name in placeholders(&spec.prompt) {
        if !names.contains(&name.as_str()) {
            return Err(RecipeError::new("unknown_placeholder"));
        }
    }
    Ok(())
}

/// Namen zu IDs fuer Anzeige und Pruefung (Store im Betrieb, Attrappe im Test).
pub trait RecipeLookup {
    fn folder_name(&self, id: &str) -> Option<String>;
    fn meeting_title(&self, id: &str) -> Option<String>;
}

pub struct StoreLookup<'a>(pub &'a MeetingStore);

impl RecipeLookup for StoreLookup<'_> {
    fn folder_name(&self, id: &str) -> Option<String> {
        self.0
            .folders_list()
            .ok()?
            .into_iter()
            .find(|f| f.id == id)
            .map(|f| f.name)
    }

    fn meeting_title(&self, id: &str) -> Option<String> {
        self.0
            .get_meeting(id)
            .ok()
            .flatten()
            .filter(|m| m.deleted_at.is_none())
            .map(|m| m.title)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderedRecipe {
    pub prompt: String,
    /// Filter-Anteile aus `person`/`folder`/`date_*`/`meeting`.
    pub filter: ScopeFilter,
}

fn local_ts(date: NaiveDate, h: u32, m: u32, s: u32) -> Option<i64> {
    let naive = date.and_hms_opt(h, m, s)?;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.timestamp())
}

/// Setzt die Werte ein und leitet den Filter ab. Leere Werte zaehlen als
/// fehlend (dann gilt die Vorgabe); eine fehlende Pflichtvariable, ein zu
/// langer Wert, ein unbekannter Ordner/eine unbekannte Besprechung oder ein
/// unlesbares Datum sind Fehler.
pub fn render_recipe(
    spec: &RecipeSpec,
    values: &HashMap<String, String>,
    lookup: &dyn RecipeLookup,
) -> Result<RenderedRecipe, RecipeError> {
    validate_spec(spec)?;
    let mut shown: HashMap<String, String> = HashMap::new();
    let mut filter = ScopeFilter::default();
    for var in &spec.variables {
        let given = values
            .get(&var.name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let fallback = var
            .default
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let Some(raw) = given.or(fallback) else {
            if var.required {
                return Err(RecipeError::new(format!("missing_variable:{}", var.name)));
            }
            shown.insert(var.name.clone(), String::new());
            continue;
        };
        if raw.chars().count() > MAX_VALUE_CHARS {
            return Err(RecipeError::new(format!("value_too_long:{}", var.name)));
        }
        let display = match var.kind {
            RecipeVarKind::Text => raw,
            RecipeVarKind::Person => {
                filter.person = Some(raw.clone());
                raw
            }
            RecipeVarKind::Folder => {
                let name = lookup
                    .folder_name(&raw)
                    .ok_or_else(|| RecipeError::new(format!("unknown_folder:{}", var.name)))?;
                filter.folder_id = Some(raw);
                name
            }
            RecipeVarKind::DateFrom | RecipeVarKind::DateTo => {
                let date = parse_date(&raw)
                    .ok_or_else(|| RecipeError::new(format!("invalid_date:{}", var.name)))?;
                let invalid = || RecipeError::new(format!("invalid_date:{}", var.name));
                if var.kind == RecipeVarKind::DateFrom {
                    filter.from = Some(local_ts(date, 0, 0, 0).ok_or_else(invalid)?);
                } else {
                    filter.to = Some(local_ts(date, 23, 59, 59).ok_or_else(invalid)?);
                }
                date.format("%d.%m.%Y").to_string()
            }
            RecipeVarKind::Meeting => {
                let title = lookup
                    .meeting_title(&raw)
                    .ok_or_else(|| RecipeError::new(format!("unknown_meeting:{}", var.name)))?;
                filter.meeting_ids = Some(vec![raw]);
                format!("„{title}“")
            }
        };
        shown.insert(var.name.clone(), display);
    }
    let prompt = PLACEHOLDER.replace_all(&spec.prompt, |caps: &regex::Captures<'_>| {
        shown.get(&caps[1]).cloned().unwrap_or_default()
    });
    let prompt = prompt
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    Ok(RenderedRecipe { prompt, filter })
}

/// Passt das Recipe zum Chat? `meeting`-Recipes nur fuer eine Besprechung,
/// `global`-Recipes nur ueber viele, waehrend der Aufnahme nur `live_ok`.
pub fn check_use(spec: &RecipeSpec, meeting_scope: bool, live: bool) -> Result<(), RecipeError> {
    match spec.scope {
        RecipeScope::Meeting if !meeting_scope => return Err(RecipeError::new("needs_meeting")),
        RecipeScope::Global if meeting_scope => return Err(RecipeError::new("needs_global")),
        _ => {}
    }
    if live && !spec.live_ok {
        return Err(RecipeError::new("not_live"));
    }
    Ok(())
}

/// Traegt den Recipe-Filter in einen globalen Scope ein (gesetzte Felder
/// gewinnen). Eine einzelne Besprechung bleibt, wie sie ist.
pub fn apply_filter(scope: ChatScope, patch: &ScopeFilter) -> ChatScope {
    match scope {
        ChatScope::Meeting { .. } => scope,
        ChatScope::Global { mut filter } => {
            if patch.meeting_ids.is_some() {
                filter.meeting_ids = patch.meeting_ids.clone();
            }
            if patch.folder_id.is_some() {
                filter.folder_id = patch.folder_id.clone();
            }
            if patch.person.is_some() {
                filter.person = patch.person.clone();
            }
            if patch.from.is_some() {
                filter.from = patch.from;
            }
            if patch.to.is_some() {
                filter.to = patch.to;
            }
            ChatScope::Global { filter }
        }
    }
}

// ---------------------------------------------------------------------------
// Mitgelieferte Recipes
// ---------------------------------------------------------------------------

fn var(name: &str, label: &str, kind: RecipeVarKind) -> RecipeVar {
    RecipeVar {
        name: name.into(),
        label: label.into(),
        kind,
        required: true,
        default: None,
    }
}

fn builtin(
    key: &str,
    title: &str,
    scope: RecipeScope,
    live_ok: bool,
    variables: Vec<RecipeVar>,
    prompt: &str,
) -> RecipeItem {
    RecipeItem {
        id: format!("{BUILTIN_PREFIX}{key}"),
        title: title.into(),
        builtin: true,
        spec: RecipeSpec {
            version: 1,
            prompt: prompt.into(),
            variables,
            scope,
            live_ok,
        },
        updated_at: 0,
    }
}

/// Die mitgelieferten Recipes (M4 §7) in Anzeigereihenfolge.
pub fn builtin_recipes() -> Vec<RecipeItem> {
    use RecipeScope::{Global, Meeting};
    use RecipeVarKind::{DateFrom, Folder, Person, Text};
    vec![
        builtin(
            "was-verpasst",
            "Was habe ich verpasst?",
            Meeting,
            true,
            vec![],
            "Was wurde in den letzten Minuten der Besprechung besprochen? Fasse Themen, \
             Entscheidungen und offene Fragen knapp in Stichpunkten zusammen.",
        ),
        builtin(
            "was-fragen",
            "Was sollte ich jetzt fragen?",
            Meeting,
            true,
            vec![],
            "Schlage mir auf Grundlage des bisherigen Gesprächs drei bis fünf sinnvolle Fragen \
             vor, die ich jetzt stellen sollte. Begründe jede Frage kurz mit dem, was gesagt wurde.",
        ),
        builtin(
            "follow-up-mail",
            "Follow-up-E-Mail an {{empfaenger}}",
            Meeting,
            false,
            vec![var("empfaenger", "Empfänger", Text)],
            "Schreibe eine kurze Follow-up-E-Mail an {{empfaenger}} zu dieser Besprechung: Dank \
             für das Gespräch, die wichtigsten Ergebnisse und die vereinbarten nächsten Schritte \
             mit Verantwortlichen und Terminen.\nDie erste Zeile lautet genau „Betreff: <Betreff \
             der E-Mail>“. Danach folgt der Text der E-Mail ohne Überschriften.",
        ),
        builtin(
            "offene-aufgaben",
            "Offene Aufgaben von {{person}} seit {{date_from}}",
            Global,
            false,
            vec![var("person", "Person", Person), var("date_from", "Seit", DateFrom)],
            "Welche offenen Aufgaben hat {{person}} seit dem {{date_from}} übernommen oder \
             zugesagt? Nenne je Aufgabe, was zu tun ist, bis wann und in welcher Besprechung sie \
             vereinbart wurde.",
        ),
        builtin(
            "entscheidungen-ordner",
            "Entscheidungen im Ordner {{folder}}",
            Global,
            false,
            vec![var("folder", "Ordner", Folder)],
            "Welche Entscheidungen wurden in den Besprechungen im Ordner {{folder}} getroffen? \
             Nenne je Entscheidung, was entschieden wurde und wann.",
        ),
        builtin(
            "einwaende",
            "Einwände und Bedenken von Kunden",
            Global,
            false,
            vec![],
            "Welche Einwände und Bedenken haben Kunden in den Besprechungen geäußert? Fasse sie \
             nach Thema zusammen und nenne jeweils, wann sie geäußert wurden.",
        ),
        builtin(
            "vorbereitung-person",
            "Vorbereitung auf das Gespräch mit {{person}}",
            Global,
            false,
            vec![var("person", "Person", Person)],
            "Bereite mich auf das nächste Gespräch mit {{person}} vor: Was wurde bisher \
             besprochen, was ist offen, was wurde zugesagt, und welche Themen sollte ich \
             ansprechen?",
        ),
        // M5-P5e: Brief zu einem Kalendertermin. Die Teilnehmenden stehen als
        // Text (nicht als `person`): der Scope kommt fertig vom Termin
        // (`people::brief_scope`), ein Namensfilter wuerde ihn wieder leeren.
        builtin(
            "vorbereitung-termin",
            "Vorbereitung auf den Termin mit {{teilnehmende}}",
            Global,
            false,
            vec![var("teilnehmende", "Teilnehmende", Text)],
            "Bereite mich auf den Termin mit {{teilnehmende}} vor: Was wurde in den früheren \
             Besprechungen mit diesen Teilnehmenden besprochen, was ist offen, was wurde \
             zugesagt, und welche Themen sollte ich ansprechen?",
        ),
    ]
}

/// Zeilen fuer `MeetingStore::recipes_seed_builtin` (`(id, title, spec_json)`).
pub fn builtin_seed_items() -> Vec<(String, String, String)> {
    builtin_recipes()
        .into_iter()
        .map(|r| {
            let json = serde_json::to_string(&r.spec).expect("Spec ist serialisierbar");
            (r.id, r.title, json)
        })
        .collect()
}

/// Recipe zu einer ID: mitgelieferte aus dem Code, eigene aus dem Store.
pub fn load_recipe(store: &MeetingStore, id: &str) -> Result<RecipeItem, ChatError> {
    if id.starts_with(BUILTIN_PREFIX) {
        return builtin_recipes()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| ChatError::new(CODE_RECIPE_INVALID, "not_found"));
    }
    let info = store
        .recipe_get(id)
        .map_err(|e| ChatError::new(super::CODE_STORE_FAILED, e.to_string()))?
        .ok_or_else(|| ChatError::new(CODE_RECIPE_INVALID, "not_found"))?;
    let spec: RecipeSpec = serde_json::from_str(&info.spec_json)
        .map_err(|_| ChatError::new(CODE_RECIPE_INVALID, "spec"))?;
    Ok(RecipeItem {
        id: info.id,
        title: info.title,
        builtin: false,
        spec,
        updated_at: info.updated_at,
    })
}

/// Liste fuer die UI: mitgelieferte (Spec aus dem Code), dann eigene. Eine
/// eigene Zeile mit unlesbarer Spec wird uebersprungen (gezaehlt im Log).
pub fn list_recipes(store: &MeetingStore) -> anyhow::Result<Vec<RecipeItem>> {
    let builtins = builtin_recipes();
    let mut out = builtins.clone();
    let mut skipped = 0usize;
    for info in store.recipes_list()? {
        if info.builtin {
            continue;
        }
        match serde_json::from_str::<RecipeSpec>(&info.spec_json) {
            Ok(spec) => out.push(RecipeItem {
                id: info.id,
                title: info.title,
                builtin: false,
                spec,
                updated_at: info.updated_at,
            }),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        log::warn!("Recipes: {skipped} Eintraege mit unlesbarer Spec uebersprungen");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::tmp_store;

    struct FakeLookup;

    impl RecipeLookup for FakeLookup {
        fn folder_name(&self, id: &str) -> Option<String> {
            (id == "f1").then(|| "Vertrieb".to_string())
        }
        fn meeting_title(&self, id: &str) -> Option<String> {
            (id == "m1").then(|| "Kickoff".to_string())
        }
    }

    fn builtin_spec(key: &str) -> RecipeSpec {
        builtin_recipes()
            .into_iter()
            .find(|r| r.id == format!("builtin:{key}"))
            .unwrap()
            .spec
    }

    fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn spec(prompt: &str, variables: Vec<RecipeVar>) -> RecipeSpec {
        RecipeSpec {
            version: 1,
            prompt: prompt.into(),
            variables,
            scope: RecipeScope::Any,
            live_ok: false,
        }
    }

    #[test]
    fn person_variable_sets_filter() {
        let spec = builtin_spec("offene-aufgaben");
        let out = render_recipe(
            &spec,
            &values(&[("person", "  Anna Berg "), ("date_from", "2026-09-01")]),
            &FakeLookup,
        )
        .unwrap();
        assert!(
            out.prompt.contains("hat Anna Berg seit dem 01.09.2026"),
            "{}",
            out.prompt
        );
        assert_eq!(out.filter.person.as_deref(), Some("Anna Berg"));
        let midnight = chrono::Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 9, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
            .timestamp();
        assert_eq!(out.filter.from, Some(midnight));
        assert!(out.filter.folder_id.is_none() && out.filter.to.is_none());
        // Der Filter landet im globalen Scope, nicht in einer Einzelbesprechung.
        match apply_filter(
            ChatScope::Global {
                filter: ScopeFilter {
                    folder_id: Some("f9".into()),
                    ..ScopeFilter::default()
                },
            },
            &out.filter,
        ) {
            ChatScope::Global { filter } => {
                assert_eq!(filter.person.as_deref(), Some("Anna Berg"));
                assert_eq!(
                    filter.folder_id.as_deref(),
                    Some("f9"),
                    "bestehender Filter bleibt"
                );
            }
            _ => panic!("global erwartet"),
        }
        assert!(matches!(
            apply_filter(
                ChatScope::Meeting {
                    meeting_id: "m".into()
                },
                &out.filter
            ),
            ChatScope::Meeting { .. }
        ));
    }

    #[test]
    fn a_missing_required_variable_fails_before_the_llm() {
        let spec = builtin_spec("offene-aufgaben");
        let err =
            render_recipe(&spec, &values(&[("date_from", "2026-09-01")]), &FakeLookup).unwrap_err();
        assert_eq!(err.0, "missing_variable:person");
        // Leerraum zaehlt als fehlend.
        let err = render_recipe(
            &spec,
            &values(&[("person", "   "), ("date_from", "2026-09-01")]),
            &FakeLookup,
        )
        .unwrap_err();
        assert_eq!(err.0, "missing_variable:person");
        assert_eq!(
            ChatError::from(err).to_string(),
            "recipe_invalid: missing_variable:person"
        );
    }

    #[test]
    fn folder_meeting_and_date_variables_resolve_and_filter() {
        let s = spec(
            "Ordner {{ordner}}, Besprechung {{b}}, bis {{bis}}",
            vec![
                var("ordner", "Ordner", RecipeVarKind::Folder),
                var("b", "Besprechung", RecipeVarKind::Meeting),
                var("bis", "Bis", RecipeVarKind::DateTo),
            ],
        );
        let out = render_recipe(
            &s,
            &values(&[("ordner", "f1"), ("b", "m1"), ("bis", "2026-09-30")]),
            &FakeLookup,
        )
        .unwrap();
        assert_eq!(
            out.prompt,
            "Ordner Vertrieb, Besprechung „Kickoff“, bis 30.09.2026"
        );
        assert_eq!(out.filter.folder_id.as_deref(), Some("f1"));
        assert_eq!(out.filter.meeting_ids, Some(vec!["m1".to_string()]));
        let end = out.filter.to.unwrap();
        let start_next = local_ts(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), 0, 0, 0).unwrap();
        assert_eq!(start_next - end, 1, "Ende des Tages");

        for (vals, reason) in [
            (
                vec![("ordner", "f9"), ("b", "m1"), ("bis", "2026-09-30")],
                "unknown_folder:ordner",
            ),
            (
                vec![("ordner", "f1"), ("b", "m9"), ("bis", "2026-09-30")],
                "unknown_meeting:b",
            ),
            (
                vec![("ordner", "f1"), ("b", "m1"), ("bis", "30.09.2026")],
                "invalid_date:bis",
            ),
        ] {
            assert_eq!(
                render_recipe(&s, &values(&vals), &FakeLookup)
                    .unwrap_err()
                    .0,
                reason
            );
        }
    }

    #[test]
    fn optional_variables_use_the_default_or_stay_empty() {
        let mut thema = var("thema", "Thema", RecipeVarKind::Text);
        thema.required = false;
        thema.default = Some("Budget".into());
        let mut zusatz = var("zusatz", "Zusatz", RecipeVarKind::Text);
        zusatz.required = false;
        let s = spec("Was gilt zu {{thema}} {{zusatz}}?", vec![thema, zusatz]);
        let out = render_recipe(&s, &HashMap::new(), &FakeLookup).unwrap();
        assert_eq!(out.prompt, "Was gilt zu Budget ?");
        let out = render_recipe(&s, &values(&[("thema", "Termine")]), &FakeLookup).unwrap();
        assert_eq!(out.prompt, "Was gilt zu Termine ?");
        let long = "x".repeat(MAX_VALUE_CHARS + 1);
        assert_eq!(
            render_recipe(&s, &values(&[("zusatz", long.as_str())]), &FakeLookup)
                .unwrap_err()
                .0,
            "value_too_long:zusatz"
        );
    }

    #[test]
    fn invalid_specs_are_rejected() {
        let ok = spec("Frage zu {{a}}", vec![var("a", "A", RecipeVarKind::Text)]);
        assert!(validate_spec(&ok).is_ok());
        let cases: Vec<(RecipeSpec, &str)> = vec![
            (
                spec("Frage zu {{b}}", vec![var("a", "A", RecipeVarKind::Text)]),
                "unknown_placeholder",
            ),
            (spec("   ", vec![]), "prompt_empty"),
            (
                spec(&"x".repeat(MAX_PROMPT_CHARS + 1), vec![]),
                "prompt_too_long",
            ),
            (
                spec(
                    "x",
                    (0..7)
                        .map(|i| var(&"v".repeat(i + 1), "L", RecipeVarKind::Text))
                        .collect(),
                ),
                "too_many_variables",
            ),
            (
                spec("x", vec![var("Name", "L", RecipeVarKind::Text)]),
                "variable_name",
            ),
            (
                spec("x", vec![var(&"a".repeat(25), "L", RecipeVarKind::Text)]),
                "variable_name",
            ),
            (
                spec(
                    "x",
                    vec![
                        var("a", "L", RecipeVarKind::Text),
                        var("a", "M", RecipeVarKind::Text),
                    ],
                ),
                "duplicate_variable:a",
            ),
            (
                spec("x", vec![var("a", " ", RecipeVarKind::Text)]),
                "variable_label:a",
            ),
            (
                spec(
                    "x",
                    vec![RecipeVar {
                        default: Some("gestern".into()),
                        ..var("d", "D", RecipeVarKind::DateFrom)
                    }],
                ),
                "invalid_date:d",
            ),
            (
                RecipeSpec {
                    version: 2,
                    ..ok.clone()
                },
                "version",
            ),
        ];
        for (s, reason) in cases {
            assert_eq!(validate_spec(&s).unwrap_err().0, reason);
        }
        // Eine deklarierte, aber im Text ungenutzte Variable wirkt nur auf den Filter.
        let filter_only = spec(
            "Was ist offen?",
            vec![var("person", "P", RecipeVarKind::Person)],
        );
        let out = render_recipe(&filter_only, &values(&[("person", "Ben")]), &FakeLookup).unwrap();
        assert_eq!(out.prompt, "Was ist offen?");
        assert_eq!(out.filter.person.as_deref(), Some("Ben"));
    }

    #[test]
    fn builtins_are_valid_and_the_follow_up_mail_demands_a_subject_line() {
        let all = builtin_recipes();
        assert_eq!(all.len(), 8);
        for r in &all {
            validate_spec(&r.spec).unwrap_or_else(|e| panic!("{}: {e}", r.id));
            assert!(r.id.starts_with("builtin:"));
            assert!(r.builtin);
            // Titel-Platzhalter sind Variablen des Recipes.
            for name in placeholders(&r.title) {
                assert!(
                    r.spec.variables.iter().any(|v| v.name == name),
                    "{}: {name}",
                    r.id
                );
            }
        }
        let live: Vec<&str> = all
            .iter()
            .filter(|r| r.spec.live_ok)
            .map(|r| r.title.as_str())
            .collect();
        assert_eq!(
            live,
            vec!["Was habe ich verpasst?", "Was sollte ich jetzt fragen?"]
        );
        let mail = builtin_spec("follow-up-mail");
        let out =
            render_recipe(&mail, &values(&[("empfaenger", "Frau Weber")]), &FakeLookup).unwrap();
        assert!(out.prompt.contains("an Frau Weber"));
        assert!(out
            .prompt
            .contains("Die erste Zeile lautet genau „Betreff: "));
        assert!(out.prompt.contains("ohne Überschriften"));
        // Seed-Zeilen sind gueltiges JSON, das wieder zur Spec wird.
        for (id, _, json) in builtin_seed_items() {
            let back: RecipeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back, all.iter().find(|r| r.id == id).unwrap().spec, "{id}");
        }
    }

    #[test]
    fn recipes_only_run_where_they_fit() {
        let meeting_only = builtin_spec("was-verpasst");
        assert!(check_use(&meeting_only, true, true).is_ok());
        assert_eq!(
            check_use(&meeting_only, false, false).unwrap_err().0,
            "needs_meeting"
        );
        let global = builtin_spec("einwaende");
        assert_eq!(
            check_use(&global, true, false).unwrap_err().0,
            "needs_global"
        );
        let mail = builtin_spec("follow-up-mail");
        assert_eq!(check_use(&mail, true, true).unwrap_err().0, "not_live");
        let any = spec("x", vec![]);
        assert!(check_use(&any, true, false).is_ok() && check_use(&any, false, false).is_ok());
    }

    #[test]
    fn recipes_load_from_code_and_store() {
        let (_dir, store) = tmp_store();
        store.recipes_seed_builtin(&builtin_seed_items()).unwrap();
        let own = store
            .recipe_save(
                None,
                "Budgetfrage",
                &serde_json::to_string(&spec("Budget?", vec![])).unwrap(),
            )
            .unwrap();
        let broken = store
            .recipe_save(None, "Kaputt", r#"{"version":1}"#)
            .unwrap();

        let loaded = load_recipe(&store, &own.id).unwrap();
        assert_eq!(loaded.spec.prompt, "Budget?");
        assert!(!loaded.builtin);
        assert_eq!(
            load_recipe(&store, "builtin:einwaende").unwrap().title,
            "Einwände und Bedenken von Kunden"
        );
        assert_eq!(
            load_recipe(&store, "gibt-es-nicht").unwrap_err().detail,
            "not_found"
        );
        assert_eq!(
            load_recipe(&store, "builtin:gibt-es-nicht")
                .unwrap_err()
                .code,
            CODE_RECIPE_INVALID
        );
        assert_eq!(load_recipe(&store, &broken.id).unwrap_err().detail, "spec");

        let list = list_recipes(&store).unwrap();
        let ids: Vec<&str> = list.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            9,
            "8 mitgelieferte + 1 eigenes, das kaputte fehlt: {ids:?}"
        );
        assert!(ids[..8].iter().all(|i| i.starts_with("builtin:")));
        assert_eq!(ids[8], own.id);
    }
}
