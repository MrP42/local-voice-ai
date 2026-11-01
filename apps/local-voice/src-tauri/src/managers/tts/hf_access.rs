//! Zugang zu gesperrten Hugging-Face-Modellen (z. B. `phonon-40m-de`) ohne Umgebungsvariable und
//! ohne Kommandozeile: die Oberfläche übernimmt einen Lese-Zugang aus der Zwischenablage, dieses
//! Modul legt ihn ab und liefert ihn dem Download.
//!
//! ## Reihenfolge beim Lesen
//!
//! 1. Der in der App abgelegte Zugang (Windows: Tresor/DPAPI) – ihn hat der Nutzer ausdrücklich
//!    übernommen; ein veraltetes `HF_TOKEN` darf ihn nicht verdecken.
//! 2. `HF_TOKEN` bzw. die Token-Datei von Hugging Face ([`phonon_models::hf_token`]) – wer schon
//!    `hf auth login` benutzt, muss nichts tun.
//!
//! ## Ablage
//!
//! * **Windows und macOS:** im Tresor der App (`int-tts-huggingface-token.bin`; Windows DPAPI,
//!   macOS Schlüssel im Schlüsselbund, lva#134), nie in den Einstellungen.
//! * **Linux:** nur nach ausdrücklicher Zustimmung in der Token-Datei von
//!   Hugging Face (`$HF_HOME/token`, sonst `~/.cache/huggingface/token`, Rechte 0600) – genau dort,
//!   wo `hf auth login` ihn ablegt. Eine vorhandene Datei wird nie überschrieben.
//!
//! Der Wert verlässt dieses Modul nur als [`Secret`] Richtung `https://huggingface.co`; die
//! Oberfläche bekommt ausschließlich [`HfAccessStatus`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use specta::Type;

use super::phonon_models;
use crate::managers::calendar::secret::{self, SecretRef, SecretStatus};
use crate::catalog::{self, Purpose};
use crate::managers::model::download::{Origin, Secret};

/// Kürzere Werte sind sicher kein Zugang (Hugging Face: `hf_` + 34 Zeichen).
const MIN_LEN: usize = 20;
const MAX_LEN: usize = 256;

fn token_ref() -> SecretRef {
    SecretRef::integration("tts-huggingface", "token")
}

/// Woher der Zugang kommt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfAccessSource {
    /// `HF_TOKEN` oder die Token-Datei von Hugging Face.
    System,
    /// Im Tresor der App.
    App,
}

/// Zustand für die Oberfläche; nie der Wert selbst.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct HfAccessStatus {
    pub present: bool,
    pub source: Option<HfAccessSource>,
    /// Ein abgelegter Zugang ist nicht lesbar (anderes Konto, beschädigt).
    pub problem: Option<String>,
}

/// Prüft die Form eines Zugangs aus der Zwischenablage und schneidet Leerraum ab.
/// Abgelehnt: zu kurz/lang, Leer- oder Steuerzeichen, nicht `hf_…`.
pub fn normalize(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    let ok = t.len() >= MIN_LEN
        && t.len() <= MAX_LEN
        && t.starts_with("hf_")
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(t.to_string())
    } else {
        Err("In der Zwischenablage liegt kein Hugging-Face-Zugang (er beginnt mit „hf_“). Zugang auf huggingface.co kopieren und erneut übernehmen.".to_string())
    }
}

