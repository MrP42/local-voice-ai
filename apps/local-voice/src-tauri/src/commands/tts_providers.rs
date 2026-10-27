//! Tauri-Befehle der Sprachausgabe-Anbieter (Goal tts-provider, Paket P3a).
//!
//! Die Regeln stehen in `managers::tts::providers` (rein und dort getestet);
//! die Befehle hier sind dünne Hüllen: Einstellungen lesen, reine Funktion
//! anwenden, schreiben. Alle Fehler gehen als [`TtsError`] zurück — die
//! Oberfläche liest `kind` (z. B. `license_blocked`, `privacy_blocked`) statt
//! den Text zu raten.
//!
//! ## NUR für die Oberfläche
//!
//! **`tts_provider_set_enabled`, `tts_provider_acknowledge_license` und
//! `tts_provider_set_allow_remote` sind die EINZIGEN Wege, Opt-in,
//! Lizenz-Zustimmung und Fernzugriff zu setzen. Sie sind ein Klick des
//! Nutzers und dürfen NIE in die Agentenbrücke, in eine CLI-Automatik oder
//! einen anderen unbeaufsichtigten Weg aufgenommen werden** — sonst höbe ein
//! Agent die Lizenz- und Datenschutzsperre selbst auf. Auch
//! `tts_provider_set_config` kann sie nicht setzen (es neutralisiert die
//! Schalter in der Konfiguration), und das Einstellungsfeld `tts_providers`
//! hat bewusst keinen allgemeinen Setter.
//!
//! Die Agentenbrücke (Paket P3b) nutzt davon nur [`select_provider`] (Anbieter
//! und Stimme wählen: prüft `check_use`, setzt nie eine Freigabe) und die
//! lesenden Kernfunktionen ([`build_entries`]); die CLI kennt gar keinen
//! dieser Befehle.
//!
//! ## Ereignis
//!
//! `tts-provider-preview` ([`TtsProviderPreview`]): `started` →
//! `first_audio` (`ttfa_ms`) → `done` | `cancelled` | `error`
//! (`error_kind`, `message`); siehe `providers::preview`.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::managers::tts::engine;
use crate::managers::tts::providers::contract::ProviderTier;
use crate::managers::tts::providers::host::ProviderSnapshot;
use crate::managers::tts::providers::policy::{self, LicenseAck, UseSubject};
use crate::managers::tts::providers::{
    ProviderDescriptor, ProviderHealth, ProviderRegistry, TtsError, TtsProvidersSettings,
    VoiceDescriptor,
};
use crate::managers::tts::TtsManager;
use crate::settings::{self, AppSettings};

/// Frist der Gesundheitsprüfung je Anbieter (Anbieter laufen parallel).
pub(crate) const HEALTH_LIMIT: Duration = Duration::from_secs(4);
/// Frist der Stimmenliste. Die inneren Fristen der Anbieter (Phonon: Katalog,
/// Ollama: Probe) liegen UNTER diesen Grenzen, damit eine Teilliste bzw. ein
/// Zwischenstand noch ankommt, statt dass die äußere Frist alles verwirft.
pub(crate) const VOICES_LIMIT: Duration = Duration::from_secs(10);

/// Ein Anbieter in der Liste der Oberfläche.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TtsProviderEntry {
    pub descriptor: ProviderDescriptor,
    /// Ergebnis der Prüfung (mit Frist; ein hängender Anbieter ist „nicht
    /// verfügbar“ mit Grund).
    pub health: ProviderHealth,
    /// Opt-in erteilt (Anbieter ohne Opt-in: immer `true`).
    pub enabled: bool,
    pub license_needs_ack: bool,
    /// Gültige Zustimmung zum AKTUELLEN Stand (Modell/Lizenz) liegt vor.
    pub license_acked: bool,
    /// Fernzugriff ausdrücklich erlaubt.
    pub allow_remote: bool,
    /// Die wirksame Adresse liegt außerhalb von Loopback.
    pub remote: bool,
    /// Aktueller Fingerabdruck der Lizenzlage — beim Bestätigen zurückgeben.
    pub fingerprint: String,
    /// Warum die Nutzung jetzt gesperrt ist (`None` = erlaubt).
    pub blocked: Option<TtsError>,
    /// Dieser Anbieter liest vor (`tts_engine`).
    pub selected: bool,
    /// Gewählte Stimme (leer = Vorgabe des Anbieters).
    pub voice: Option<String>,
    /// Stimmen, die im Vorlesen-Dropdown stehen (leer = alle). Siehe
    /// [`TtsProvidersSettings::visible_voices`]; gesetzt über `tts_provider_set_visible_voices`.
    pub visible_voices: Vec<String>,
}

fn manager(app: &AppHandle) -> Arc<TtsManager> {
    app.state::<Arc<TtsManager>>().inner().clone()
}

/// Liste aus Momentaufnahme und Prüfergebnissen (rein, testbar).
pub fn build_entries(
    snapshot: &ProviderSnapshot,
    health: Vec<(String, ProviderHealth)>,
) -> Vec<TtsProviderEntry> {
    let mut health: std::collections::HashMap<String, ProviderHealth> =
        health.into_iter().collect();
    // Je Instanz EIN Zugriff: Beschreibung und Freigabezustand (mit dem Gegenstand
    // aus der Instanz selbst) stammen aus demselben Stand der Registry.
    snapshot
        .registry
        .list()
        .into_iter()
        .map(|provider| {
            let (descriptor, state) = snapshot.use_state_of(provider.as_ref());
            let health = health
                .remove(&descriptor.id)
                .unwrap_or_else(|| ProviderHealth::unavailable("Nicht geprüft".to_string(), None));
            TtsProviderEntry {
                selected: snapshot.selected == descriptor.id,
                voice: snapshot.voice_of(&descriptor.id),
                visible_voices: snapshot
                    .settings
                    .visible_voices
                    .get(&descriptor.id)
                    .cloned()
                    .unwrap_or_default(),
                enabled: state.enabled,
                license_needs_ack: state.license_needs_ack,
                license_acked: state.license_acked,
                allow_remote: state.allow_remote,
                remote: state.remote,
                fingerprint: state.fingerprint,
                blocked: state.blocked,
                descriptor,
                health,
            }
        })
        .collect()
}

fn known(registry: &ProviderRegistry, id: &str) -> Result<ProviderDescriptor, TtsError> {
    registry
        .get(id)
        .map(|p| p.descriptor())
        .ok_or_else(|| TtsError::bad_request(format!("Unbekannter Anbieter '{id}'")))
}

