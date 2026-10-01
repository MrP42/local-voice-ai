//! Die Dienste der laufenden App hinter den App-Bausteinen (B4, siehe `app_actions`).
//!
//! Duenne Huelle: jeder Aufruf geht denselben Weg wie ein Klick des Nutzers
//! (`enhance_and_notify`, `generate_and_notify`: Auftrag im Verzeichnis der Verarbeitungen,
//! Fortschritt und Ereignisse fuer die Oberflaeche), nur blockierend, weil die Engine auf
//! einem eigenen Arbeiter-Thread laeuft. Die Entscheidungen (Fehlerklassen, Warten, Abbruch)
//! stehen in `app_actions` und sind dort getestet; hier steht nur der Anschluss an die
//! verwalteten Zustaende der App.
//!
//! Speicher: das Sprachmodell startet der Modellverwalter mit seinem RAM-Start-Tor und im
//! Job-Objekt (`process_guard`); bei zu wenig Arbeitsspeicher meldet er `memory_low`, der
//! Schritt wartet dann (`Defer`). Die Sprachausgabe geht durch die `TtsManager`-Warteschlange
//! mit ihrem eigenen Tor.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::commands::{meeting_enhance, meeting_minutes};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};
use crate::managers::tts::TtsManager;
use crate::summarizer::SummaryOptions;

use super::app_actions::{
    classify_generation_error, finish_generation, run_cancellable, AppServices, GenRequest,
    ServiceError,
};
use super::toast;

pub struct AppServicesImpl {
    app: AppHandle,
}

impl AppServicesImpl {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }
}

impl AppServices for AppServicesImpl {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        self.app
            .try_state::<Arc<MeetingStore>>()
            .map(|s| Arc::clone(&s))
            .ok_or_else(|| {
                ServiceError::Transient("Die Besprechungen sind noch nicht bereit.".to_string())
            })
    }

    fn llm_is_local(&self) -> bool {
        crate::settings::get_settings(&self.app)
            .active_post_process_provider()
            .map(crate::managers::llm::is_local)
            // Ohne Anbieter scheitert der Schritt ohnehin (`no_provider`); sicherheitshalber lokal.
            .unwrap_or(true)
    }

    fn generate_notes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        let store = self.store()?;
        let recording_active = self
            .app
            .try_state::<Arc<MeetingRecorderManager>>()
            .is_some_and(|r| r.is_recording());
        finish_generation(run_cancellable(
            cancel,
            meeting_enhance::enhance_and_notify(
                &self.app,
                store,
                recording_active,
                &req.meeting_id,
                req.template_id.as_deref(),
                &req.basis,
            ),
        ))
    }

    fn generate_minutes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        let store = self.store()?;
        finish_generation(run_cancellable(
            cancel,
            meeting_minutes::generate_and_notify(
                &self.app,
                store,
                &req.meeting_id,
                req.template_id.as_deref(),
                &req.basis,
            ),
        ))
    }

    fn summarize(
        &self,
        text: &str,
        opts: &SummaryOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        let settings = crate::settings::get_settings(&self.app);
        finish_generation(run_cancellable(
            cancel,
            crate::summarizer::summarize(&settings, text, opts),
        ))
    }

    fn audio_dir(&self, meeting_id: Option<&str>) -> Result<PathBuf, ServiceError> {
        let transient = |e: String| {
            ServiceError::Transient(format!("Der Ordner für die Audiodatei ist nicht erreichbar ({e})."))
        };
        match meeting_id {
            Some(id) => Ok(crate::managers::meetings::meetings_data_dir(&self.app)
                .map_err(|e| transient(e.to_string()))?
                .join(id)),
            None => Ok(crate::portable::app_data_dir(&self.app)
                .map_err(|e| transient(e.to_string()))?
                .join("workflow-audio")),
        }
    }

    fn render_speech(
        &self,
        text: &str,
        out: &Path,
        cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError> {
        let tts = self
            .app
            .try_state::<Arc<TtsManager>>()
            .map(|s| Arc::clone(&s))
            .ok_or_else(|| {
                ServiceError::NotAvailable("Die Sprachausgabe ist nicht bereit.".to_string())
            })?;
        let out_text = out.to_string_lossy().to_string();
        let result = run_cancellable(cancel, tts.speak_to_file(text, &out_text));
        if result.is_none() {
            // Das Future ist gefallen; der Sprachserver soll auch aufhoeren.
            tts.cancel_export();
        }
        match result {
            None => Err(ServiceError::Cancelled),
            Some(Ok((_, path))) => Ok(PathBuf::from(path)),
            Some(Err(e)) if e == "abgebrochen" => Err(ServiceError::Cancelled),
            Some(Err(e)) => match classify_generation_error(&e) {
                // Speichermangel und Gleiches melden die Verwalter als Text; alles andere ist
                // ein Fehler der Sprachausgabe, an dem nichts haengt (es wurde nichts abgelegt).
                busy @ ServiceError::Busy { .. } => Err(busy),
                _ => Err(ServiceError::Transient(format!(
                    "Die Sprachausgabe ist gescheitert: {e}"
                ))),
            },
        }
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError> {
        toast::show(&self.app.config().identifier, title, body)
            .map_err(|e| ServiceError::Transient(format!("Die Mitteilung ließ sich nicht anzeigen ({e}).")))
    }
}
