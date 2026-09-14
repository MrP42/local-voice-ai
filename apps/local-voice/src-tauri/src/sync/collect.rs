//! Brücke zwischen Sync-Objekten und den lokalen Daten: Seiten aus
//! `projects/*` und die synchronisierte Teilmenge der Einstellungen. Baut
//! Objekte (Klartext-JSON) und wendet empfangene an. Kennt keinen Schlüssel
//! und keinen Server.

use crate::commands::pages;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tauri::AppHandle;

pub const COLLECTION_PAGE: &str = "page";
pub const COLLECTION_SETTINGS: &str = "settings";
pub const SETTINGS_OBJECT_ID: &str = "app";

/// Einstellungen, die zwischen Geräten gleich sein sollen. Pfade, Ports,
/// Tastenkürzel, Geräte- und Engine-Wahl bleiben gerätespezifisch.
pub const SYNCED_SETTINGS: &[&str] = &[
    "app_language",
    "theme",
    "custom_words",
    "tts_seed",
    "tts_idle_minutes",
    "tts_max_chars",
    "tts_translate_lang",
    "tts_volume",
    "tts_normalize",
    "tts_prewarm",
    "tts_enhance",
    "tts_enhance_strength",
    "tts_reference_auto_transcribe",
    "tts_speed",
    "tts_export_format",
    "tts_export_bitrate",
    "tts_context_menu",
    "tts_tag_favorites",
    "tts_tag_provider",
    "tts_tag_model",
];

/// Größte Klartextlänge, die der Hub trägt (Spec Abschnitt 4).
pub const MAX_PLAINTEXT: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct LocalObject {
    pub collection: &'static str,
    pub id: String,
    pub value: Value,
}

/// Kanonische Form: Schlüssel rekursiv sortiert, kompakt. Zwei Geräte mit
/// gleichem Inhalt kommen so auf denselben Hash.
pub fn canonical(value: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let sorted: BTreeMap<_, _> = m.iter().map(|(k, v)| (k.clone(), sort(v))).collect();
                Value::Object(sorted.into_iter().collect::<Map<_, _>>())
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(value)).unwrap_or_default()
}