/// Opt-in setzen. Nur Anbieter mit `opt_in` haben einen (rein, testbar).
pub fn set_enabled_in(
    providers: &mut TtsProvidersSettings,
    registry: &ProviderRegistry,
    id: &str,
    enabled: bool,
) -> Result<(), TtsError> {
    let desc = known(registry, id)?;
    if !desc.opt_in {
        return Err(TtsError::bad_request(format!(
            "{} braucht keine Freischaltung",
            desc.label
        )));
    }
    providers.set_enabled(id, enabled);
    Ok(())
}

/// Lizenz bestätigen — nur zum AKTUELLEN Fingerabdruck: hat sich Modell oder
/// Lizenzangabe zwischen Anzeige und Klick geändert, wird abgelehnt (rein,
/// testbar). `now`: Zeitstempel (RFC 3339).
pub fn acknowledge_in(
    providers: &mut TtsProvidersSettings,
    registry: &ProviderRegistry,
    subject: &UseSubject,
    id: &str,
    fingerprint: &str,
    now: &str,
) -> Result<LicenseAck, TtsError> {
    let desc = known(registry, id)?;
    if !policy::license_needs_ack(&desc) {
        return Err(TtsError::bad_request(format!(
            "{} braucht keine Lizenz-Bestätigung",
            desc.label
        )));
    }
    let current = policy::license_fingerprint(&desc, subject);
    if fingerprint != current {
        return Err(TtsError::bad_request(
            "Modell oder Lizenzangaben haben sich geändert. Die Lizenz bitte erneut ansehen und bestätigen.",
        ));
    }
    providers.record_ack(id, &current, now);
    Ok(providers
        .license_ack(id)
        .cloned()
        .expect("die Zustimmung wurde gerade eingetragen"))
}

/// Fernzugriff erlauben oder zurücknehmen (rein, testbar).
pub fn set_allow_remote_in(
    providers: &mut TtsProvidersSettings,
    registry: &ProviderRegistry,
    id: &str,
    allow: bool,
) -> Result<(), TtsError> {
    known(registry, id)?;
    providers.set_remote_allowed(id, allow);
    Ok(())
}

/// Obergrenze für die Zahl der gemerkten Stimmen je Anbieter (ein Katalog hat Dutzende, nie Tausende).
const MAX_VISIBLE_VOICES: usize = 1000;

/// Welche Stimmen eines Anbieters im Vorlesen-Dropdown stehen (rein, testbar). Die Kennungen werden
/// bereinigt (getrimmt, ohne Leere, Steuerzeichen und Doppelte); eine leere Liste löscht die Auswahl
/// (= alle sichtbar). Ändert keine Freigabe und braucht keinen Neuaufbau der Anbieter.
pub fn set_visible_voices_in(
    providers: &mut TtsProvidersSettings,
    registry: &ProviderRegistry,
    id: &str,
    voices: Vec<String>,
) -> Result<(), TtsError> {
    known(registry, id)?;
    let mut clean: Vec<String> = Vec::new();
    for voice in voices {
        let voice = voice.trim();
        if voice.is_empty() || voice.len() > 256 || voice.chars().any(char::is_control) {
            continue;
        }
        if !clean.iter().any(|v| v == voice) {
            clean.push(voice.to_string());
        }
        if clean.len() >= MAX_VISIBLE_VOICES {
            break;
        }
    }
    if clean.is_empty() {
        providers.visible_voices.remove(id);
    } else {
        providers.visible_voices.insert(id.to_string(), clean);
    }
    Ok(())
}

/// Eine Stimme eines Anbieters für das Vorlesen-Dropdown.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ProviderVoiceOption {
    pub provider_id: String,
    pub provider_label: String,
    pub voice: VoiceDescriptor,
}

/// Bietet der Anbieter Stimmen im Dropdown an? Fish und Piper haben ihre eigenen Einträge dort;
/// Mock und gesperrte Stufen gehören nie in die Auswahl.
pub fn offers_dropdown_voices(desc: &ProviderDescriptor) -> bool {
    !matches!(desc.id.as_str(), policy::FISH_ID | policy::PIPER_ID)
        && matches!(
            desc.tier,
            ProviderTier::Supported | ProviderTier::Experimental
        )
}

