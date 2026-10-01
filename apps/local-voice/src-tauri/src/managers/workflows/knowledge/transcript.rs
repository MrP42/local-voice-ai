//! Der Baustein `youtube.transcript` (B6): die Untertitel eines YouTube-Videos als Transkript holen.
//!
//! Der Weg ist der aus A3 (E1, Variante B'): ein selbst installiertes `yt-dlp` hinter dem Schalter
//! „privat“ in den Einstellungen, im Job-Objekt von `process_guard` (Speicher- und CPU-Deckel,
//! `KILL_ON_JOB_CLOSE`, Zeitlimit), Tor und Audit `media.fetch` je Abruf. Der Baustein ruft ihn ueber
//! `AppServices::youtube_subtitles` (Spur waehlen: die empfohlene, sonst die erste; laden; als aktive Fassung
//! anlegen). Nichts wird gebuendelt oder von der App geladen.
//!
//! **Recht.** `media.fetch` an der YouTube-Integration `via`: fuer Ablaeufe ab Werk AUS (das Holen einer Datei
//! ueber ein externes Programm schaltet der Nutzer ausdruecklich ein, A1). Zusaetzlich verlangt der A3-Weg den
//! Schalter „privat“ und ein gefundenes Programm; fehlt eins, steht der Satz aus A3 im Ergebnis.
//!
//! **Idempotenz.** Hat die Besprechung schon ein Transkript (Segmente), tut der Baustein nichts (`reused`):
//! eine Wiederholung nach einem Absturz legt keine zweite Fassung an. Die Besprechung muss zum Video
//! gehoeren (`video` stimmt mit der Quelle der Besprechung ueberein), sonst `Permanent`.
//!
//! **Nicht Teil dieses Bausteins**: die eigene Transkription (Audio holen, Sprachmodell laufen lassen) fuer
//! Videos ohne Untertitel. Ein Video ohne Untertitel endet mit einem Satz (`Permanent`); den Lauf kann der
//! Nutzer von Hand ueber „Eigene Transkription“ der Besprechung fortsetzen.
//!
//! # Fehlerklassen
//!
//! | Lage | Ergebnis |
//! |------|----------|
//! | Schalter „privat“ aus, yt-dlp fehlt oder veraltet, keine Untertitel, Video privat/geloescht, Tor sagt nein | `Permanent` mit dem Satz aus A3 |
//! | Netz, 429, Zeit, yt-dlp endet mit Fehler, Speicher gesperrt | `Transient` (es wurde nichts angelegt) |
//! | Nutzer bricht ab | `Transient` |
//! | Besprechung gehoert nicht zum Video / ist keine YouTube-Besprechung | `Permanent` |

use std::sync::Arc;

use serde_json::{json, Value};

use crate::managers::youtube::{source::read_source, YoutubeError};

use super::super::action::{Action, EffectKind, Needs, NeedsError, RunCtx, StepError, StepOutput};
use super::super::app_actions::{
    meeting_id_of, no_meeting, spec_of, svc, text_param, AppServices, ServiceError,
};
use super::super::catalog::{self, ActionSpec};
use crate::managers::provenance::SourceRef;

/// Ein Fehler des A3-Wegs in die Sprache der Engine.
pub fn service_error_of_youtube(e: &YoutubeError) -> ServiceError {
    let text = e.to_string();
    match e {
        YoutubeError::Cancelled => ServiceError::Cancelled,
        // Etwas, das der Nutzer erst aendern muss, oder was nie gelingt.
        YoutubeError::PrivateOff
        | YoutubeError::ToolMissing
        | YoutubeError::ToolOutdated
        | YoutubeError::NoSubtitles
        | YoutubeError::Disabled(_)
        | YoutubeError::Unavailable
        | YoutubeError::Link(_)
        | YoutubeError::Project
        | YoutubeError::Target(_) => ServiceError::Permanent(text),
        // Nichts wurde angelegt (Netz, Zeit, Kindprozess, Datenbank): ein neuer Versuch ist sicher.
        YoutubeError::Busy
        | YoutubeError::RateLimited
        | YoutubeError::Timeout
        | YoutubeError::Network(_)
        | YoutubeError::Http(_)
        | YoutubeError::BadResponse(_)
        | YoutubeError::Store(_)
        | YoutubeError::ToolFailed(_)
        | YoutubeError::ToolStart(_) => ServiceError::Transient(text),
    }
}

pub struct YoutubeTranscript {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl YoutubeTranscript {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("youtube.transcript"),
            services,
        }
    }
}

fn output(
    meeting_id: &str,
    title: &str,
    video: &str,
    language: Option<&str>,
    auto: Option<bool>,
    segments: usize,
    reused: bool,
) -> StepOutput {
    let url = format!("https://www.youtube.com/watch?v={video}");
    let mut out = StepOutput::with_data(json!({
        "meeting_id": meeting_id,
        "meeting": {"id": meeting_id, "title": title},
        "video_id": video,
        "language": language,
        "auto": auto,
        "segments": segments,
        "reused": reused,
    }));
    out.summary = Some(if reused {
        "Das Transkript lag schon vor.".to_string()
    } else {
        format!(
            "Untertitel geholt ({} Segmente{}).",
            segments,
            match (language, auto) {
                (Some(l), Some(true)) => format!(", {l}, automatisch erzeugt"),
                (Some(l), _) => format!(", {l}"),
                _ => String::new(),
            }
        )
    });
    let mut source = SourceRef::new("youtube", video, Some(title));
    source.url = Some(url);
    out.sources = vec![source];
    out
}

impl Action for YoutubeTranscript {
    fn id(&self) -> &str {
        "youtube.transcript"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let video = text_param(params, "video").ok_or_else(|| {
            StepError::Permanent(
                "Es ist kein Video angegeben (Parameter video, die Video-ID).".to_string(),
            )
        })?;
        let meeting_id = meeting_id_of(ctx).ok_or_else(no_meeting)?;
        let store = self.services.store().map_err(svc)?;
        let source = read_source(&store, &meeting_id)
            .map_err(|e| StepError::Transient(format!("Die Quelle ließ sich nicht lesen ({e}).")))?
            .ok_or_else(|| {
                StepError::Permanent("Die Besprechung ist keine YouTube-Besprechung.".to_string())
            })?;
        if source.video_id != video {
            return Err(StepError::Permanent(
                "Die Besprechung gehört zu einem anderen Video als dem angegebenen.".to_string(),
            ));
        }
        // Schon ein Transkript (Wiederholung, Absturz nach dem Anlegen)? Dann nichts tun.
        let existing = store.get_segments(&meeting_id).map_err(|e| {
            StepError::Transient(format!("Das Transkript ließ sich nicht lesen ({e})."))
        })?;
        if !existing.is_empty() {
            return Ok(output(
                &meeting_id,
                &source.title,
                video,
                None,
                None,
                existing.len(),
                true,
            ));
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen.".to_string(),
            ));
        }
        let outcome = self
            .services
            .youtube_subtitles(&meeting_id, ctx.cancel)
            .map_err(svc)?;
        Ok(output(
            &meeting_id,
            &source.title,
            video,
            Some(&outcome.language),
            Some(outcome.auto),
            outcome.segments,
            false,
        ))
    }
}
