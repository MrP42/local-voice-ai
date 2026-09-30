//! P8a: Pruefhaken fuer den Headless-Lauf (`--job-script`, `--job-events`,
//! `--continue-meeting`). Kein Produktpfad: er laesst einen echten Import oder
//! ein echtes Fortsetzen ueber dieselben Aufrufe steuern, die die Knoepfe der
//! Oberflaeche nehmen (`job::global()`), und schreibt jedes Ereignis als
//! JSON-Zeile in eine Datei, damit "Fortschritt kommt an, Pause und Stopp
//! greifen" als Beleg vorliegt, ohne Fenster.
//!
//! Das Skript ist eine Liste `aktion@sekunden` (`pause`, `resume`, `stop`),
//! gerechnet ab dem Moment, in dem der Auftrag erscheint, zum Beispiel
//! `pause@4,resume@9,stop@14`. Mit einer Phase statt der Sekunden wartet der
//! Schritt, bis der Auftrag in dieser Phase ist (`stop@speakers`), optional mit
//! Abstand (`stop@speakers+5`); so trifft ein Stopp die Sprechertrennung, deren
//! Beginn von der Laenge der Aufnahme abhaengt.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri_specta::Event;

use super::job::{JobPhase, MeetingJobs};
use super::recorder::MeetingEvent;

/// So lange wartet das Skript auf den Auftrag, bevor es aufgibt.
pub const WAIT_FOR_JOB: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptAction {
    Pause,
    Resume,
    Stop,
}

impl ScriptAction {
    fn name(self) -> &'static str {
        match self {
            ScriptAction::Pause => "pause",
            ScriptAction::Resume => "resume",
            ScriptAction::Stop => "stop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptStep {
    /// Bezugspunkt: das Erscheinen des Auftrags (`None`) oder der Beginn
    /// dieser Phase.
    pub phase: Option<JobPhase>,
    /// Abstand zum Bezugspunkt.
    pub at: Duration,
    pub action: ScriptAction,
}

fn parse_phase(name: &str) -> Option<JobPhase> {
    match name {
        "prepare" => Some(JobPhase::Prepare),
        "transcription" => Some(JobPhase::Transcription),
        "final_pass" => Some(JobPhase::FinalPass),
        "speakers" => Some(JobPhase::Speakers),
        "notes" => Some(JobPhase::Notes),
        "minutes" => Some(JobPhase::Minutes),
        _ => None,
    }
}

fn parse_seconds(text: &str) -> Result<Duration, String> {
    let secs: f64 = text
        .trim()
        .parse()
        .map_err(|_| format!("'{text}' ist keine Sekundenzahl"))?;
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("'{text}': Sekunden muessen >= 0 sein"));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// `pause@4,resume@9.5,stop@speakers+5` -> Schritte. Reine Zeitschritte werden
/// nach Zeit geordnet; sobald eine Phase vorkommt, gilt die Reihenfolge der
/// Eingabe (die Zeit der Phase ist vorher nicht bekannt).
pub fn parse_script(text: &str) -> Result<Vec<ScriptStep>, String> {
    let mut steps = Vec::new();
    for item in text.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (name, when) = item
            .split_once('@')
            .ok_or_else(|| format!("'{item}': erwartet aktion@sekunden"))?;
        let action = match name.trim().to_ascii_lowercase().as_str() {
            "pause" => ScriptAction::Pause,
            "resume" => ScriptAction::Resume,
            "stop" => ScriptAction::Stop,
            other => return Err(format!("unbekannte Aktion '{other}' (pause, resume, stop)")),
        };
        let when = when.trim();
        let starts_with_letter = when.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        let (phase, at) = if starts_with_letter && !when.eq_ignore_ascii_case("nan") {
            let (phase_name, offset) = match when.split_once('+') {
                Some((p, o)) => (p, parse_seconds(o)?),
                None => (when, Duration::ZERO),
            };
            let phase = parse_phase(&phase_name.trim().to_ascii_lowercase())
                .ok_or_else(|| format!("unbekannte Phase '{phase_name}'"))?;
            (Some(phase), offset)
        } else {
            (None, parse_seconds(when)?)
        };
        steps.push(ScriptStep { phase, at, action });
    }
    if steps.is_empty() {
        return Err("leeres Skript".to_string());
    }
    if steps.iter().all(|s| s.phase.is_none()) {
        steps.sort_by_key(|s| s.at);
    }
    Ok(steps)
}