/// Die Stimmen für das Vorlesen-Dropdown: je freigegebenem Anbieter (`offers` und Guardrails, gesperrte
/// Anbieter fehlen) seine Stimmenliste, gefiltert nach den gewählten (`visible_voice`: Standardstimme
/// und gerade gewählte Stimme bleiben immer). Ein Anbieter, dessen Liste scheitert oder zu lange
/// braucht, fehlt einfach — das Dropdown darf nie am Anbieter hängen.
pub async fn voice_options(
    snapshot: &ProviderSnapshot,
    offers: impl Fn(&ProviderDescriptor) -> bool,
) -> Vec<ProviderVoiceOption> {
    let jobs = snapshot.registry.list().into_iter().filter_map(|provider| {
        let desc = provider.descriptor();
        if !offers(&desc) || snapshot.checked(&desc.id).is_err() {
            return None;
        }
        let visible = snapshot.settings.visible_voices.get(&desc.id).cloned();
        let selected = snapshot
            .voice_of(&desc.id)
            .or_else(|| snapshot.settings.voices.get(&desc.id).cloned());
        Some(async move {
            let listed = tokio::time::timeout(VOICES_LIMIT, provider.list_voices(None)).await;
            let voices = match listed {
                Ok(Ok(voices)) => voices,
                _ => Vec::new(),
            };
            voices
                .into_iter()
                .filter(|v| policy::visible_voice(visible.as_ref(), selected.as_deref(), &v.id))
                .map(|voice| ProviderVoiceOption {
                    provider_id: desc.id.clone(),
                    provider_label: desc.label.clone(),
                    voice,
                })
                .collect::<Vec<_>>()
        })
    });
    futures_util::future::join_all(jobs)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Anbieter und Stimme wählen: prüft die Guardrails und setzt `tts_engine`
/// (`fish`/`piper` bleiben ihre alten Werte, sonst `provider:<id>`) sowie die
/// Stimme am passenden Ort (`tts_voice`, `tts_piper_voice` oder
/// `tts_providers.voices`). Bei einem Fehler bleibt `settings` unverändert
/// (rein, testbar).
///
/// Geprüft wird gegen den Stand von `settings` — den Stand IM MOMENT DES
/// SCHREIBENS (der Aufrufer liest ihn unter [`SETTINGS_LOCK`]) —, mit dem
/// Gegenstand (Modell, Adresse) NEU aufgelöst aus genau diesen Einstellungen.
/// `snapshot` liefert nur die Registry (welche Anbieter es gibt). Wurde
/// zwischen der Momentaufnahme und dem Schreiben eine Freigabe widerrufen oder
/// das Modell gewechselt, lehnt die Wahl ab.
pub fn select_in(
    settings: &mut AppSettings,
    snapshot: &ProviderSnapshot,
    id: &str,
    voice: Option<String>,
) -> Result<(), TtsError> {
    select_in_with_env(settings, snapshot, id, voice, &policy::process_env)
}

/// Wie [`select_in`], mit eigener Umgebung (Tests).
pub fn select_in_with_env(
    settings: &mut AppSettings,
    snapshot: &ProviderSnapshot,
    id: &str,
    voice: Option<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), TtsError> {
    let desc = snapshot.descriptor(id)?;
    // Mit demselben Stand "installiert" wie die Momentaufnahme: sonst stuende hier ein anderes
    // Modell als in der Instanz, und die Lizenz-Zustimmung (Fingerabdruck) passte nie.
    let subject = policy::resolve_provider_configs_with(
        &settings.tts_providers,
        env,
        &policy::ProviderOverrides::default(),
        snapshot.resolved.installed(),
    )
    .subject(id);
    policy::check_use(&desc, &subject, &settings.tts_providers)?;
    let voice = voice
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    settings.tts_engine = engine::engine_setting_for(id);
    match id {
        policy::FISH_ID => settings.tts_voice = voice,
        policy::PIPER_ID => settings.tts_piper_voice = voice,
        other => match voice {
            Some(v) => {
                settings.tts_providers.voices.insert(other.to_string(), v);
            }
            None => {
                settings.tts_providers.voices.remove(other);
            }
        },
    }
    Ok(())
}

// Schreibzugriffe auf die Einstellungen der Anbieter nacheinander: zwei
// Befehle (z. B. Opt-in und Bestätigung kurz hintereinander) dürfen sich
// nicht gegenseitig überschreiben (Lesen–Ändern–Schreiben). Dieselbe Sperre
// schützt den Abgleich des gemeinsamen Stands der Freigaben mit der Datei.
use crate::managers::tts::providers::host::SETTINGS_LOCK;

/// Lesen–Ändern–Schreiben unter der Sperre. Nach dem Schreiben — noch unter der
/// Sperre — gilt der neue Stand der Freigaben sofort für jede laufende Nutzung
/// (Engine, Probestimme, Probe); danach baut `providers_changed` Registry und
/// Engine neu und beendet eine laufende Probestimme, deren Anbieter jetzt
/// gesperrt ist. Ein Widerruf wirkt damit auch auf laufende Aufträge.
fn mutate_settings<T>(
    app: &AppHandle,
    change: impl FnOnce(&mut AppSettings) -> Result<T, TtsError>,
) -> Result<T, TtsError> {
    let out = {
        let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut current = settings::get_settings(app);
        let out = change(&mut current)?;
        let providers = current.tts_providers.clone();
        settings::write_settings(app, current);
        if let Some(tts) = app.try_state::<Arc<TtsManager>>() {
            tts.live_approvals().set(providers);
        }
        out
    };
    if let Some(tts) = app.try_state::<Arc<TtsManager>>() {
        tts.providers_changed();
    }
    Ok(out)
}

// ---- Befehle ----------------------------------------------------------------------------

/// Alle Anbieter mit Beschreibung, Gesundheit (parallel, mit Frist je
/// Anbieter), Freigabestand und Auswahl. Dauert höchstens etwa die Frist des
/// langsamsten Anbieters, nie die Summe.
#[tauri::command]
#[specta::specta]
pub async fn tts_providers_list(app: AppHandle) -> Result<Vec<TtsProviderEntry>, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    let health = snapshot.registry.health_all(HEALTH_LIMIT).await;
    Ok(build_entries(&snapshot, health))
}

/// Gesundheit EINES Anbieters (mit Frist).
#[tauri::command]
#[specta::specta]
pub async fn tts_provider_health(app: AppHandle, id: String) -> Result<ProviderHealth, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    snapshot.registry.health(&id, HEALTH_LIMIT).await
}

/// Stimmen eines Anbieters, optional auf eine Sprache eingegrenzt. Gilt als
/// Nutzung: gesperrte Anbieter (kein Opt-in, keine Zustimmung, Fernzugriff
/// nicht erlaubt) antworten mit dem Sperrgrund.
#[tauri::command]
#[specta::specta]
pub async fn tts_provider_voices(
    app: AppHandle,
    id: String,
    language: Option<String>,
) -> Result<Vec<VoiceDescriptor>, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    snapshot.checked(&id)?;
    let provider = snapshot
        .registry
        .get(&id)
        .ok_or_else(|| TtsError::bad_request(format!("Unbekannter Anbieter '{id}'")))?;
    match tokio::time::timeout(VOICES_LIMIT, provider.list_voices(language.as_deref())).await {
        Ok(result) => result,
        Err(_) => Err(TtsError::timeout(format!(
            "Die Stimmenliste kam nicht innerhalb von {} s",
            VOICES_LIMIT.as_secs()
        ))),
    }
}

/// Gespeicherte Konfiguration eines Anbieters als JSON-Text (Felder wie in
/// `PhononConfig`/`OpenAiSpeechConfig`/`OllamaSpeechConfig`). Fish, Piper:
/// `bad_request` (keine Konfiguration).
#[tauri::command]
#[specta::specta]
pub fn tts_provider_get_config(app: AppHandle, id: String) -> Result<String, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    policy::config_json(&snapshot.settings, &id)
}

/// Konfiguration eines Anbieters speichern. Der Adapter prüft vorher
/// (`validate()`: Pfade, Adresse, Bereiche); ein Fehler kommt als
/// `invalid_config` mit lesbarem Grund zurück und ändert nichts. Die Schalter
/// `allow_remote` in der Konfiguration werden NICHT übernommen (siehe
/// Moduldoku); eine entfernte Adresse lässt sich speichern, benutzen darf man
/// sie erst nach `tts_provider_set_allow_remote`.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_set_config(app: AppHandle, id: String, json: String) -> Result<(), TtsError> {
    mutate_settings(&app, |s| {
        policy::apply_config_json(&mut s.tts_providers, &id, &json)
    })
}

