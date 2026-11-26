//! Gemeinsames Tor fuer Installationsbeginn und Audio-Starts (lva#869 H3).
use serde::{Deserialize, Serialize};
use specta::Type;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

const FRIST: Duration = Duration::from_secs(10 * 60);
const GRUENDE: [Grund; 4] = [Grund::Diktat, Grund::Besprechung, Grund::Vorlesen, Grund::Anruf];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub enum Grund {
    Diktat,
    Besprechung,
    Vorlesen,
    Anruf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "code", content = "grund", rename_all = "snake_case")]
pub enum UpdateGateFehler {
    Belegt(Grund),
    UpdateLaeuft,
    NichtVerfuegbar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct UpdateGateStatus {
    pub belegt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grund: Option<Grund>,
}

#[derive(Default)]
struct Gate {
    installation: Option<(String, Instant, bool)>,
    starts: BTreeMap<u64, Grund>,
    next: u64,
    gemeldet: Option<UpdateGateStatus>,
}

impl Gate {
    fn expire(&mut self, jetzt: Instant) {
        if self.installation.as_ref().is_some_and(|(_, seit, installiert)| {
            !installiert && jetzt.saturating_duration_since(*seit) >= FRIST
        }) {
            self.abort();
        }
    }
    fn abort(&mut self) {
        self.installation = None;
    }
    fn start(&mut self, grund: Grund, jetzt: Instant) -> Result<u64, &'static str> {
        self.expire(jetzt);
        if self.installation.is_some() {
            return Err("update_laeuft");
        }
        self.next = self.next.checked_add(1).ok_or("update_laeuft")?;
        self.starts.insert(self.next, grund);
        Ok(self.next)
    }
    fn begin(
        &mut self,
        version: String,
        grund: Option<Grund>,
        jetzt: Instant,
    ) -> Result<(), UpdateGateFehler> {
        self.expire(jetzt);
        if self.installation.is_some() {
            return Err(UpdateGateFehler::UpdateLaeuft);
        }
        if let Some(grund) = self.starts.values().next().copied().or(grund) {
            return Err(UpdateGateFehler::Belegt(grund));
        }
        self.installation = Some((version, jetzt, false));
        Ok(())
    }
    fn install(&mut self, version: &str, jetzt: Instant) -> Result<(), UpdateGateFehler> {
        self.expire(jetzt);
        match self.installation.as_mut() {
            Some((bestaetigt, _, installiert)) if bestaetigt == version => {
                *installiert = true;
                Ok(())
            }
            _ => Err(UpdateGateFehler::NichtVerfuegbar),
        }
    }
}

static GATE: Mutex<Gate> = Mutex::new(Gate {
    installation: None,
    starts: BTreeMap::new(),
    next: 0,
    gemeldet: None,
});

fn erster_grund(zustaende: [Result<bool, ()>; 4]) -> Option<Grund> {
    GRUENDE.into_iter().zip(zustaende).find_map(|(grund, wert)| {
        if wert != Ok(false) { Some(grund) } else { None }
    })
}

fn zustaende_belegt(app: &AppHandle, gate: &Gate) -> Option<Grund> {
    use crate::managers::{
        audio::AudioRecordingManager, meetings::recorder::MeetingRecorderManager, tts::TtsManager,
    };
    erster_grund([
        app.try_state::<Arc<AudioRecordingManager>>()
            .ok_or(())
            .and_then(|m| m.update_belegt()),
        app.try_state::<Arc<MeetingRecorderManager>>()
            .ok_or(())
            .and_then(|m| m.update_belegt()),
        app.try_state::<Arc<TtsManager>>()
            .ok_or(())
            .and_then(|m| m.update_belegt()),
        // Die Gespraechs-Engine besitzt eine Reservierung bis nach Capture-/Player-Stopp.
        Ok(gate.starts.values().any(|grund| *grund == Grund::Anruf)),
    ])
}

fn status_unter_sperre(app: &AppHandle, gate: &Gate) -> UpdateGateStatus {
    let grund = gate.starts.values().next().copied().or_else(|| zustaende_belegt(app, gate));
    UpdateGateStatus { belegt: grund.is_some(), grund }
}

pub fn beschaeftigt(app: &AppHandle) -> Option<Grund> {
    GATE.lock().map(|gate| status_unter_sperre(app, &gate).grund).unwrap_or(Some(Grund::Diktat))
}

#[tauri::command]
#[specta::specta]
pub fn update_gate_status(app: AppHandle) -> Result<UpdateGateStatus, UpdateGateFehler> {
    let grund = beschaeftigt(&app);
    Ok(UpdateGateStatus { belegt: grund.is_some(), grund })
}

/// Meldung nach Freigabe der Manager-Sperren; keine Sperrordnung Manager -> Tor.
pub fn geaendert(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(mut gate) = GATE.lock() {
            let status = status_unter_sperre(&app, &gate);
            if gate.gemeldet.as_ref() != Some(&status) {
                gate.gemeldet = Some(status.clone());
                let _ = app.emit("lv-update-gate", status);
            }
        }
    });
}

