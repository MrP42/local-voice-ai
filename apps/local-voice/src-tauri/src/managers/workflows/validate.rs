//! Pruefung einer Definition `lva-workflow@1` (B1).
//!
//! Die Pruefung laeuft auf dem rohen JSON, nicht auf den Typen: so gibt es zu jedem
//! Fehler einen JSON-Zeiger (`/steps/2/params/to`) und einen deutschen Satz, und
//! mehrere Fehler auf einmal (das Formular der Oberflaeche markiert sie alle).
//! Erst wenn nichts mehr zu beanstanden ist, entsteht der Typ `WorkflowDef`.
//!
//! Geprueft wird:
//! - Form und Grenzen (Schritte, Variablen, Textlaengen, Groesse),
//! - `trigger.type` und seine Felder, `steps[].action` und deren Parameter gegen den
//!   Katalog (`catalog`), feste Felder (Empfaenger) ohne `{{...}}`,
//! - jede Bedingung und jede Vorlage PARSET (`expr`), und jeder Pfad darin zeigt auf
//!   etwas, das es geben kann: `steps.<id>` nur auf einen FRUEHEREN Schritt,
//!   `vars.<name>` nur auf eine deklarierte Variable, `trigger.<feld>` nur auf ein
//!   Feld, das der Ausloeser liefert.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use super::catalog::{self, FieldSpec, TriggerSpec};
use super::expr::{self, Path};
use super::model::{
    VarType, WorkflowDef, MAX_ATTEMPTS_LIMIT, MAX_BACKOFF_MS, MAX_DEFINITION_BYTES, MAX_NAME_CHARS,
    MAX_PARAMS_BYTES, MAX_STEPS, MAX_TEXT_CHARS, MAX_VARIABLES, MIN_BACKOFF_MS, SCHEMA_ID,
};

/// Ein Befund: wo (JSON-Zeiger, "" = ganze Definition) und was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

struct Issues(Vec<Issue>);

impl Issues {
    fn add(&mut self, path: &str, message: impl Into<String>) {
        if self.0.len() < 200 {
            self.0.push(Issue {
                path: path.to_string(),
                message: message.into(),
            });
        }
    }
}

/// Erlaubter Name fuer Schritte und Variablen: `[a-z][a-z0-9_]{0,31}`.
pub fn valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Bereiche, aus denen Ausdruecke lesen duerfen.
pub const ROOTS: &[&str] = &["trigger", "vars", "steps", "meeting", "run", "workflow"];
const RUN_FIELDS: &[&str] = &["id", "dry_run", "started_at"];
const WORKFLOW_FIELDS: &[&str] = &["id", "name"];

fn check_unknown_keys(obj: &Map<String, Value>, allowed: &[&str], at: &str, issues: &mut Issues) {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            issues.add(
                &format!("{at}/{key}"),
                format!("Unbekanntes Feld „{key}“ (erlaubt: {}).", allowed.join(", ")),
            );
        }
    }
}

fn text_field(
    obj: &Map<String, Value>,
    key: &str,
    at: &str,
    max: usize,
    required: bool,
    issues: &mut Issues,
) -> Option<String> {
    match obj.get(key) {
        None | Some(Value::Null) if !required => None,
        None | Some(Value::Null) => {
            issues.add(&format!("{at}/{key}"), "Pflichtfeld fehlt.");
            None
        }
        Some(Value::String(s)) => {
            if s.chars().count() > max {
                issues.add(
                    &format!("{at}/{key}"),
                    format!("Zu lang (höchstens {max} Zeichen)."),
                );
            }
            if s.chars().any(|c| c.is_control() && c != '\n') {
                issues.add(&format!("{at}/{key}"), "Enthält Steuerzeichen.");
            }
            Some(s.clone())
        }
        Some(_) => {
            issues.add(&format!("{at}/{key}"), "Text erwartet.");
            None
        }
    }
}

/// Fuer Pfade in Ausdruecken: gibt es das, was gelesen wird?
struct RefScope<'a> {
    /// Schritte VOR dem aktuellen.
    steps_before: &'a HashSet<String>,
    /// Alle Schritte (fuer die Meldung "laeuft erst spaeter").
    steps_all: &'a HashSet<String>,
    vars: &'a BTreeSet<String>,
    trigger: Option<&'static TriggerSpec>,
}