/// Einen Modellordner ansehen: liegen f32- und/oder q8-Gewichte vor? Die
/// Einstellungsmaske schlägt damit `q8` vor, wenn nur `model.q8.gguf` da ist (der
/// f32-Lauf scheitert dort am Fehlen der Gewichte). Nur Dateisystem, kein Prozess.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_inspect_model_folder(
    path: String,
) -> crate::managers::tts::providers::phonon::ModelFolderInfo {
    crate::managers::tts::providers::phonon::inspect_model_folder(&path)
}

/// Opt-in eines Anbieters setzen oder zurücknehmen. NUR OBERFLÄCHE (siehe Moduldoku).
#[tauri::command]
#[specta::specta]
pub fn tts_provider_set_enabled(app: AppHandle, id: String, enabled: bool) -> Result<(), TtsError> {
    let registry = manager(&app).provider_snapshot().registry;
    mutate_settings(&app, |s| {
        set_enabled_in(&mut s.tts_providers, &registry, &id, enabled)
    })
}

/// Die Lizenz eines Anbieters bestätigen. `fingerprint` ist der Wert aus
/// [`TtsProviderEntry::fingerprint`], der dem Nutzer zur Bestätigung gezeigt
/// wurde; hat sich der Stand seitdem geändert, wird abgelehnt. Die Bestätigung
/// gilt nur für genau dieses Modell/diese Lizenzangabe. NUR OBERFLÄCHE.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_acknowledge_license(
    app: AppHandle,
    id: String,
    fingerprint: String,
) -> Result<LicenseAck, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    let subject = snapshot.subject(&id);
    let now = chrono::Utc::now().to_rfc3339();
    mutate_settings(&app, |s| {
        acknowledge_in(
            &mut s.tts_providers,
            &snapshot.registry,
            &subject,
            &id,
            &fingerprint,
            &now,
        )
    })
}

/// Fernzugriff (Adresse außerhalb dieses Rechners) für einen Anbieter erlauben
/// oder zurücknehmen. Der Text der Anfrage verlässt dann das Gerät. NUR OBERFLÄCHE.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_set_allow_remote(
    app: AppHandle,
    id: String,
    allow: bool,
) -> Result<(), TtsError> {
    let registry = manager(&app).provider_snapshot().registry;
    mutate_settings(&app, |s| {
        set_allow_remote_in(&mut s.tts_providers, &registry, &id, allow)
    })
}

/// Welche Stimmen eines Anbieters im Vorlesen-Dropdown stehen. Leere Liste: alle. „Keine“ ist
/// `["default"]` (die Standardstimme steht immer da). Ändert weder Freigaben noch die laufende
/// Wiedergabe, darum ohne Neuaufbau der Anbieter (anders als [`mutate_settings`]).
#[tauri::command]
#[specta::specta]
pub fn tts_provider_set_visible_voices(
    app: AppHandle,
    id: String,
    voices: Vec<String>,
) -> Result<(), TtsError> {
    let registry = manager(&app).provider_snapshot().registry;
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut current = settings::get_settings(&app);
    set_visible_voices_in(&mut current.tts_providers, &registry, &id, voices)?;
    settings::write_settings(&app, current);
    Ok(())
}

/// Die Stimmen der freigegebenen Anbieter für das Vorlesen-Dropdown (nur die gewählten, siehe
/// [`voice_options`]). Anbieter ohne Freigabe, mit Sperre oder ohne Antwort fehlen.
#[tauri::command]
#[specta::specta]
pub async fn tts_provider_voice_options(
    app: AppHandle,
) -> Result<Vec<ProviderVoiceOption>, TtsError> {
    let snapshot = manager(&app).provider_snapshot();
    Ok(voice_options(&snapshot, offers_dropdown_voices).await)
}

/// Anbieter (und Stimme) fürs Vorlesen wählen. Prüft die Guardrails: ein
/// gesperrter Anbieter lässt sich nicht wählen (`license_blocked`/
/// `privacy_blocked`). Wirkt auch mitten im Vorlesen (wie ein Stimmwechsel).
#[tauri::command]
#[specta::specta]
pub async fn tts_provider_select(
    app: AppHandle,
    id: String,
    voice: Option<String>,
) -> Result<(), TtsError> {
    select_provider(&app, &id, voice).await
}

/// Der Weg hinter `tts_provider_select` — auch für die Agentenbrücke: Momentaufnahme,
/// `check_use`, Einstellungen schreiben (unter der Sperre), Stimmwechsel anstoßen.
/// Setzt nie Opt-in, Zustimmung oder Fernzugriff.
pub(crate) async fn select_provider(
    app: &AppHandle,
    id: &str,
    voice: Option<String>,
) -> Result<(), TtsError> {
    let tts = manager(app);
    let snapshot = tts.provider_snapshot();
    mutate_settings(app, |s| select_in(s, &snapshot, id, voice))?;
    tts.voice_changed().await;
    Ok(())
}

/// Probestimme: der Anbieter spricht `text`, das Audio wird abgespielt, sobald
/// es eintrifft. Liefert die `request_id`; Verlauf über das Ereignis
/// `tts-provider-preview`. Läuft das Vorlesen, wird abgelehnt (`bad_request`);
/// eine laufende Probestimme wird abgelöst. Gilt als Nutzung: Guardrails wie
/// beim Vorlesen.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_preview(
    app: AppHandle,
    id: String,
    text: String,
    voice: Option<String>,
) -> Result<String, TtsError> {
    manager(&app).start_provider_preview(&id, &text, voice)
}