pub fn hash(value: &Value) -> String {
    let digest = Sha256::digest(canonical(value).as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Alle lokalen Objekte: eine je Seite, eines für die Einstellungen.
pub fn collect(app: &AppHandle) -> Result<Vec<LocalObject>, String> {
    let mut out = Vec::new();
    let index = pages::load_index(app)?;
    for page in &index.pages {
        let state_raw = std::fs::read_to_string(pages::page_path(app, &page.id)?.join("state.json")).ok();
        let state = state_raw
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or(Value::Null);
        out.push(LocalObject {
            collection: COLLECTION_PAGE,
            id: page.id.clone(),
            value: json!({ "title": page.title, "state": state }),
        });
    }
    out.push(LocalObject {
        collection: COLLECTION_SETTINGS,
        id: SETTINGS_OBJECT_ID.to_string(),
        value: settings_subset(&serde_json::to_value(crate::settings::get_settings(app)).unwrap_or(Value::Null)),
    });
    Ok(out)
}

pub fn settings_subset(all: &Value) -> Value {
    let mut m = Map::new();
    if let Value::Object(src) = all {
        for k in SYNCED_SETTINGS {
            if let Some(v) = src.get(*k) {
                m.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(m)
}

/// Eine empfangene Seite anwenden: fehlt sie, entsteht sie am Ende der Liste;
/// sonst werden Titel und Arbeitsstand ersetzt. Die Reihenfolge bleibt
/// gerätelokal.
pub fn apply_page(app: &AppHandle, id: &str, value: &Value) -> Result<(), String> {
    let title = value.get("title").and_then(Value::as_str).unwrap_or("Seite").to_string();
    let state = value.get("state").cloned().unwrap_or(Value::Null);
    let dir = pages::page_path(app, id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Seitenordner: {e}"))?;
    if !state.is_null() {
        let raw = serde_json::to_string(&state).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("state.json"), raw).map_err(|e| format!("state.json: {e}"))?;
    }
    let mut index = pages::load_index(app)?;
    match index.pages.iter_mut().find(|p| p.id == id) {
        Some(p) => p.title = title,
        None => index.pages.push(pages::PageInfo { id: id.to_string(), title, ..Default::default() }),
    }
    pages::store_index(app, &index)
}

/// Tombstone: Seite aus dem Index nehmen, Ordner nach `projects/_geloescht/`
/// schieben — Dateien bleiben, der Nutzer räumt selbst auf.
pub fn tombstone_page(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut index = pages::load_index(app)?;
    let before = index.pages.len();
    index.pages.retain(|p| p.id != id);
    if index.pages.len() != before {
        pages::store_index(app, &index)?;
    }
    let dir = pages::page_path(app, id)?;
    if dir.is_dir() {
        let trash = pages::projects_root(app)?.join("_geloescht");
        std::fs::create_dir_all(&trash).map_err(|e| format!("Papierkorb: {e}"))?;
        let mut target = trash.join(id);
        if target.exists() {
            target = trash.join(format!("{id}-{}", chrono::Utc::now().timestamp_millis()));
        }
        std::fs::rename(&dir, &target).map_err(|e| format!("Seite verschieben: {e}"))?;
    }
    Ok(())
}

/// Konfliktkopie: der lokale Stand wird eine neue Seite, damit die
/// Server-Fassung das Original übernehmen kann, ohne dass etwas verloren geht.
pub fn conflict_copy(app: &AppHandle, local: &Value, device_name: &str) -> Result<String, String> {
    let title = local.get("title").and_then(Value::as_str).unwrap_or("Seite");
    let copy = pages::pages_create(app.clone(), format!("{title} (Konflikt von {device_name})"))?;
    if let Some(state) = local.get("state").filter(|s| !s.is_null()) {
        let raw = serde_json::to_string(state).map_err(|e| e.to_string())?;
        std::fs::write(pages::page_path(app, &copy.id)?.join("state.json"), raw)
            .map_err(|e| format!("state.json: {e}"))?;
    }
    Ok(copy.id)
}

/// Empfangene Einstellungen anwenden: nur die Allowlist, alles andere bleibt.
pub fn apply_settings(app: &AppHandle, value: &Value) -> Result<(), String> {
    let current = crate::settings::get_settings(app);
    let mut all = serde_json::to_value(&current).map_err(|e| e.to_string())?;
    let Value::Object(incoming) = value else {
        return Err("Einstellungen: kein Objekt".into());
    };
    if let Value::Object(dst) = &mut all {
        for k in SYNCED_SETTINGS {
            if let Some(v) = incoming.get(*k) {
                dst.insert((*k).to_string(), v.clone());
            }
        }
    }
    let merged: crate::settings::AppSettings =
        serde_json::from_value(all).map_err(|e| format!("Einstellungen unlesbar: {e}"))?;
    crate::settings::write_settings(app, merged);
    Ok(())
}

/// Vergleicht zwei Seiten-Objekte auf den Inhalt, den ein Mensch verliert:
/// Text, Übersetzung, Zusammenfassung, Titel. Reiter und URL zählen nicht.
pub fn pages_differ(a: &Value, b: &Value) -> bool {
    fn essence(v: &Value) -> Value {
        let s = v.get("state").cloned().unwrap_or(Value::Null);
        json!({
            "title": v.get("title"),
            "text": s.get("text"),
            "summary": s.get("summary"),
            "translation": s.get("translation"),
        })
    }
    canonical(&essence(a)) != canonical(&essence(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanonisch_ist_reihenfolgeunabhaengig() {
        let a = json!({"b": 1, "a": {"y": [1, {"k": 2, "j": 3}], "x": 0}});
        let b = json!({"a": {"x": 0, "y": [1, {"j": 3, "k": 2}]}, "b": 1});
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(hash(&a), hash(&b));
        assert_ne!(hash(&a), hash(&json!({"b": 2})));
    }

    #[test]
    fn einstellungs_teilmenge_laesst_pfade_weg() {
        let all = json!({"tts_seed": 42, "tts_fish_dir": "C:/fish", "theme": "dark", "tts_port": 8080, "bindings": {}});
        let sub = settings_subset(&all);
        assert_eq!(sub, json!({"tts_seed": 42, "theme": "dark"}));
    }

    #[test]
    fn seitenvergleich_ignoriert_reiter() {
        let a = json!({"title": "T", "state": {"text": "x", "tab": "original", "sourceUrl": ""}});
        let b = json!({"title": "T", "state": {"text": "x", "tab": "summary", "sourceUrl": "u"}});
        let c = json!({"title": "T", "state": {"text": "y", "tab": "original"}});
        assert!(!pages_differ(&a, &b));
        assert!(pages_differ(&a, &c));
        assert!(pages_differ(&a, &json!({"title": "U", "state": {"text": "x"}})));
    }
}