/// Fuehrt `steps` gegen den (einzigen) laufenden Auftrag aus. Jeder Schritt
/// und sein Ergebnis gehen an `log`. Gibt die Ergebnisse zurueck (`ok` oder der
/// Fehlercode des Befehls).
pub fn run_script(
    jobs: &MeetingJobs,
    steps: &[ScriptStep],
    wait_for_job: Duration,
    log: &dyn Fn(Value),
) -> Vec<(ScriptAction, String)> {
    let waited = Instant::now();
    let meeting_id = loop {
        if let Some(first) = jobs.snapshots().into_iter().next() {
            break first.meeting_id;
        }
        if waited.elapsed() > wait_for_job {
            log(json!({ "kind": "script", "error": "no_job_appeared" }));
            return Vec::new();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut origin = Instant::now();
    let mut results = Vec::new();
    for step in steps {
        if let Some(phase) = step.phase {
            // Auf den Beginn der Phase warten (so lange der Auftrag laeuft: die
            // Transkription einer langen Aufnahme braucht Minuten); der Abstand
            // zaehlt ab dann.
            loop {
                let reached = jobs
                    .snapshots()
                    .into_iter()
                    .find(|p| p.meeting_id == meeting_id)
                    .is_some_and(|p| p.phase == phase);
                if reached {
                    break;
                }
                // Der Auftrag ist zu Ende, ohne je in die Phase zu kommen.
                if !jobs.is_running(&meeting_id) {
                    log(json!({
                        "kind": "script",
                        "action": step.action.name(),
                        "error": "phase_not_reached"
                    }));
                    return results;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            origin = Instant::now();
        }
        if let Some(rest) = step.at.checked_sub(origin.elapsed()) {
            std::thread::sleep(rest);
        }
        let result = match step.action {
            ScriptAction::Pause => jobs.pause(&meeting_id),
            ScriptAction::Resume => jobs.resume(&meeting_id),
            ScriptAction::Stop => jobs.stop(&meeting_id),
        };
        let outcome = match result {
            Ok(()) => "ok".to_string(),
            Err(e) => e.to_string(),
        };
        log(json!({
            "kind": "script",
            "action": step.action.name(),
            "at_ms": origin.elapsed().as_millis() as u64,
            "result": outcome,
            "state": jobs.snapshots().into_iter().find(|p| p.meeting_id == meeting_id)
                .map(|p| serde_json::to_value(p.state).unwrap_or(Value::Null)),
        }));
        results.push((step.action, outcome));
    }
    results
}

/// Eine Logdatei mit einer JSON-Zeile je Eintrag und der Zeit seit dem Start.
pub struct EventLog {
    file: Mutex<std::fs::File>,
    t0: Instant,
}

impl EventLog {
    pub fn create(path: &Path) -> std::io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            file: Mutex::new(std::fs::File::create(path)?),
            t0: Instant::now(),
        }))
    }

    pub fn write(&self, mut value: Value) {
        value["t_ms"] = json!(self.t0.elapsed().as_millis() as u64);
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{value}");
            let _ = file.flush();
        }
    }
}

