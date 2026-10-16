//! API-Schluessel der Sprachmodell-Verbindungen verschluesselt ablegen.
//!
//! Bis 0.21.x standen sie im Klartext in `settings_store.json`. Jetzt
//! wandern sie an der Grenze zur Datei in den DPAPI-Geheimnisspeicher
//! (`managers::calendar::secret`, Integrationsraum: `int-llm-<id>-apikey.bin`)
//! und die Datei bekommt einen Leerwert. Beim Laden werden sie wieder
//! eingesetzt -- fuer alle, die `post_process_api_keys` lesen, aendert sich
//! nichts.
//!
//! Scheitert das Verschluesseln (kein Windows, Ordner nicht bereit), meldet
//! `seal` einen Fehler: der neue Schluessel wird nicht gespeichert -- nie im
//! Klartext --, die Datei behaelt fuer ihn ihren bisherigen Wert, ein alter
//! verschluesselter Schluessel bleibt unberuehrt (`settings::try_write_settings`
//! laesst das Speichern sichtbar scheitern). Alte Klartext-Dateien zaehlt
//! weiter das Schild.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::managers::calendar::secret::{self, SecretRef};
use crate::settings::SecretMap;

/// Entschluesselte Schluessel dieses Laufs: spart DPAPI bei jedem Lesen der
/// Einstellungen und erkennt, ob sich ein Schluessel geaendert hat.
static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn secret_ref(id: &str) -> SecretRef {
    SecretRef::integration(&format!("llm-{id}"), "apikey")
}