/// Reserviert vor jeder Vorbereitung. Der Send-faehige Guard darf ueber await leben.
pub struct Start {
    id: u64,
    app: AppHandle,
}
pub struct Aenderung(pub AppHandle);
impl Drop for Aenderung {
    fn drop(&mut self) {
        geaendert(&self.0);
    }
}
impl Drop for Start {
    fn drop(&mut self) {
        if let Ok(mut gate) = GATE.lock() {
            gate.starts.remove(&self.id);
        }
        geaendert(&self.app);
    }
}
pub fn pruefe_start(app: &AppHandle, grund: Grund) -> Result<Start, String> {
    let id = GATE.lock().map_err(|_| "update_laeuft")?.start(grund, Instant::now())?;
    geaendert(app);
    Ok(Start { id, app: app.clone() })
}

#[tauri::command]
#[specta::specta]
pub fn update_install_begin(app: AppHandle, version: String) -> Result<(), UpdateGateFehler> {
    let mut gate = GATE.lock().map_err(|_| UpdateGateFehler::NichtVerfuegbar)?;
    let jetzt = Instant::now();
    let grund = zustaende_belegt(&app, &gate);
    gate.begin(version, grund, jetzt)?;
    drop(gate);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FRIST).await;
        if let Ok(mut gate) = GATE.lock() {
            gate.expire(Instant::now());
        }
    });
    Ok(())
}

/// Unmittelbar vor plugin-updater.install: ab hier keine automatische Freigabe.
#[tauri::command]
#[specta::specta]
pub fn update_install_commit(version: String) -> Result<(), UpdateGateFehler> {
    GATE.lock().map_err(|_| UpdateGateFehler::NichtVerfuegbar)?.install(&version, Instant::now())
}

#[tauri::command]
#[specta::specta]
pub fn update_install_abort() -> Result<(), UpdateGateFehler> {
    GATE.lock().map_err(|_| UpdateGateFehler::NichtVerfuegbar)?.abort();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn update_call_begin(app: AppHandle) -> Result<String, String> {
    let id = GATE.lock().map_err(|_| "update_laeuft")?.start(Grund::Anruf, Instant::now())?;
    geaendert(&app);
    Ok(id.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn update_call_end(app: AppHandle, id: String) -> Result<(), String> {
    let id = id.parse::<u64>().map_err(|_| "update_sitzung")?;
    let mut gate = GATE.lock().map_err(|_| "update_laeuft")?;
    if gate.starts.get(&id) == Some(&Grund::Anruf) {
        gate.starts.remove(&id);
    }
    drop(gate);
    geaendert(&app);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateSpaeter {
    pub version: String,
    pub bis: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct UpdateVerlauf {
    pub spaeter: Option<UpdateSpaeter>,
    pub aktualisiert_auf: Option<String>,
}

static STARTVERSION: Mutex<Option<Option<String>>> = Mutex::new(None);

fn aktualisiert(vorher: Option<&str>, jetzt: &str) -> bool {
    vorher
        .and_then(|v| {
            Some(crate::local_update::parse_version(jetzt)? > crate::local_update::parse_version(v)?)
        })
        .unwrap_or(false)
}

/// Eigene Schluessel im Einstellungsspeicher; kein Rueckschreiben alter AppSettings.
#[tauri::command]
#[specta::specta]
pub fn update_verlauf(app: AppHandle) -> Result<UpdateVerlauf, String> {
    use tauri_plugin_store::StoreExt;
    let store = app.store(crate::portable::store_path(crate::settings::SETTINGS_STORE_PATH))
        .map_err(|_| "update_speicher")?;
    let mut start = STARTVERSION.lock().map_err(|_| "update_speicher")?;
    if start.is_none() {
        let version = app.package_info().version.to_string();
        let vorher = store.get("update_letzter_start").and_then(|v| v.as_str().map(str::to_owned));
        let neuer = aktualisiert(vorher.as_deref(), &version);
        store.set("update_letzter_start", serde_json::json!(version));
        store.save().map_err(|_| "update_speicher")?;
        *start = Some(neuer.then_some(version));
    }
    Ok(UpdateVerlauf {
        spaeter: store.get("update_spaeter").and_then(|v| serde_json::from_value(v).ok()),
        aktualisiert_auf: start.as_ref().cloned().flatten(),
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_spaeter(app: AppHandle, version: String, bis: f64) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    if !bis.is_finite() || bis < 0.0 {
        return Err("update_frist".into());
    }
    let store = app.store(crate::portable::store_path(crate::settings::SETTINGS_STORE_PATH))
        .map_err(|_| "update_speicher")?;
    store.set("update_spaeter", serde_json::json!(UpdateSpaeter { version, bis }));
    store.save().map_err(|_| "update_speicher".into())
}

#[cfg(test)]
#[path = "update_gate_tests.rs"]
mod tests;