fn check_refs(refs: &[Path], at: &str, scope: &RefScope<'_>, issues: &mut Issues) {
    for p in refs {
        let root = p.root();
        match root {
            "steps" => match p.second() {
                None => issues.add(at, "„steps“ braucht einen Schrittnamen (steps.<schritt>.<feld>)."),
                Some(id) if scope.steps_before.contains(id) => {}
                Some(id) if scope.steps_all.contains(id) => issues.add(
                    at,
                    format!("Schritt „{id}“ läuft erst später oder ist dieser Schritt selbst; sein Ergebnis gibt es hier noch nicht."),
                ),
                Some(id) => issues.add(at, format!("Es gibt keinen Schritt „{id}“.")),
            },
            "vars" => match p.second() {
                None => issues.add(at, "„vars“ braucht einen Variablennamen (vars.<name>)."),
                Some(name) if scope.vars.contains(name) => {}
                Some(name) => issues.add(at, format!("Die Variable „{name}“ ist nicht deklariert.")),
            },
            "trigger" => {
                if let (Some(spec), Some(field)) = (scope.trigger, p.second()) {
                    if !spec.provides.is_empty() && !spec.provides.contains(&field) {
                        issues.add(
                            at,
                            format!(
                                "Der Auslöser „{}“ liefert kein Feld „{field}“ (verfügbar: {}).",
                                spec.id,
                                spec.provides.join(", ")
                            ),
                        );
                    }
                }
            }
            "run" => {
                if p.second().is_none_or(|f| !RUN_FIELDS.contains(&f)) {
                    issues.add(at, format!("„run“ kennt nur: {}.", RUN_FIELDS.join(", ")));
                }
            }
            "workflow" => {
                if p.second().is_none_or(|f| !WORKFLOW_FIELDS.contains(&f)) {
                    issues.add(
                        at,
                        format!("„workflow“ kennt nur: {}.", WORKFLOW_FIELDS.join(", ")),
                    );
                }
            }
            "meeting" => {}
            other => issues.add(
                at,
                format!(
                    "Unbekannter Bereich „{other}“ (erlaubt: {}).",
                    ROOTS.join(", ")
                ),
            ),
        }
    }
}

fn check_trigger(v: Option<&Value>, issues: &mut Issues) -> Option<&'static TriggerSpec> {
    let Some(Value::Object(obj)) = v else {
        issues.add("/trigger", "Der Auslöser fehlt oder ist kein Objekt.");
        return None;
    };
    let Some(Value::String(kind)) = obj.get("type") else {
        issues.add("/trigger/type", "Pflichtfeld „type“ fehlt (Text erwartet).");
        return None;
    };
    let Some(spec) = catalog::trigger_spec(kind) else {
        let known: Vec<&str> = catalog::triggers().iter().map(|t| t.id).collect();
        issues.add(
            "/trigger/type",
            format!("Unbekannter Auslöser „{kind}“ (bekannt: {}).", known.join(", ")),
        );
        return None;
    };
    for (key, val) in obj {
        if key == "type" {
            continue;
        }
        match spec.fields.iter().find(|f| f.name == key) {
            None => {
                let names: Vec<&str> = spec.fields.iter().map(|f| f.name).collect();
                issues.add(
                    &format!("/trigger/{key}"),
                    format!(
                        "Unbekanntes Feld „{key}“ für „{kind}“ (erlaubt: {}).",
                        if names.is_empty() {
                            "keine".to_string()
                        } else {
                            names.join(", ")
                        }
                    ),
                );
            }
            Some(f) => {
                if let Err(m) = catalog::check_field(f, val, false) {
                    issues.add(&format!("/trigger/{key}"), m);
                }
            }
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        if !obj.contains_key(f.name) {
            issues.add(&format!("/trigger/{}", f.name), "Pflichtfeld fehlt.");
        }
    }
    Some(spec)
}

fn check_variables(v: Option<&Value>, issues: &mut Issues) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let obj = match v {
        None | Some(Value::Null) => return names,
        Some(Value::Object(o)) => o,
        Some(_) => {
            issues.add("/variables", "Objekt erwartet (Name -> Beschreibung).");
            return names;
        }
    };
    if obj.len() > MAX_VARIABLES {
        issues.add(
            "/variables",
            format!("Zu viele Variablen (höchstens {MAX_VARIABLES})."),
        );
    }
    for (name, decl) in obj {
        let at = format!("/variables/{name}");
        if !valid_ident(name) {
            issues.add(
                &at,
                "Name: Kleinbuchstaben, Ziffern und _, mit einem Buchstaben beginnend, höchstens 32 Zeichen.",
            );
        }
        names.insert(name.clone());
        let Value::Object(d) = decl else {
            issues.add(&at, "Objekt erwartet ({\"type\": ..., \"default\": ...}).");
            continue;
        };
        check_unknown_keys(d, &["type", "default", "description"], &at, issues);
        text_field(d, "description", &at, MAX_TEXT_CHARS, false, issues);
        let ty = match d.get("type").and_then(Value::as_str) {
            Some("string") => Some(VarType::String),
            Some("number") => Some(VarType::Number),
            Some("bool") => Some(VarType::Bool),
            Some("list") => Some(VarType::List),
            _ => {
                issues.add(
                    &format!("{at}/type"),
                    "Pflichtfeld „type“: string, number, bool oder list.",
                );
                None
            }
        };
        if let (Some(ty), Some(default)) = (ty, d.get("default")) {
            if !default.is_null() && !ty.accepts(default) {
                issues.add(
                    &format!("{at}/default"),
                    format!("Der Vorgabewert passt nicht zur Art „{}“.", ty.as_str()),
                );
            }
        }
    }
    names
}