/// Haengt die Logdatei an die Besprechungs-Ereignisse: Zustand, Fortschritt,
/// Ende und Fehler (nie Segmenttexte: die Datei ist ein Beleg, kein Transkript).
pub fn attach_event_log(app: &tauri::AppHandle, log: Arc<EventLog>) {
    MeetingEvent::listen_any(app, move |event| match &event.payload {
        MeetingEvent::Progress { .. }
        | MeetingEvent::JobEnded { .. }
        | MeetingEvent::State { .. }
        | MeetingEvent::Error { .. }
        | MeetingEvent::TranscriptFinal { .. } => {
            if let Ok(value) = serde_json::to_value(&event.payload) {
                log.write(value);
            }
        }
        MeetingEvent::Segments { appended, .. } => {
            log.write(json!({ "kind": "segments", "count": appended.len() }));
        }
        _ => {}
    });
}

/// U7: haengt die Zustaende der Import-Warteschlange (wartend, laufend,
/// angehalten, warum nicht) an dieselbe Logdatei.
pub fn attach_queue_log(app: &tauri::AppHandle, log: Arc<EventLog>) {
    use super::queue::ImportQueueEvent;
    ImportQueueEvent::listen_any(app, move |event| {
        let snapshot = &event.payload.snapshot;
        log.write(json!({
            "kind": "queue",
            "waiting": snapshot.waiting,
            "running": snapshot.running,
            "held": snapshot.held,
            "limit": snapshot.limit,
            "blocked": snapshot.blocked,
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::job::{JobPhase, JobRunState};
    use std::sync::Mutex as StdMutex;

    #[test]
    fn a_script_is_parsed_and_ordered_by_time() {
        let steps = parse_script("stop@14, pause@4 ,resume@9.5").unwrap();
        assert_eq!(
            steps,
            vec![
                ScriptStep {
                    phase: None,
                    at: Duration::from_secs(4),
                    action: ScriptAction::Pause
                },
                ScriptStep {
                    phase: None,
                    at: Duration::from_millis(9_500),
                    action: ScriptAction::Resume
                },
                ScriptStep {
                    phase: None,
                    at: Duration::from_secs(14),
                    action: ScriptAction::Stop
                },
            ]
        );
    }

    #[test]
    fn a_step_can_wait_for_a_phase_with_an_optional_offset_and_keeps_the_input_order() {
        let steps = parse_script("stop@speakers+2.5, pause@transcription").unwrap();
        assert_eq!(
            steps,
            vec![
                ScriptStep {
                    phase: Some(JobPhase::Speakers),
                    at: Duration::from_millis(2_500),
                    action: ScriptAction::Stop
                },
                ScriptStep {
                    phase: Some(JobPhase::Transcription),
                    at: Duration::ZERO,
                    action: ScriptAction::Pause
                },
            ],
            "mit Phase zaehlt die Reihenfolge der Eingabe"
        );
        assert!(parse_script("stop@sprechen")
            .unwrap_err()
            .contains("unbekannte Phase"));
        assert!(parse_script("stop@speakers+abc").is_err());
    }

    #[test]
    fn a_phase_step_fires_when_the_job_reaches_the_phase_and_gives_up_when_it_never_does() {
        let jobs = MeetingJobs::new();
        let guard = jobs.try_start("m1", Arc::new(|_| {})).unwrap();
        guard.begin_phase(JobPhase::Transcription, 1_000);
        let jobs2 = jobs.clone();
        let handle = Arc::clone(guard.handle());
        // Nach 150 ms wechselt der Auftrag in die Sprechertrennung.
        let switcher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            handle.begin_phase(JobPhase::Speakers, 1_000);
        });
        let steps = parse_script("stop@speakers").unwrap();
        let lines: StdMutex<Vec<Value>> = StdMutex::new(Vec::new());
        let results = run_script(&jobs2, &steps, Duration::from_secs(3), &|v| {
            lines.lock().unwrap().push(v)
        });
        switcher.join().unwrap();
        assert_eq!(results, vec![(ScriptAction::Stop, "ok".to_string())]);
        assert!(guard.is_stopped());

        // Ein Auftrag, der nie die Phase erreicht und endet: kein Haengen.
        let jobs = MeetingJobs::new();
        let guard = jobs.try_start("m2", Arc::new(|_| {})).unwrap();
        guard.begin_phase(JobPhase::Transcription, 1_000);
        drop(guard);
        let lines: StdMutex<Vec<Value>> = StdMutex::new(Vec::new());
        let steps = parse_script("stop@speakers").unwrap();
        // Ohne Auftrag wartet das Skript auf einen, hier gibt es keinen mehr.
        let results = run_script(&jobs, &steps, Duration::from_millis(100), &|v| {
            lines.lock().unwrap().push(v)
        });
        assert!(results.is_empty());
    }

    #[test]
    fn a_broken_script_is_refused_with_a_reason() {
        assert!(parse_script("").is_err());
        assert!(parse_script("pause")
            .unwrap_err()
            .contains("aktion@sekunden"));
        assert!(parse_script("explode@3")
            .unwrap_err()
            .contains("unbekannte Aktion"));
        assert!(parse_script("pause@abc").is_err());
        assert!(parse_script("pause@-2").is_err());
        assert!(parse_script("pause@NaN").is_err());
    }

    #[test]
    fn the_script_steers_the_running_job_like_the_buttons_do() {
        let jobs = MeetingJobs::new();
        let guard = jobs.try_start("m1", Arc::new(|_| {})).expect("Auftrag");
        guard.begin_phase(JobPhase::Transcription, 100_000);
        let steps = parse_script("pause@0,resume@0.05,stop@0.1").unwrap();
        let lines: StdMutex<Vec<Value>> = StdMutex::new(Vec::new());
        let results = run_script(&jobs, &steps, Duration::from_secs(2), &|v| {
            lines.lock().unwrap().push(v)
        });
        assert_eq!(
            results,
            vec![
                (ScriptAction::Pause, "ok".to_string()),
                (ScriptAction::Resume, "ok".to_string()),
                (ScriptAction::Stop, "ok".to_string()),
            ]
        );
        assert!(guard.is_stopped());
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["action"], "pause");
        assert_eq!(
            lines[0]["state"],
            serde_json::to_value(JobRunState::Pausing).unwrap()
        );
        assert_eq!(
            lines[2]["state"],
            serde_json::to_value(JobRunState::Stopping).unwrap()
        );
    }

    #[test]
    fn a_script_reports_the_command_error_and_gives_up_without_a_job() {
        // Pause in einer Phase, die nicht pausiert: der Fehlercode steht im Beleg.
        let jobs = MeetingJobs::new();
        let guard = jobs.try_start("m1", Arc::new(|_| {})).unwrap();
        guard.begin_phase(JobPhase::Speakers, 1_000);
        let steps = parse_script("pause@0").unwrap();
        let results = run_script(&jobs, &steps, Duration::from_secs(1), &|_| {});
        assert_eq!(
            results,
            vec![(ScriptAction::Pause, "not_pausable".to_string())]
        );

        // Ohne Auftrag: gibt nach der Wartezeit auf, ohne zu haengen.
        let empty = MeetingJobs::new();
        let lines: StdMutex<Vec<Value>> = StdMutex::new(Vec::new());
        let results = run_script(&empty, &steps, Duration::from_millis(80), &|v| {
            lines.lock().unwrap().push(v)
        });
        assert!(results.is_empty());
        assert_eq!(lines.lock().unwrap()[0]["error"], "no_job_appeared");
    }

    #[test]
    fn the_event_log_writes_one_json_line_per_entry_with_the_elapsed_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let log = EventLog::create(&path).unwrap();
        log.write(json!({ "kind": "progress", "done": 1 }));
        log.write(json!({ "kind": "state", "status": "ready" }));
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "progress");
        assert!(lines[1]["t_ms"].is_u64());
    }
}