/// Probestimme abbrechen (Synthese UND Wiedergabe). `false`: lief nicht mehr.
#[tauri::command]
#[specta::specta]
pub fn tts_provider_cancel(app: AppHandle, request_id: String) -> Result<bool, TtsError> {
    Ok(manager(&app).cancel_provider_preview(&request_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::TtsErrorKind;
    use crate::managers::tts::providers::mock::MockProvider;
    use crate::managers::tts::providers::policy::{
        license_fingerprint, resolve_provider_configs, ProviderOverrides,
    };
    use crate::managers::tts::providers::setup::{build_registry, ProviderPaths};

    fn paths() -> ProviderPaths {
        ProviderPaths {
            fish_dir: std::path::PathBuf::from("C:/gibt/es/nicht"),
            ..Default::default()
        }
    }

    /// Momentaufnahme mit den fünf echten Anbietern, ohne App.
    fn snapshot(providers: TtsProvidersSettings, selected: &str) -> ProviderSnapshot {
        let resolved =
            resolve_provider_configs(&providers, &|_| None, &ProviderOverrides::default());
        let registry = build_registry(&resolved, &paths(), None).unwrap();
        ProviderSnapshot {
            settings: Arc::new(providers),
            resolved: Arc::new(resolved),
            registry: Arc::new(registry),
            selected: selected.to_string(),
            fish_voice: Some("patrick".into()),
            piper_voice: None,
        }
    }

    fn healthy(snapshot: &ProviderSnapshot) -> Vec<(String, ProviderHealth)> {
        snapshot
            .registry
            .descriptors()
            .into_iter()
            .map(|d| (d.id, ProviderHealth::ok()))
            .collect()
    }

    #[test]
    fn die_liste_nennt_alle_anbieter_mit_freigabestand_und_auswahl() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let entries = build_entries(&snap, healthy(&snap));
        let ids: Vec<&str> = entries.iter().map(|e| e.descriptor.id.as_str()).collect();
        assert_eq!(
            ids,
            ["fish", "piper", "phonon", "openai-speech", "ollama-speech"]
        );
        let fish = &entries[0];
        assert!(fish.selected && fish.enabled && fish.blocked.is_none());
        assert_eq!(fish.voice.as_deref(), Some("patrick"));
        assert!(!fish.license_needs_ack, "Fish ist nie gesperrt");
        let piper = &entries[1];
        assert!(!piper.selected && piper.enabled && piper.blocked.is_none());
        let phonon = &entries[2];
        assert!(!phonon.enabled, "Opt-in fehlt");
        assert!(phonon.license_needs_ack && !phonon.license_acked);
        assert_eq!(
            phonon.blocked.as_ref().unwrap().kind,
            TtsErrorKind::LicenseBlocked
        );
        assert_eq!(phonon.fingerprint.len(), 32);
    }

    #[test]
    fn die_liste_zeigt_freigabe_zustimmung_und_fernzugriff() {
        let mut s = TtsProvidersSettings::default();
        s.openai_speech.base_url = "https://tts.example.org/v1".into();
        s.openai_speech.model = "m".into();
        s.set_enabled("openai-speech", true);
        let snap = snapshot(s.clone(), "openai-speech");
        let e = &build_entries(&snap, healthy(&snap))[3];
        assert!(e.selected && e.enabled && e.remote && !e.allow_remote);
        // Ungeprüfte Lizenz: ohne Zustimmung gesperrt (Lizenz kommt vor Datenschutz).
        assert_eq!(
            e.blocked.as_ref().unwrap().kind,
            TtsErrorKind::LicenseBlocked
        );

        let desc = snap.descriptor("openai-speech").unwrap();
        s.record_ack(
            "openai-speech",
            &license_fingerprint(&desc, &snap.resolved.subject("openai-speech")),
            "t",
        );
        let snap = snapshot(s.clone(), "openai-speech");
        let e = &build_entries(&snap, healthy(&snap))[3];
        assert!(e.license_acked);
        assert_eq!(
            e.blocked.as_ref().unwrap().kind,
            TtsErrorKind::PrivacyBlocked
        );

        s.set_remote_allowed("openai-speech", true);
        let snap = snapshot(s, "openai-speech");
        let e = &build_entries(&snap, healthy(&snap))[3];
        assert!(e.allow_remote && e.blocked.is_none());
    }

    #[test]
    fn fehlende_pruefergebnisse_werden_als_nicht_geprueft_gemeldet() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let entries = build_entries(&snap, Vec::new());
        assert!(entries
            .iter()
            .all(|e| !e.health.available && e.health.reason.is_some()));
    }

    #[tokio::test]
    async fn die_liste_haelt_die_frist_auch_bei_haengenden_anbietern() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        snap.registry
            .register(Arc::new(
                MockProvider::new().with_id("haengt").hang_health(),
            ))
            .unwrap();
        let t0 = std::time::Instant::now();
        let health = snap.registry.health_all(Duration::from_millis(200)).await;
        let entries = build_entries(&snap, health);
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        let hung = entries
            .iter()
            .find(|e| e.descriptor.id == "haengt")
            .unwrap();
        assert!(!hung.health.available);
        assert!(hung.health.reason.as_deref().unwrap().contains("Zeit"));
    }

    #[test]
    fn opt_in_setzen_nur_fuer_opt_in_anbieter() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let mut s = TtsProvidersSettings::default();
        set_enabled_in(&mut s, &snap.registry, "phonon", true).unwrap();
        assert!(s.is_enabled("phonon"));
        set_enabled_in(&mut s, &snap.registry, "phonon", false).unwrap();
        assert!(!s.is_enabled("phonon"));
        for id in ["fish", "piper", "gibt-es-nicht"] {
            let e = set_enabled_in(&mut s, &snap.registry, id, true).unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::BadRequest, "{id}");
        }
        assert!(s.enabled.is_empty());
    }

    #[test]
    fn die_lizenz_wird_nur_zum_aktuellen_fingerabdruck_bestaetigt() {
        let mut p = TtsProvidersSettings::default();
        p.ollama_speech.model = "phonon:de".into();
        let snap = snapshot(p, "fish");
        let subj = snap.resolved.subject("ollama-speech");
        let desc = snap.descriptor("ollama-speech").unwrap();
        let fp = license_fingerprint(&desc, &subj);
        let mut s = TtsProvidersSettings::default();

        // Falscher Fingerabdruck (z. B. Modell wechselte zwischen Anzeige und Klick).
        let e = acknowledge_in(
            &mut s,
            &snap.registry,
            &subj,
            "ollama-speech",
            "veraltet",
            "t",
        )
        .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert!(e.message.contains("geändert"));
        assert!(s.license_acks.is_empty());

        let ack = acknowledge_in(
            &mut s,
            &snap.registry,
            &subj,
            "ollama-speech",
            &fp,
            "2026-10-26T12:00:00Z",
        )
        .unwrap();
        assert_eq!(ack.provider_id, "ollama-speech");
        assert_eq!(ack.fingerprint, fp);
        assert_eq!(s.license_acks, vec![ack]);

        // Anbieter ohne Bestätigungsbedarf und unbekannte Anbieter.
        for id in ["fish", "piper", "gibt-es-nicht"] {
            let e = acknowledge_in(&mut s, &snap.registry, &subj, id, "x", "t").unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::BadRequest, "{id}");
        }
    }

    #[test]
    fn die_bestaetigung_gilt_nur_bis_das_modell_wechselt() {
        let mut p = TtsProvidersSettings::default();
        p.ollama_speech.model = "phonon:de".into();
        let snap = snapshot(p.clone(), "fish");
        let subj = snap.resolved.subject("ollama-speech");
        let desc = snap.descriptor("ollama-speech").unwrap();
        let mut s = p.clone();
        s.set_enabled("ollama-speech", true);
        acknowledge_in(
            &mut s,
            &snap.registry,
            &subj,
            "ollama-speech",
            &license_fingerprint(&desc, &subj),
            "t",
        )
        .unwrap();
        let ok = snapshot(s.clone(), "fish");
        assert!(ok.checked("ollama-speech").is_ok());
        // Modell geändert: dieselben Freigaben, aber die Zustimmung ist erloschen.
        s.ollama_speech.model = "phonon:en".into();
        let changed = snapshot(s, "fish");
        let e = changed.checked("ollama-speech").unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
    }

    #[test]
    fn fernzugriff_setzen_und_zuruecknehmen() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let mut s = TtsProvidersSettings::default();
        set_allow_remote_in(&mut s, &snap.registry, "openai-speech", true).unwrap();
        assert!(s.is_remote_allowed("openai-speech"));
        set_allow_remote_in(&mut s, &snap.registry, "openai-speech", false).unwrap();
        assert!(!s.is_remote_allowed("openai-speech"));
        assert_eq!(
            set_allow_remote_in(&mut s, &snap.registry, "gibt-es-nicht", true)
                .unwrap_err()
                .kind,
            TtsErrorKind::BadRequest
        );
    }

    #[test]
    fn select_setzt_engine_und_stimme_am_richtigen_ort() {
        let mut p = TtsProvidersSettings::default();
        p.set_enabled("phonon", true);
        // Lizenz bestätigen, damit phonon wählbar ist.
        let probe = snapshot(p.clone(), "fish");
        let subj = probe.resolved.subject("phonon");
        let desc = probe.descriptor("phonon").unwrap();
        p.record_ack("phonon", &license_fingerprint(&desc, &subj), "t");
        let snap = snapshot(p.clone(), "fish");

        let mut s = crate::settings::get_default_settings();
        // Der Stand der Einstellungen im Moment des Schreibens (dort wird geprüft).
        s.tts_providers = p;
        // Anbieter mit Stimme.
        select_in(&mut s, &snap, "phonon", Some(" Zoey ".into())).unwrap();
        assert_eq!(s.tts_engine, "provider:phonon");
        assert_eq!(
            s.tts_providers.voices.get("phonon").map(String::as_str),
            Some("Zoey")
        );
        // Ohne Stimme: Vorgabe des Anbieters, frühere Auswahl fällt weg.
        select_in(&mut s, &snap, "phonon", None).unwrap();
        assert!(s.tts_providers.voices.is_empty());
        // Zurück zu Fish: alter Wert, Stimme in `tts_voice`.
        select_in(&mut s, &snap, "fish", Some("patrick".into())).unwrap();
        assert_eq!(s.tts_engine, "fish");
        assert_eq!(s.tts_voice.as_deref(), Some("patrick"));
        // Piper: alter Wert, Stimme in `tts_piper_voice`.
        select_in(&mut s, &snap, "piper", Some("de_DE-thorsten-high".into())).unwrap();
        assert_eq!(s.tts_engine, "piper");
        assert_eq!(s.tts_piper_voice.as_deref(), Some("de_DE-thorsten-high"));
    }

    #[test]
    fn select_lehnt_gesperrte_und_unbekannte_anbieter_ab_ohne_etwas_zu_aendern() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let mut s = crate::settings::get_default_settings();
        let before = (
            s.tts_engine.clone(),
            s.tts_voice.clone(),
            s.tts_providers.clone(),
        );
        // phonon: nicht freigeschaltet.
        let e = select_in(&mut s, &snap, "phonon", Some("Zoey".into())).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        // Unbekannt.
        let e = select_in(&mut s, &snap, "gibt-es-nicht", None).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert_eq!(
            (
                s.tts_engine.clone(),
                s.tts_voice.clone(),
                s.tts_providers.clone()
            ),
            before
        );
        assert_eq!(s.tts_engine, "fish", "der Standard bleibt");
    }

    /// Beschreibung: ein Anbieter mit Opt-in, Lizenz-Zustimmung und aktuellem Modell.
    fn approved_phonon() -> (TtsProvidersSettings, ProviderSnapshot) {
        let mut p = TtsProvidersSettings::default();
        p.set_enabled("phonon", true);
        let probe = snapshot(p.clone(), "fish");
        let subj = probe.resolved.subject("phonon");
        let desc = probe.descriptor("phonon").unwrap();
        p.record_ack("phonon", &license_fingerprint(&desc, &subj), "t");
        let snap = snapshot(p.clone(), "fish");
        assert!(snap.checked("phonon").is_ok());
        (p, snap)
    }

    /// Befund 8: die „zweite Prüfung beim Schreiben“ gilt dem Stand im Moment
    /// des Schreibens, nicht dem der Momentaufnahme. Hat der Nutzer das Opt-in
    /// zwischen Momentaufnahme und Schreiben zurückgenommen, lehnt `select_in` ab
    /// und ändert nichts.
    #[test]
    fn select_prueft_den_stand_im_moment_des_schreibens_nicht_den_der_momentaufnahme() {
        let (p, snap) = approved_phonon();
        let mut current = crate::settings::get_default_settings();
        let mut revoked = p;
        revoked.set_enabled("phonon", false);
        current.tts_providers = revoked;
        let before = (
            current.tts_engine.clone(),
            current.tts_voice.clone(),
            current.tts_providers.clone(),
        );
        let e = select_in_with_env(&mut current, &snap, "phonon", Some("Zoey".into()), &|_| {
            None
        })
        .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked, "{e:?}");
        assert_eq!(
            (
                current.tts_engine.clone(),
                current.tts_voice.clone(),
                current.tts_providers.clone()
            ),
            before,
            "bei einer Ablehnung bleibt alles unverändert"
        );
    }

    /// Der Gegenstand wird neu aufgelöst: wurde das Modell nach der
    /// Momentaufnahme gewechselt, gilt die Zustimmung zum alten Modell nicht.
    #[test]
    fn select_loest_modell_und_adresse_aus_dem_aktuellen_stand_neu_auf() {
        let mut p = TtsProvidersSettings::default();
        p.ollama_speech.model = "phonon:de".into();
        p.set_enabled("ollama-speech", true);
        let probe = snapshot(p.clone(), "fish");
        let subj = probe.resolved.subject("ollama-speech");
        let desc = probe.descriptor("ollama-speech").unwrap();
        p.record_ack("ollama-speech", &license_fingerprint(&desc, &subj), "t");
        let snap = snapshot(p.clone(), "fish");
        assert!(snap.checked("ollama-speech").is_ok());

        // Modellwechsel nach der Momentaufnahme.
        let mut current = crate::settings::get_default_settings();
        current.tts_providers = p.clone();
        current.tts_providers.ollama_speech.model = "phonon:en".into();
        let e =
            select_in_with_env(&mut current, &snap, "ollama-speech", None, &|_| None).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        assert!(e.message.contains("geändert"), "{}", e.message);

        // Adresse nach der Momentaufnahme auf einen fremden Rechner gestellt.
        let mut current = crate::settings::get_default_settings();
        current.tts_providers = p;
        current.tts_providers.ollama_speech.base_url = "http://192.168.1.20:11434".into();
        let e =
            select_in_with_env(&mut current, &snap, "ollama-speech", None, &|_| None).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);

        // Stimmt der aktuelle Stand, geht die Wahl durch.
        let (p, snap) = approved_phonon();
        let mut current = crate::settings::get_default_settings();
        current.tts_providers = p;
        select_in_with_env(&mut current, &snap, "phonon", None, &|_| None).unwrap();
        assert_eq!(current.tts_engine, "provider:phonon");
    }

    /// Stufe "installiert": das Modell der Instanz kommt aus der Installation der Modellseite. Die
    /// Wahl muss denselben Stand sehen, sonst passte der Fingerabdruck der Zustimmung nie.
    #[test]
    fn select_loest_mit_demselben_stand_installiert_auf_wie_die_momentaufnahme() {
        let installed = policy::InstalledPaths {
            phonon: policy::InstalledPhonon {
                exe_path: Some("C:/app/tts/phonon/windows-x64/ptts.exe".into()),
                model: Some("C:/app/tts/phonon/models/phonon-test".into()),
            },
        };
        let mut p = TtsProvidersSettings::default();
        p.set_enabled("phonon", true);
        let resolved = policy::resolve_provider_configs_with(
            &p,
            &|_| None,
            &policy::ProviderOverrides::default(),
            &installed,
        );
        assert_eq!(
            resolved.subject("phonon").model,
            "C:/app/tts/phonon/models/phonon-test"
        );
        let registry = build_registry(&resolved, &paths(), None).unwrap();
        let snap = ProviderSnapshot {
            settings: Arc::new(p.clone()),
            resolved: Arc::new(resolved),
            registry: Arc::new(registry),
            selected: "fish".to_string(),
            fish_voice: None,
            piper_voice: None,
        };
        let desc = snap.descriptor("phonon").unwrap();
        p.record_ack(
            "phonon",
            &license_fingerprint(&desc, &snap.resolved.subject("phonon")),
            "t",
        );

        let mut current = crate::settings::get_default_settings();
        current.tts_providers = p;
        select_in_with_env(&mut current, &snap, "phonon", None, &|_| None).unwrap();
        assert_eq!(current.tts_engine, "provider:phonon");

        // Wechselt der Nutzer auf ein eigenes Modell, gilt die Zustimmung zum installierten nicht mehr.
        let mut other = crate::settings::get_default_settings();
        other.tts_providers = current.tts_providers.clone();
        other.tts_providers.phonon.model = "C:/eigenes/modell".into();
        let e = select_in_with_env(&mut other, &snap, "phonon", None, &|_| None).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked, "{e:?}");
    }

    /// Befund 7: die inneren Fristen der Anbieter liegen unter den äußeren
    /// Grenzen (sonst verwirft die äußere Frist die Teilliste).
    #[test]
    fn die_stimmenfrist_der_oberflaeche_liegt_ueber_der_inneren_katalogfrist_von_phonon() {
        use crate::managers::tts::providers::phonon::LIST_VOICES_TIMEOUT;
        assert!(
            LIST_VOICES_TIMEOUT < VOICES_LIMIT,
            "innere Frist {LIST_VOICES_TIMEOUT:?} muss unter {VOICES_LIMIT:?} liegen"
        );
        assert!(
            LIST_VOICES_TIMEOUT < crate::agent_bridge::tools::tts_providers::VOICES_LIMIT,
            "auch unter der Frist der Agentenbrücke"
        );
    }

    /// Befund 2: Beschreibung und Freigabezustand der Liste kommen aus derselben
    /// Instanz; die Liste ändert sich nicht, wenn `resolved` einen anderen Stand
    /// trägt als die Registry.
    #[test]
    fn die_liste_nimmt_den_gegenstand_aus_der_instanz_nicht_aus_resolved() {
        // Registry gebaut mit Modell A (bestätigt); `resolved` trägt Modell B.
        let mut a = TtsProvidersSettings::default();
        a.ollama_speech.model = "phonon:de".into();
        a.set_enabled("ollama-speech", true);
        let probe = snapshot(a.clone(), "fish");
        let desc = probe.descriptor("ollama-speech").unwrap();
        a.record_ack(
            "ollama-speech",
            &license_fingerprint(&desc, &probe.resolved.subject("ollama-speech")),
            "t",
        );
        let mut snap = snapshot(a.clone(), "fish");
        let mut other = a;
        other.ollama_speech.model = "phonon:en".into();
        snap.resolved = Arc::new(resolve_provider_configs(
            &other,
            &|_| None,
            &ProviderOverrides::default(),
        ));
        assert_ne!(
            snap.resolved.subject("ollama-speech"),
            snap.subject("ollama-speech")
        );
        assert_eq!(snap.subject("ollama-speech").model, "phonon:de");
        let entries = build_entries(&snap, healthy(&snap));
        let e = entries
            .iter()
            .find(|e| e.descriptor.id == "ollama-speech")
            .unwrap();
        assert!(
            e.license_acked,
            "die Zustimmung gilt der Instanz mit Modell A"
        );
        assert!(e.blocked.is_none(), "{:?}", e.blocked);
        assert!(snap.checked("ollama-speech").is_ok());
    }

    // ---- Sichtbare Stimmen im Vorlesen-Dropdown ---------------------------------------

    fn voice_ids(options: &[ProviderVoiceOption]) -> Vec<&str> {
        options.iter().map(|o| o.voice.id.as_str()).collect()
    }

    /// Momentaufnahme mit einem freigegebenen Mock-Anbieter (zwei Stimmen: mock-de, mock-en).
    fn mock_snapshot(mutate: impl FnOnce(&mut TtsProvidersSettings)) -> ProviderSnapshot {
        let mut s = TtsProvidersSettings::default();
        s.set_enabled("mock", true);
        mutate(&mut s);
        let snap = snapshot(s, "fish");
        snap.registry
            .register(Arc::new(MockProvider::new()))
            .unwrap();
        snap
    }

    #[test]
    fn sichtbare_stimmen_werden_bereinigt_gespeichert_und_leer_loescht_die_auswahl() {
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        let mut s = TtsProvidersSettings::default();
        s.set_enabled("phonon", true);
        s.record_ack("phonon", "fp", "t");
        s.allow_remote = vec!["phonon".into()];
        let freigaben = (
            s.enabled.clone(),
            s.license_acks.clone(),
            s.allow_remote.clone(),
        );

        set_visible_voices_in(
            &mut s,
            &snap.registry,
            "phonon",
            vec![
                " Nele ".into(),
                "Nele".into(),
                "".into(),
                "  ".into(),
                "Anton".into(),
                "bö\u{7}se".into(),
                "x".repeat(300),
            ],
        )
        .unwrap();
        assert_eq!(s.visible_voices["phonon"], ["Nele", "Anton"]);
        assert_eq!(
            (
                s.enabled.clone(),
                s.license_acks.clone(),
                s.allow_remote.clone()
            ),
            freigaben,
            "Freigaben bleiben unberuehrt"
        );

        set_visible_voices_in(&mut s, &snap.registry, "phonon", Vec::new()).unwrap();
        assert!(!s.visible_voices.contains_key("phonon"), "leer = alle");

        let e = set_visible_voices_in(&mut s, &snap.registry, "gibt-es-nicht", vec!["a".into()])
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert!(s.visible_voices.is_empty());
    }

    #[test]
    fn die_liste_nennt_die_sichtbaren_stimmen_je_anbieter() {
        let mut p = TtsProvidersSettings::default();
        p.visible_voices
            .insert("phonon".into(), vec!["Nele".into()]);
        let snap = snapshot(p, "fish");
        let entries = build_entries(&snap, healthy(&snap));
        let by_id = |id: &str| entries.iter().find(|e| e.descriptor.id == id).unwrap();
        assert_eq!(by_id("phonon").visible_voices, ["Nele"]);
        assert!(by_id("piper").visible_voices.is_empty(), "leer = alle");
    }

    #[tokio::test]
    async fn das_dropdown_zeigt_nur_die_gewaehlten_stimmen_eines_freigegebenen_anbieters() {
        let all = |_: &ProviderDescriptor| true;
        // Ohne Auswahl: alle Stimmen.
        let snap = mock_snapshot(|_| {});
        let options = voice_options(&snap, all).await;
        assert_eq!(voice_ids(&options), ["mock-de", "mock-en"]);
        assert!(options.iter().all(|o| o.provider_id == "mock"));
        assert!(options.iter().all(|o| o.provider_label == "Mock (Test)"));

        // Mit Auswahl: nur diese.
        let snap = mock_snapshot(|s| {
            s.visible_voices
                .insert("mock".into(), vec!["mock-en".into()]);
        });
        assert_eq!(voice_ids(&voice_options(&snap, all).await), ["mock-en"]);

        // „Keine“ (nur die Standardstimme): keine benannte Stimme ausser der gewaehlten.
        let snap = mock_snapshot(|s| {
            s.visible_voices
                .insert("mock".into(), vec![policy::DEFAULT_VOICE_ID.into()]);
        });
        assert!(voice_options(&snap, all).await.is_empty());

        // Die gerade gewaehlte Stimme bleibt sichtbar, auch wenn sie abgewaehlt ist.
        let snap = mock_snapshot(|s| {
            s.visible_voices
                .insert("mock".into(), vec!["mock-en".into()]);
            s.voices.insert("mock".into(), "mock-de".into());
        });
        assert_eq!(
            voice_ids(&voice_options(&snap, all).await),
            ["mock-de", "mock-en"]
        );
    }

    #[tokio::test]
    async fn das_dropdown_fragt_nur_freigegebene_anbieter_und_nie_fish_piper_oder_mock() {
        let all = |_: &ProviderDescriptor| true;
        // Ohne Opt-in gesperrt: keine Stimmen, kein Aufruf.
        let snap = snapshot(TtsProvidersSettings::default(), "fish");
        snap.registry
            .register(Arc::new(MockProvider::new()))
            .unwrap();
        assert!(voice_options(&snap, all).await.is_empty());

        // Freigegeben, aber mit den echten Regeln: Mock (nur fuer Tests) und die eingebauten fehlen.
        let snap = mock_snapshot(|_| {});
        assert!(voice_options(&snap, offers_dropdown_voices)
            .await
            .is_empty());
        let by_id = |id: &str| {
            snap.registry
                .descriptors()
                .into_iter()
                .find(|d| d.id == id)
                .unwrap()
        };
        assert!(!offers_dropdown_voices(&by_id("fish")));
        assert!(!offers_dropdown_voices(&by_id("piper")));
        assert!(offers_dropdown_voices(&by_id("phonon")));

        // Nicht erreichbarer Anbieter: fehlt einfach, kein Fehler.
        let mut s = TtsProvidersSettings::default();
        s.set_enabled("kaputt", true);
        let snap = snapshot(s, "fish");
        snap.registry
            .register(Arc::new(
                MockProvider::new().with_id("kaputt").unavailable("aus"),
            ))
            .unwrap();
        assert!(voice_options(&snap, all).await.is_empty());
    }

    #[test]
    fn die_gewaehlte_engine_ueberlebt_den_weg_ueber_die_einstellungen() {
        // `provider:<id>` ist ein gewöhnlicher String in `tts_engine`: hin und zurück.
        let mut s = crate::settings::get_default_settings();
        s.tts_engine = engine::engine_setting_for("openai-speech");
        let json = serde_json::to_value(&s).unwrap();
        let back: AppSettings = serde_json::from_value(json).unwrap();
        assert_eq!(
            engine::provider_id_from_setting(&back.tts_engine).as_deref(),
            Some("openai-speech")
        );
    }
}