fn check_params(
    spec: &catalog::ActionSpec,
    params: &Map<String, Value>,
    at: &str,
    scope: &RefScope<'_>,
    issues: &mut Issues,
) {
    let names: Vec<&str> = spec.fields.iter().map(|f| f.name).collect();
    for (key, val) in params {
        match spec.fields.iter().find(|f: &&FieldSpec| f.name == key) {
            None => issues.add(
                &format!("{at}/{key}"),
                format!(
                    "Unbekannter Parameter „{key}“ für „{}“ (erlaubt: {}).",
                    spec.id,
                    if names.is_empty() {
                        "keine".to_string()
                    } else {
                        names.join(", ")
                    }
                ),
            ),
            Some(f) => {
                if let Err(m) = catalog::check_field(f, val, true) {
                    issues.add(&format!("{at}/{key}"), m);
                }
            }
        }
    }
    for f in spec.fields.iter().filter(|f| f.required) {
        match params.get(f.name) {
            None | Some(Value::Null) => {
                issues.add(&format!("{at}/{}", f.name), "Pflichtparameter fehlt.");
            }
            _ => {}
        }
    }
    match expr::collect_value_refs(&Value::Object(params.clone())) {
        Ok(refs) => check_refs(&refs, at, scope, issues),
        Err((pointer, e)) => issues.add(&format!("{at}{pointer}"), e.to_string()),
    }
}

fn check_retry(v: &Value, at: &str, issues: &mut Issues) {
    let Value::Object(o) = v else {
        issues.add(at, "Objekt erwartet ({\"max_attempts\": 3, \"backoff_ms\": 2000}).");
        return;
    };
    check_unknown_keys(o, &["max_attempts", "backoff_ms"], at, issues);
    if let Some(a) = o.get("max_attempts") {
        match a.as_u64() {
            Some(n) if (1..=MAX_ATTEMPTS_LIMIT as u64).contains(&n) => {}
            _ => issues.add(
                &format!("{at}/max_attempts"),
                format!("Ganze Zahl von 1 bis {MAX_ATTEMPTS_LIMIT} erwartet."),
            ),
        }
    }
    if let Some(b) = o.get("backoff_ms") {
        match b.as_u64() {
            Some(n) if (MIN_BACKOFF_MS..=MAX_BACKOFF_MS).contains(&n) => {}
            _ => issues.add(
                &format!("{at}/backoff_ms"),
                format!("Ganze Zahl von {MIN_BACKOFF_MS} bis {MAX_BACKOFF_MS} (Millisekunden) erwartet."),
            ),
        }
    }
}