/// Token-Datei von Hugging Face (gleiche Auflösung wie [`phonon_models::hf_token`]).
fn hf_token_file(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |k: &str| {
        env(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    if let Some(home) = non_empty("HF_HOME") {
        return Some(PathBuf::from(home).join("token"));
    }
    if let Some(cache) = non_empty("XDG_CACHE_HOME") {
        return Some(PathBuf::from(cache).join("huggingface").join("token"));
    }
    let home = non_empty("HOME").or_else(|| non_empty("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cache").join("huggingface").join("token"))
}

/// Zugang für den Download: erst der Tresor der App, dann System (`HF_TOKEN`/Token-Datei).
pub(crate) fn load(
    secrets_dir: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Secret> {
    vault_token(secrets_dir).or_else(|| phonon_models::hf_token(env))
}

fn vault_token(secrets_dir: Option<&Path>) -> Option<Secret> {
    let bytes = secret::get_ref_in(secrets_dir?, &token_ref()).ok()??;
    let text = std::str::from_utf8(&bytes[..]).ok()?;
    Secret::new(text.trim())
}

pub fn status(secrets_dir: Option<&Path>, env: &dyn Fn(&str) -> Option<String>) -> HfAccessStatus {
    let vault = secrets_dir.map(|dir| secret::status_ref_in(dir, &token_ref()));
    if matches!(vault, Some(SecretStatus::Present)) {
        return HfAccessStatus {
            present: true,
            source: Some(HfAccessSource::App),
            problem: None,
        };
    }
    if phonon_models::hf_token(env).is_some() {
        return HfAccessStatus {
            present: true,
            source: Some(HfAccessSource::System),
            problem: None,
        };
    }
    HfAccessStatus {
        present: false,
        source: None,
        problem: match vault {
            Some(SecretStatus::Broken(why)) => Some(why),
            _ => None,
        },
    }
}

/// Zugang ablegen (siehe Modulkopf). Ungültige Werte schreiben nichts.
pub fn save(
    secrets_dir: &Path,
    raw: &str,
    allow_file: bool,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<HfAccessStatus, String> {
    let token = normalize(raw)?;
    if secret::VAULT_SUPPORTED {
        std::fs::create_dir_all(secrets_dir)
            .map_err(|_| "Der Ordner für Geheimnisse lässt sich nicht anlegen.".to_string())?;
        secret::put_ref_in(secrets_dir, &token_ref(), token.as_bytes())?;
    } else {
        // Ohne Tresor nur nach ausdrücklicher Zustimmung zum Klartext-Hinweis (Codex 31.10.2026).
        if !allow_file {
            return Err("Ohne Schlüsselbund liegt der Zugang als Datei im Klartext. Bitte den Hinweis bestätigen.".to_string());
        }
        let path = hf_token_file(env)
            .ok_or_else(|| "Der Ordner von Hugging Face ist nicht bestimmbar.".to_string())?;
        write_token_file(&path, &token)?;
    }
    Ok(status(Some(secrets_dir), env))
}

/// Nur den in der App abgelegten Zugang entfernen (die Token-Datei von Hugging Face gehört dem
/// Nutzer bzw. `hf auth` und bleibt).
pub fn delete(secrets_dir: &Path) {
    secret::delete_ref_in(secrets_dir, &token_ref());
}

/// Ergebnis der Zugangsprüfung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HfAccessCheck {
    /// Hugging Face liefert das Modell aus.
    Granted,
    /// Kein Zugang abgelegt.
    Missing,
    /// Zugang abgelehnt: Bedingungen noch nicht angenommen oder Zugang ungültig.
    Denied,
    /// Netz oder Server: später erneut prüfen.
    Unreachable,
}

/// Erste Datei eines gesperrten Modells im Katalog, nur wenn sie auf `https://huggingface.co` liegt.
pub(crate) fn probe_url(model_id: &str) -> Option<reqwest::Url> {
    let entry = catalog::tts_entries(Purpose::PhononModel)
        .into_iter()
        .find(|e| e.id == model_id && e.gated)?;
    let url = reqwest::Url::parse(&entry.files.first()?.url).ok()?;
    (Origin::of(&url) == Some(Origin::huggingface())).then_some(url)
}

/// Ordnet die Antwort von Hugging Face ein (ohne Weiterleitungen zu folgen: 2xx/3xx = freigegeben).
/// 404 heißt bei öffentlichen, gesperrten Modellen „Adresse stimmt nicht“, nicht „abgelehnt“.
pub fn classify(status: reqwest::StatusCode) -> HfAccessCheck {
    if status.is_success() || status.is_redirection() {
        HfAccessCheck::Granted
    } else if matches!(status.as_u16(), 401 | 403) {
        HfAccessCheck::Denied
    } else {
        HfAccessCheck::Unreachable
    }
}

/// Prüft mit einer HEAD-Anfrage an die erste Datei des Modells, ob der Zugang wirkt. Das Token geht
/// nur an `https://huggingface.co`; Weiterleitungen (CDN) werden nicht verfolgt.
pub(crate) async fn verify(token: Option<Secret>, model_id: &str) -> HfAccessCheck {
    let Some(token) = token else {
        return HfAccessCheck::Missing;
    };
    let (Some(url), Some(bearer)) = (probe_url(model_id), token.bearer()) else {
        return HfAccessCheck::Unreachable;
    };
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return HfAccessCheck::Unreachable;
    };
    match client
        .head(url)
        .header(reqwest::header::AUTHORIZATION, bearer)
        .send()
        .await
    {
        Ok(resp) => classify(resp.status()),
        Err(_) => HfAccessCheck::Unreachable,
    }
}

fn write_token_file(path: &Path, token: &str) -> Result<(), String> {
    if path.exists() {
        return Err("Es gibt schon eine Token-Datei von Hugging Face; sie bleibt unverändert. Mit „hf auth login“ lässt sie sich ersetzen.".to_string());
    }
    let dir = path
        .parent()
        .ok_or_else(|| "Der Ordner von Hugging Face ist nicht bestimmbar.".to_string())?;
    std::fs::create_dir_all(dir)
        .map_err(|_| "Der Ordner von Hugging Face lässt sich nicht anlegen.".to_string())?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut f = opts
        .open(path)
        .map_err(|_| "Die Token-Datei von Hugging Face lässt sich nicht schreiben.".to_string())?;
    let written = f
        .write_all(token.as_bytes())
        .and_then(|_| f.write_all(b"\n"))
        .and_then(|_| f.sync_all());
    if written.is_err() {
        // Keine halbe Datei zurücklassen: sie würde jede spätere Ablage blockieren.
        drop(f);
        let _ = std::fs::remove_file(path);
        return Err("Die Token-Datei von Hugging Face lässt sich nicht schreiben.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const GOOD: &str = "hf_abcdefghijklmnopqrstuvwxyz012345678";

    fn env_of(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn nur_echte_zugaenge_werden_angenommen() {
        assert_eq!(normalize(&format!("  {GOOD}\n")).unwrap(), GOOD);
        for bad in ["", "hf_kurz", "abc_defghijklmnopqrstuvwxyz0123", "hf_mit leerzeichen_abcdefghijk", "hf_ümlaut_abcdefghijklmnopqrs"] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn die_meldung_nennt_nie_den_inhalt_der_zwischenablage() {
        let geheim = "geheimes-passwort-1234567890";
        let err = normalize(geheim).unwrap_err();
        assert!(!err.contains(geheim));
    }

    #[test]
    fn ohne_irgendeinen_zugang_ist_nichts_da() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[("HOME", tmp.path().join("leer").display().to_string())]);
        let s = status(Some(tmp.path()), &env);
        assert!(!s.present);
        assert!(load(Some(tmp.path()), &env).is_none());
    }

    #[test]
    fn ein_systemzugang_wirkt_ohne_eigenen() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[("HF_TOKEN", GOOD.to_string())]);
        let s = status(Some(tmp.path()), &env);
        assert_eq!(s.source, Some(HfAccessSource::System));
        assert!(load(Some(tmp.path()), &env).is_some());
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn der_eigene_zugang_hat_vorrang_vor_hf_token() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[("HF_TOKEN", "hf_veraltetveraltetveraltet".to_string())]);
        save(tmp.path(), GOOD, false, &env).unwrap();
        assert_eq!(status(Some(tmp.path()), &env).source, Some(HfAccessSource::App));
        assert_eq!(
            load(Some(tmp.path()), &env).and_then(|s| s.bearer()),
            Secret::new(GOOD).and_then(|s| s.bearer())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ohne_tresor_landet_der_zugang_in_der_token_datei_mit_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let hf_home = tmp.path().join("hf");
        let env = env_of(&[("HF_HOME", hf_home.display().to_string())]);
        let s = save(tmp.path(), GOOD, true, &env).unwrap();
        assert!(s.present);
        assert_eq!(s.source, Some(HfAccessSource::System));
        let file = hf_home.join("token");
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), GOOD);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn eine_vorhandene_token_datei_wird_nie_ueberschrieben() {
        let tmp = tempfile::tempdir().unwrap();
        let hf_home = tmp.path().join("hf");
        std::fs::create_dir_all(&hf_home).unwrap();
        std::fs::write(hf_home.join("token"), "  \n").unwrap();
        let env = env_of(&[("HF_HOME", hf_home.display().to_string())]);
        assert!(save(tmp.path(), GOOD, true, &env).is_err());
        assert_eq!(std::fs::read_to_string(hf_home.join("token")).unwrap(), "  \n");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ohne_zustimmung_entsteht_keine_klartextdatei() {
        let tmp = tempfile::tempdir().unwrap();
        let hf_home = tmp.path().join("hf");
        let env = env_of(&[("HF_HOME", hf_home.display().to_string())]);
        assert!(save(tmp.path(), GOOD, false, &env).is_err());
        assert!(!hf_home.join("token").exists());
    }

    #[test]
    fn geprueft_wird_nur_gegen_huggingface_und_nur_gesperrte_modelle() {
        let url = probe_url("phonon-40m-de").expect("40m-de ist gesperrt");
        assert_eq!(url.host_str(), Some("huggingface.co"));
        assert_eq!(url.scheme(), "https");
        assert!(probe_url("phonon-31m-en").is_none(), "nicht gesperrt");
        assert!(probe_url("gibt-es-nicht").is_none());
    }

    #[test]
    fn antworten_werden_eingeordnet() {
        use reqwest::StatusCode as S;
        assert_eq!(classify(S::OK), HfAccessCheck::Granted);
        assert_eq!(classify(S::FOUND), HfAccessCheck::Granted);
        assert_eq!(classify(S::UNAUTHORIZED), HfAccessCheck::Denied);
        assert_eq!(classify(S::FORBIDDEN), HfAccessCheck::Denied);
        assert_eq!(classify(S::NOT_FOUND), HfAccessCheck::Unreachable);
        assert_eq!(classify(S::BAD_GATEWAY), HfAccessCheck::Unreachable);
    }

    #[test]
    fn ungueltige_werte_schreiben_nichts() {
        let tmp = tempfile::tempdir().unwrap();
        let hf_home = tmp.path().join("hf");
        let env = env_of(&[("HF_HOME", hf_home.display().to_string())]);
        assert!(save(tmp.path(), "kein-zugang", true, &env).is_err());
        assert!(!hf_home.join("token").exists());
        assert!(!status(Some(tmp.path()), &env).present);
    }
}
