use super::injection_state::{ContextKey, PreparedSnapshot, ReplacementPlan, RunState};
use crate::input::{self, EnigoState, ReplacementContext};
use crate::settings::{get_settings, PasteMethod};
use enigo::{Direction, Key, Keyboard};
use log::{debug, warn};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const TARGET_PASTE_SETTLE_DELAY: Duration = Duration::from_millis(120);
const QUEUE_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct InjectionHandle {
    tx: Arc<mpsc::Sender<InjectionCommand>>,
}

enum InjectionCommand {
    Begin {
        run_id: u64,
        refinement_enabled: bool,
    },
    Append {
        run_id: u64,
        fragment: String,
    },
    RegisterSentence {
        run_id: u64,
        sentence_id: u64,
        original: String,
    },
    ReplaceSentence {
        run_id: u64,
        sentence_id: u64,
        original: String,
        candidate: String,
    },
    PrepareFinal {
        run_id: u64,
        reply: mpsc::SyncSender<Option<PreparedSnapshot>>,
    },
    ReplaceFinal {
        run_id: u64,
        snapshot: PreparedSnapshot,
        candidate: String,
        reply: mpsc::SyncSender<bool>,
    },
    Cancel {
        run_id: u64,
    },
}

impl InjectionHandle {
    pub(crate) fn new(app: &AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = app.clone();
        std::thread::spawn(move || run_worker(app, rx));
        Self { tx: Arc::new(tx) }
    }

    pub(crate) fn begin(&self, run_id: u64, refinement_enabled: bool) {
        let _ = self.tx.send(InjectionCommand::Begin {
            run_id,
            refinement_enabled,
        });
    }

    pub(crate) fn append(&self, run_id: u64, fragment: String) {
        let _ = self.tx.send(InjectionCommand::Append { run_id, fragment });
    }

    pub(crate) fn register_sentence(&self, run_id: u64, sentence_id: u64, original: String) {
        let _ = self.tx.send(InjectionCommand::RegisterSentence {
            run_id,
            sentence_id,
            original,
        });
    }

    pub(crate) fn replace_sentence(
        &self,
        run_id: u64,
        sentence_id: u64,
        original: String,
        candidate: String,
    ) {
        let _ = self.tx.send(InjectionCommand::ReplaceSentence {
            run_id,
            sentence_id,
            original,
            candidate,
        });
    }

    pub(crate) fn prepare_final(&self, run_id: u64) -> Option<PreparedSnapshot> {
        let (reply, result) = mpsc::sync_channel(1);
        self.tx
            .send(InjectionCommand::PrepareFinal { run_id, reply })
            .ok()?;
        result.recv_timeout(QUEUE_REPLY_TIMEOUT).ok().flatten()
    }

    pub(crate) fn replace_final(
        &self,
        run_id: u64,
        snapshot: PreparedSnapshot,
        candidate: String,
    ) -> bool {
        let (reply, result) = mpsc::sync_channel(1);
        if self
            .tx
            .send(InjectionCommand::ReplaceFinal {
                run_id,
                snapshot,
                candidate,
                reply,
            })
            .is_err()
        {
            return false;
        }
        result.recv_timeout(QUEUE_REPLY_TIMEOUT).unwrap_or(false)
    }

    pub(crate) fn cancel(&self, run_id: u64) {
        let _ = self.tx.send(InjectionCommand::Cancel { run_id });
    }
}