fn check_steps(
    v: Option<&Value>,
    trigger: Option<&'static TriggerSpec>,
    vars: &BTreeSet<String>,
    issues: &mut Issues,
) {
    let Some(Value::Array(steps)) = v else {
        issues.add("/steps", "Die Schritte fehlen (Liste erwartet).");
        return;
    };
    if steps.is_empty() {
        issues.add("/steps", "Ein Ablauf braucht mindestens einen Schritt.");
    }
    if steps.len() > MAX_STEPS {
        issues.add(
            "/steps",
            format!("Zu viele Schritte (höchstens {MAX_STEPS})."),
        );
    }
    // Erst alle Kennungen einsammeln (fuer "laeuft erst spaeter").
    let mut all_ids: HashSet<String> = HashSet::new();
    for s in steps {
        if let Some(Value::String(id)) = s.get("id") {
            all_ids.insert(id.clone());
        }
    }
    let mut before: HashSet<String> = HashSet::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (i, step) in steps.iter().enumerate() {
        let at = format!("/steps/{i}");
        let Value::Object(obj) = step else {
            issues.add(&at, "Objekt erwartet.");
            continue;
        };
        check_unknown_keys(
            obj,
            &["id", "action", "label", "when", "params", "on_error", "retry"],
            &at,
            issues,
        );
        let id = text_field(obj, "id", &at, 32, true, issues);
        if let Some(id) = &id {
            if !valid_ident(id) {
                issues.add(
                    &format!("{at}/id"),
                    "Name: Kleinbuchstaben, Ziffern und _, mit einem Buchstaben beginnend, höchstens 32 Zeichen.",
                );
            }
            if !seen.insert(id.clone()) {
                issues.add(
                    &format!("{at}/id"),
                    format!("Der Name „{id}“ kommt mehrfach vor."),
                );
            }
        }
        text_field(obj, "label", &at, MAX_NAME_CHARS, false, issues);
        let scope = RefScope {
            steps_before: &before,
            steps_all: &all_ids,
            vars,
            trigger,
        };
        if let Some(when) = text_field(obj, "when", &at, expr::MAX_EXPR_CHARS, false, issues) {
            match expr::parse_condition(&when) {
                Ok(e) => check_refs(&e.refs(), &format!("{at}/when"), &scope, issues),
                Err(e) => issues.add(&format!("{at}/when"), e.to_string()),
            }
        }
        match obj.get("on_error") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s == "fail" || s == "continue" => {}
            Some(_) => issues.add(
                &format!("{at}/on_error"),
                "„fail“ oder „continue“ erwartet.",
            ),
        }
        if let Some(r) = obj.get("retry") {
            if !r.is_null() {
                check_retry(r, &format!("{at}/retry"), issues);
            }
        }
        let action = text_field(obj, "action", &at, 64, true, issues);
        let params = match obj.get("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => {
                issues.add(&format!("{at}/params"), "Objekt erwartet.");
                Map::new()
            }
        };
        if Value::Object(params.clone()).to_string().len() > MAX_PARAMS_BYTES {
            issues.add(
                &format!("{at}/params"),
                format!("Zu groß (höchstens {} KiB).", MAX_PARAMS_BYTES / 1024),
            );
        }
        if let Some(action) = action {
            match catalog::action_spec(&action) {
                None => {
                    let known: Vec<&str> = catalog::actions().iter().map(|a| a.id).collect();
                    issues.add(
                        &format!("{at}/action"),
                        format!("Unbekannter Baustein „{action}“ (bekannt: {}).", known.join(", ")),
                    );
                }
                Some(spec) => {
                    check_params(spec, &params, &format!("{at}/params"), &scope, issues);
                }
            }
        }
        if let Some(id) = id {
            before.insert(id);
        }
    }
}

/// Alle Befunde zu einer Definition (leer = gueltig).
pub fn check(v: &Value) -> Vec<Issue> {
    let mut issues = Issues(Vec::new());
    let Some(obj) = v.as_object() else {
        issues.add("", "Die Definition muss ein JSON-Objekt sein.");
        return issues.0;
    };
    if v.to_string().len() > MAX_DEFINITION_BYTES {
        issues.add(
            "",
            format!(
                "Die Definition ist zu groß (höchstens {} KiB).",
                MAX_DEFINITION_BYTES / 1024
            ),
        );
        return issues.0;
    }
    check_unknown_keys(
        obj,
        &["schema", "name", "description", "trigger", "variables", "steps"],
        "",
        &mut issues,
    );
    match obj.get("schema") {
        Some(Value::String(s)) if s == SCHEMA_ID => {}
        _ => issues.add("/schema", format!("Muss „{SCHEMA_ID}“ sein.")),
    }
    if let Some(name) = text_field(obj, "name", "", MAX_NAME_CHARS, true, &mut issues) {
        if name.trim().is_empty() {
            issues.add("/name", "Der Name darf nicht leer sein.");
        }
    }
    text_field(obj, "description", "", MAX_TEXT_CHARS, false, &mut issues);
    let trigger = check_trigger(obj.get("trigger"), &mut issues);
    let vars = check_variables(obj.get("variables"), &mut issues);
    check_steps(obj.get("steps"), trigger, &vars, &mut issues);
    issues.0
}

/// JSON-Wert -> Definition, oder alle Befunde.
pub fn parse_definition(v: &Value) -> Result<WorkflowDef, Vec<Issue>> {
    let issues = check(v);
    if !issues.is_empty() {
        return Err(issues);
    }
    serde_json::from_value::<WorkflowDef>(v.clone()).map_err(|e| {
        vec![Issue {
            path: String::new(),
            message: format!("Die Definition ließ sich nicht lesen: {e}"),
        }]
    })
}

/// JSON-Text -> Definition. Ungueltiges JSON ist ein Befund mit Zeile und Spalte.
pub fn parse_definition_str(text: &str) -> Result<WorkflowDef, Vec<Issue>> {
    let value: Value = serde_json::from_str(text).map_err(|e| {
        vec![Issue {
            path: String::new(),
            message: format!(
                "Kein gültiges JSON (Zeile {}, Spalte {}).",
                e.line(),
                e.column()
            ),
        }]
    })?;
    parse_definition(&value)
}

#[cfg(test)]
mod tests;