fn with_cache<T>(f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
    let mut guard = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Legt den Geheimnisordner fest, falls das noch niemand getan hat.
pub fn ensure_dir(app: &tauri::AppHandle) {
    if !secret::dir_initialized() {
        if let Ok(dir) = secret::secrets_dir_for(app) {
            secret::init_dir(dir);
        }
    }
}

/// `seal` ist fuer mindestens einen Schluessel gescheitert.
#[derive(Debug)]
pub struct SealFailure {
    /// Wie `seal` es sonst liefert; die gescheiterten Eintraege sind leer --
    /// der Aufrufer setzt dort den bisherigen Dateiwert ein.
    pub sealed: SecretMap,
    pub failed_ids: Vec<String>,
    pub message: String,
}

/// Fuer die Datei: Schluessel in den Geheimnisspeicher, Leerwert zurueck.
/// Ein geleerter Schluessel loescht auch das Geheimnis. Laesst sich ein
/// Schluessel nicht verschluesseln, kommt [`SealFailure`] -- nie der Klartext,
/// und das bisherige Geheimnis dieser Verbindung bleibt unberuehrt.
pub fn seal(map: &SecretMap) -> Result<SecretMap, SealFailure> {
    seal_with(map, |id, key| secret::secret_put_ref(&secret_ref(id), key))
}

fn seal_with(
    map: &SecretMap,
    put: impl Fn(&str, &[u8]) -> Result<(), String>,
) -> Result<SecretMap, SealFailure> {
    let mut out = map.clone();
    let mut failed = Vec::new();
    for (id, value) in out.iter_mut() {
        let key = value.trim().to_string();
        if key.is_empty() {
            // Nur ein vorher gespeicherter Schluessel hat ein Geheimnis.
            let had = with_cache(|c| c.remove(id).is_some_and(|v| !v.is_empty()));
            if had {
                secret::secret_delete_ref(&secret_ref(id));
            }
            continue;
        }
        let unchanged = with_cache(|c| c.get(id) == Some(&key));
        if !unchanged {
            if let Err(e) = put(id, key.as_bytes()) {
                failed.push((id.clone(), e));
                value.clear();
                continue;
            }
        }
        with_cache(|c| c.insert(id.clone(), key));
        value.clear();
    }
    if failed.is_empty() {
        return Ok(out);
    }
    let message = format!(
        "API-Schluessel nicht verschluesselbar, nicht gespeichert ({})",
        failed
            .iter()
            .map(|(id, e)| format!("{id}: {e}"))
            .collect::<Vec<_>>()
            .join("; ")
    );
    Err(SealFailure {
        sealed: out,
        failed_ids: failed.into_iter().map(|(id, _)| id).collect(),
        message,
    })
}

/// Nach dem Laden: Leerwerte aus dem Geheimnisspeicher fuellen.
pub fn unseal(map: &mut SecretMap) {
    for (id, value) in map.iter_mut() {
        if !value.is_empty() {
            continue;
        }
        if let Some(key) = with_cache(|c| c.get(id).cloned()) {
            *value = key;
            continue;
        }
        match secret::secret_get_ref(&secret_ref(id)) {
            Ok(Some(bytes)) => {
                if let Ok(key) = String::from_utf8(bytes.to_vec()) {
                    with_cache(|c| c.insert(id.clone(), key.clone()));
                    *value = key;
                }
            }
            // Kein Schluessel: als leer merken, damit nicht jedes Lesen der
            // Einstellungen die Platte fragt.
            Ok(None) => {
                with_cache(|c| c.insert(id.clone(), String::new()));
            }
            // Nicht entschluesselbar oder Ordner nicht bereit: leer lassen,
            // beim naechsten Lesen erneut versuchen.
            Err(_) => {}
        }
    }
}

/// Wie viele Schluessel liegen in der Datei noch im Klartext? (Schild)
pub fn plaintext_count(stored: &serde_json::Value) -> usize {
    stored
        .get("post_process_api_keys")
        .and_then(|m| m.as_object())
        .map(|m| {
            m.values()
                .filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
                .count()
        })
        .unwrap_or(0)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> SecretMap {
        serde_json::from_value(serde_json::json!(pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>()))
        .unwrap()
    }

    /// Versiegeln legt den Schluessel verschluesselt ab und hinterlaesst einen
    /// Leerwert; Entsiegeln (auch ohne Zwischenspeicher, wie nach einem
    /// Neustart) holt ihn zurueck; Leeren loescht das Geheimnis.
    #[test]
    fn keys_round_trip_through_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        secret::init_dir(dir.path().to_path_buf());
        let id = "mistral-test1";
        let sealed = seal(&map(&[(id, "sk-geheim-123"), ("openai", "")])).unwrap();
        assert_eq!(sealed.get(id).map(String::as_str), Some(""));
        let file = dir.path().join(format!("int-llm-{id}-apikey.bin"));
        assert!(file.is_file());
        let raw = std::fs::read(&file).unwrap();
        assert!(
            !raw.windows(13).any(|w| w == b"sk-geheim-123"),
            "Klartext in der Datei"
        );

        *CACHE.lock().unwrap() = None; // wie nach einem Neustart
        let mut loaded = sealed.clone();
        unseal(&mut loaded);
        assert_eq!(loaded.get(id).map(String::as_str), Some("sk-geheim-123"));
        assert_eq!(loaded.get("openai").map(String::as_str), Some(""));

        let cleared = seal(&map(&[(id, "")])).unwrap();
        assert_eq!(cleared.get(id).map(String::as_str), Some(""));
        assert!(
            !file.exists(),
            "geleerter Schluessel muss das Geheimnis loeschen"
        );
    }

    /// Scheitert das Verschluesseln, gibt es einen Fehler -- und auch der
    /// Ersatzwert fuer die Datei enthaelt keinen Klartext.
    #[test]
    fn failed_sealing_never_keeps_plaintext() {
        let id = "sealfail-test1";
        let input = map(&[(id, "sk-geheim-456")]);
        let err = seal_with(&input, |_, _| Err("kaputt".into())).unwrap_err();
        let msg = &err.message;
        assert!(msg.contains(id) && !msg.contains("sk-geheim-456"), "{msg}");
        assert_eq!(err.failed_ids, vec![id.to_string()]);
        assert_eq!(err.sealed.get(id).map(String::as_str), Some(""));
        assert!(with_cache(|c| !c.contains_key(id)));
    }

    #[test]
    fn plaintext_keys_in_the_file_are_counted() {
        let v = serde_json::json!({ "post_process_api_keys": { "a": "sk-1", "b": "", "c": "  " } });
        assert_eq!(plaintext_count(&v), 1);
    }
}