fn run_worker(app: AppHandle, rx: mpsc::Receiver<InjectionCommand>) {
    let mut state: Option<RunState> = None;
    while let Ok(command) = rx.recv() {
        match command {
            InjectionCommand::Begin {
                run_id,
                refinement_enabled,
            } => state = Some(RunState::new(run_id, refinement_enabled)),
            InjectionCommand::Append { run_id, fragment } => {
                let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) else {
                    continue;
                };
                let track_context = run.wants_context();
                let before = track_context.then(capture_key).flatten();
                let pasted = paste_fragment(&app, &fragment);
                let after = track_context.then(capture_key).flatten();
                run.record_append(&fragment, before, after, pasted);
            }
            InjectionCommand::RegisterSentence {
                run_id,
                sentence_id,
                original,
            } => {
                if let Some(run) = state.as_mut() {
                    run.register_sentence(run_id, sentence_id, &original);
                }
            }
            InjectionCommand::ReplaceSentence {
                run_id,
                sentence_id,
                original,
                candidate,
            } => {
                let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) else {
                    continue;
                };
                let Some(current) = capture_key() else {
                    run.invalidate();
                    continue;
                };
                let Some(plan) =
                    run.plan_sentence(run_id, sentence_id, &original, &candidate, current)
                else {
                    continue;
                };
                if execute_replacement(&app, run, plan, current) {
                    debug!("Sentence refinement applied");
                }
            }
            InjectionCommand::PrepareFinal { run_id, reply } => {
                let snapshot = state
                    .as_mut()
                    .and_then(|run| run.prepare_final(run_id, capture_key()));
                let _ = reply.send(snapshot);
            }
            InjectionCommand::ReplaceFinal {
                run_id,
                snapshot,
                candidate,
                reply,
            } => {
                let applied = if let Some(run) = state.as_mut().filter(|run| run.is_run(run_id)) {
                    if let Some(current) = capture_key() {
                        if let Some(plan) = run.plan_final(run_id, &snapshot, &candidate, current) {
                            execute_replacement(&app, run, plan, current)
                        } else {
                            false
                        }
                    } else {
                        run.invalidate();
                        false
                    }
                } else {
                    false
                };
                let _ = reply.send(applied);
            }
            InjectionCommand::Cancel { run_id } => {
                if let Some(run) = state.as_mut() {
                    run.cancel(run_id);
                }
            }
        }
    }
}

fn paste_fragment(app: &AppHandle, fragment: &str) -> bool {
    // Stream injection fires every few hundred milliseconds while the user
    // keeps using the machine, so it must honour the configured paste method
    // instead of forcing Ctrl+V on everyone: a held Ctrl is held for the whole
    // desktop, and the user's scrolling then reads as zoom.
    let direct = get_settings(app).paste_method == PasteMethod::Direct;

    if !direct {
        if let Err(error) = app.clipboard().write_text(fragment) {
            warn!("stream injection: clipboard write failed: {error}");
            return false;
        }
    }
    let Some(state) = app.try_state::<EnigoState>() else {
        warn!("stream injection: Enigo not initialised");
        return false;
    };
    let mut enigo = match state.0.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("stream injection: Enigo mutex poisoned, recovering");
            poisoned.into_inner()
        }
    };
    let injected = if direct {
        input::paste_text_direct(&mut enigo, fragment)
    } else {
        input::send_paste_ctrl_v(&mut enigo)
    };
    if let Err(error) = injected {
        warn!("stream injection failed: {error}");
        return false;
    }
    drop(enigo);
    std::thread::sleep(TARGET_PASTE_SETTLE_DELAY);
    true
}

fn execute_replacement(
    app: &AppHandle,
    run: &mut RunState,
    plan: ReplacementPlan,
    expected_context: ContextKey,
) -> bool {
    if let Err(error) = app.clipboard().write_text(&plan.replacement) {
        warn!("text refinement: clipboard write failed: {error}");
        run.invalidate();
        return false;
    }
    let Some(state) = app.try_state::<EnigoState>() else {
        warn!("text refinement: Enigo not initialised");
        run.invalidate();
        return false;
    };
    let mut enigo = match state.0.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("text refinement: Enigo mutex poisoned, recovering");
            poisoned.into_inner()
        }
    };

    if capture_key() != Some(expected_context) {
        run.invalidate();
        return false;
    }
    if let Err(error) = input::send_select_left(&mut enigo, plan.select_chars) {
        warn!("text refinement: selection failed: {error}");
        let _ = enigo.key(Key::RightArrow, Direction::Click);
        run.invalidate();
        return false;
    }
    if let Err(error) = input::send_paste_ctrl_v(&mut enigo) {
        warn!("text refinement: replacement paste failed: {error}");
        let _ = enigo.key(Key::RightArrow, Direction::Click);
        run.invalidate();
        return false;
    }
    drop(enigo);
    std::thread::sleep(TARGET_PASTE_SETTLE_DELAY);

    if capture_key() != Some(expected_context) {
        run.invalidate();
        return false;
    }
    run.commit(plan);
    true
}

fn capture_key() -> Option<ContextKey> {
    input::capture_replacement_context().map(context_key)
}

fn context_key(context: ReplacementContext) -> ContextKey {
    ContextKey {
        foreground: context.foreground,
        focus: context.focus,
        physical_generation: context.physical_generation,
    }
}
