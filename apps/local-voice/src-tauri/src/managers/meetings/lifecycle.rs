//! lva#538 (Runde 2): der Lebenszyklus einer Aufnahme (Start, Abbruch, Stopp, Loeschen)
//! als EIN Zustand unter EINER Sperre.
//!
//! Vorher lag er verteilt auf mehrere Atomics und Sperren (`starting`, `StartInfo`,
//! Zustand, Sitzung). Dazwischen blieben Fenster, in denen ein Stopp "nicht aufgenommen"
//! bekam und der Start danach doch veroeffentlichte, ein Loeschen nichts fand und die
//! Aufnahme erst danach begann, oder ein Stopp `Idle` meldete, solange die Writer noch
//! offen waren. Hier gibt es nur noch diese Phasen:
//!
//! ```text
//! Idle -> Starting{op, Ziel, Abbruchmerker} -> Recording{op, Sitzung} -> Stopping -> Idle
//! ```
//!
//! - **Der Start registriert sich atomar** mit seiner Vorgangs-Id (`begin_start`), bevor er
//!   irgendetwas anderes tut. Ein Abbruch (`cancel_op`) adressiert genau diesen Vorgang;
//!   kommt er VOR der Registrierung (der Stopp-Befehl ueberholt den Start-Befehl), wird die
//!   Id vermerkt, und die Registrierung weist den Start mit `start_cancelled` ab. Ein
//!   Abbruch geht nie verloren.
//! - **Ein Stopp waehrend des Starts gilt verbindlich** (`begin_stop`): er setzt den
//!   Abbruchmerker, der Start veroeffentlicht nichts mehr (`publish_started` prueft ihn
//!   unter derselben Sperre) und raeumt selbst auf.
//! - **Loeschen reserviert** (`reserve_delete`): ab dann nimmt kein Start mehr in diese
//!   Besprechung auf (`meeting_not_found`), ein laufender Start wird abgebrochen, eine
//!   laufende Aufnahme beendet, ein laufender Stopp abgewartet (samt Abschluss: Pipeline
//!   leeren, Writer schliessen, `Finalizing`). Die Reservierung gilt, bis der Aufrufer sie
//!   fallen laesst (nach dem Tombstone). Ein Warten, das ablaeuft, ist ein FEHLER
//!   (`recording_busy`), nie ein stiller Erfolg.
//!
//! Die Sperrreihenfolge ist trivial: es gibt genau eine. Warten laeuft ueber eine
//! `Condvar` auf diese Sperre (kein Polling mit Verzug, kein verlorenes Aufwachen).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

/// Fehlercode: der Start wurde durch einen Stopp bzw. Abbruch beendet.
pub const START_CANCELLED: &str = "start_cancelled";
/// Fehlercode: die Besprechung ist (mittlerweile) geloescht oder wird gerade geloescht.
pub const MEETING_NOT_FOUND: &str = "meeting_not_found";
/// Fehlercode: ein Stopp oder Loeschen konnte nicht rechtzeitig abwarten, bis die
/// Aufnahme ganz zu ist. Nichts wurde geloescht bzw. beendet, ein neuer Versuch ist moeglich.
pub const RECORDING_BUSY: &str = "recording_busy";
/// Fehlercode: der abgebrochene Start ist noch nicht zu Ende (haengt im Treiber).
pub const START_BUSY: &str = "start_busy";

/// lva#538 (Runde 3): so viele Abbrueche, die ihrem Start vorauseilten, haelt der Kern, bis
/// der Start sie verbraucht. Ein BESTAETIGTER Abbruch wird nie wieder vergessen: ist die
/// Grenze erreicht, wird der naechste Abbruch nicht bestaetigt (`CancelOp::Refused`), der
/// aelteste nicht verdraengt.
const MAX_PENDING_CANCELS: usize = 1024;
/// So viele zuletzt registrierte Vorgangs-Ids merkt sich der Kern, damit ein Abbruch, der einem
/// laengst beendeten Start gilt, nicht als "kommt noch" vorgemerkt wird. Das Verdraengen einer
/// alten Id kostet hoechstens einen unnuetzen Merker, nie einen verlorenen Abbruch.
const REGISTERED_MEMORY: usize = 64;

/// Sperre nehmen, auch wenn ein anderer Thread mit ihr abgestuerzt ist. Der Zustand hier
/// ist ein kleines Enum: ein "vergifteter" Wert ist nie halb geschrieben. Wer auf dem
/// Hotkey-Pfad `lock().unwrap()` ruft, nimmt sonst das Diktat mit in die Panik (#15).
pub(super) fn lock_recovering<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Der Start, der gerade laeuft.
struct StartOp {
    op: String,
    /// Die Besprechung, in die er aufnimmt, sobald bekannt (Loeschen vergleicht).
    meeting_id: Option<String>,
    /// Abbruchmerker: Stopp und Loeschen setzen ihn (nur unter der Sperre); jeder Schritt
    /// des Starts und das Warten auf den Treiber lesen ihn lock-frei.
    cancel: Arc<AtomicBool>,
    /// Gesetzt, wenn der Abbruch vom Loeschen kam (Code `meeting_not_found`).
    deleted: Arc<AtomicBool>,
}

/// Die veroeffentlichte Aufnahme.
struct Run<S> {
    op: String,
    meeting_id: String,
    paused: bool,
    session: S,
    /// Der Start raeumt noch auf (Anzeige, Ereignis, Waechter): ein Stopp wartet, sonst
    /// schaltete der Start die Anzeige NACH dem Stopp wieder ein.
    start_tail: bool,
}

enum Phase<S> {
    Idle,
    Starting(StartOp),
    Recording(Run<S>),
    /// Ein Stopp (oder das Loeschen) hat die Sitzung genommen und schliesst die Captures:
    /// solange gilt die Aufnahme als laufend (Diktat-Hotkey, neuer Start, Pause).
    Stopping {
        op: String,
        meeting_id: String,
    },
}

struct Inner<S> {
    phase: Phase<S>,
    /// Abgebrochene Vorgangs-Ids, die sich noch nicht registriert hatten. Bleiben, bis genau
    /// dieser Start sie verbraucht (`begin_start`); keine Verdraengung.
    cancelled: Vec<String>,
    /// Die zuletzt registrierten Vorgangs-Ids (Beweis, dass der Start schon kam und ging).
    registered: VecDeque<String>,
    /// Besprechungen, die gerade geloescht werden (Reservierung bis nach dem Tombstone).
    deleting: Vec<String>,
    /// Die Besprechungen, deren Stopp noch abschliesst (Pipeline leeren, Writer schliessen):
    /// die Captures sind zu und neue Aufnahmen duerfen starten, die Dateien aber noch nicht
    /// geloescht werden. Eine Liste, kein Feld: waehrend der Abschluss einer Aufnahme laeuft,
    /// kann die naechste schon gestartet und beendet sein.
    finalizing: Vec<String>,
    next_op: u64,
}

/// Was ein Abbruch (`cancel_op`) vorgefunden hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CancelOp {
    /// Der Start laeuft: sein Abbruchmerker ist gesetzt, er raeumt selbst auf.
    Starting,
    /// Der Start hat schon veroeffentlicht: der Abbruch ist ein Stopp der Aufnahme.
    Running,
    /// Der Vorgang hat sich noch nicht registriert: die Id ist vermerkt und bleibt es, bis
    /// genau dieser Start sie verbraucht; die Registrierung weist ihn ab.
    Remembered,
    /// Der Vorgang hatte sich registriert und ist laengst zu Ende: nichts zu vermerken, nichts
    /// zu tun.
    Finished,
    /// Zu viele Abbrueche warten auf ihren Start: dieser wird NICHT bestaetigt (und keiner
    /// vergessen). Der Aufrufer meldet es als Fehler, ein neuer Versuch ist moeglich.
    Refused,
}

/// Was ein Stopp vorfindet.
pub(crate) enum StopStep<'a, S> {
    /// Eine laufende Aufnahme: ihre Sitzung und das Ticket (`Idle` erst mit dessen Ende).
    Running(S, StopTicket<'a, S>),
    /// Es lief nur ein Start (noch keine Sitzung): sein Abbruchmerker ist gesetzt, er
    /// raeumt selbst auf und endet mit einem Fehlercode. Die Id des Vorgangs.
    CancelledStart(String),
}

/// Was `delete_step` nach dem Warten vorfindet.
pub(crate) enum DeleteStep<'a, S> {
    /// Nichts nimmt (mehr) in diese Besprechung auf: Writer zu, kein Start, kein Abschluss.
    Free,
    /// Ihre laufende Aufnahme: der Aufrufer beendet sie und fragt danach erneut.
    Running(S, StopTicket<'a, S>),
}

/// Die Zustandsregeln des Recorders mit der Sitzung als Typparameter, damit sie ohne
/// Mikrofon, Modell und `AppHandle` pruefbar sind.
pub(crate) struct RunCore<S> {
    inner: Mutex<Inner<S>>,
    changed: Condvar,
    /// Test-Haken: wie viele Threads gerade in `wait` stehen (zaehlt unter der Sperre hoch, die
    /// das Warten erst freigibt). Wer ihn sieht und dann die Sperre nimmt, weiss: der Wartende
    /// steht in der Condvar, ein Aufwecken geht nicht mehr verloren. Ersetzt Schlafzeiten.
    #[cfg(test)]
    waiters: std::sync::atomic::AtomicUsize,
}

impl<S> RunCore<S> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                phase: Phase::Idle,
                cancelled: Vec::new(),
                registered: VecDeque::new(),
                deleting: Vec::new(),
                finalizing: Vec::new(),
                next_op: 0,
            }),
            changed: Condvar::new(),
            #[cfg(test)]
            waiters: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<S>> {
        lock_recovering(&self.inner)
    }

    /// Wartet auf eine Aenderung (oder `deadline`); gibt die Sperre zurueck.
    fn wait<'a>(
        &self,
        guard: MutexGuard<'a, Inner<S>>,
        deadline: Instant,
    ) -> MutexGuard<'a, Inner<S>> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        #[cfg(test)]
        self.waiters.fetch_add(1, Ordering::SeqCst);
        let guard = match self.changed.wait_timeout(guard, remaining) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
        #[cfg(test)]
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        guard
    }

    fn notify(&self) {
        self.changed.notify_all();
    }

    /// Laeuft eine Aufnahme oder gerade ein Start/Stopp? Hotkey-Pfad: darf nie panikieren.
    pub(crate) fn is_recording(&self) -> bool {
        !matches!(self.lock().phase, Phase::Idle)
    }

    /// Nur `Idle` darf starten.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self.lock().phase, Phase::Idle)
    }

    /// Beginnt einen Start und registriert ihn ATOMAR mit seiner Vorgangs-Id (`op`; ohne
    /// Angabe wird eine erzeugt) und, wenn schon bekannt, seiner Besprechung. Fehler:
    /// `start_cancelled`, wenn dieser Vorgang schon abgebrochen wurde, bevor er sich
    /// registrieren konnte; `already_recording`; `meeting_not_found`, wenn die Besprechung
    /// gerade geloescht wird.
    pub(crate) fn begin_start(
        &self,
        op: Option<&str>,
        target: Option<&str>,
    ) -> Result<StartTicket<'_, S>, String> {
        let mut inner = self.lock();
        let op = match op {
            Some(op) => {
                if let Some(at) = inner.cancelled.iter().position(|c| c == op) {
                    inner.cancelled.swap_remove(at);
                    return Err(START_CANCELLED.to_string());
                }
                op.to_string()
            }
            None => {
                inner.next_op += 1;
                format!("auto-{}", inner.next_op)
            }
        };
        if !matches!(inner.phase, Phase::Idle) {
            return Err("already_recording".to_string());
        }
        if target.is_some_and(|t| inner.deleting.iter().any(|d| d == t)) {
            return Err(MEETING_NOT_FOUND.to_string());
        }
        if inner.registered.len() >= REGISTERED_MEMORY {
            inner.registered.pop_front();
        }
        inner.registered.push_back(op.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let deleted = Arc::new(AtomicBool::new(false));
        inner.phase = Phase::Starting(StartOp {
            op: op.clone(),
            meeting_id: target.map(str::to_string),
            cancel: Arc::clone(&cancel),
            deleted: Arc::clone(&deleted),
        });
        Ok(StartTicket {
            core: self,
            op,
            cancel,
            deleted,
        })
    }

    /// Die Capture laeuft: der Start wird zur Aufnahme, Zustand und Sitzung zusammen unter
    /// der Sperre. `Err` gibt die Sitzung zurueck, wenn der Start inzwischen abgebrochen
    /// wurde (Stopp, Loeschen): dann wird nichts veroeffentlicht, der Aufrufer raeumt sie ab.
    pub(crate) fn publish_started(
        &self,
        ticket: &StartTicket<'_, S>,
        meeting_id: &str,
        session: S,
    ) -> Result<(), S> {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (publish_started)");
        let mut inner = self.lock();
        let ours = matches!(&inner.phase, Phase::Starting(s) if s.op == ticket.op);
        if !ours || ticket.cancel.load(Ordering::Acquire) {
            return Err(session);
        }
        inner.phase = Phase::Recording(Run {
            op: ticket.op.clone(),
            meeting_id: meeting_id.to_string(),
            paused: false,
            session,
            start_tail: true,
        });
        drop(inner);
        self.notify();
        Ok(())
    }

    /// Ein Abbruch, der EINEN Vorgang adressiert (der Befehl "Start abbrechen" mit seiner
    /// Id). Kommt er vor der Registrierung des Starts, wird die Id vermerkt.
    pub(crate) fn cancel_op(&self, op: &str) -> CancelOp {
        let mut inner = self.lock();
        match &inner.phase {
            Phase::Starting(s) if s.op == op => {
                s.cancel.store(true, Ordering::Release);
                CancelOp::Starting
            }
            Phase::Recording(run) if run.op == op => CancelOp::Running,
            Phase::Stopping { op: o, .. } if o == op => CancelOp::Running,
            _ if inner.registered.iter().any(|r| r == op) => CancelOp::Finished,
            _ if inner.cancelled.iter().any(|c| c == op) => CancelOp::Remembered,
            _ if inner.cancelled.len() >= MAX_PENDING_CANCELS => CancelOp::Refused,
            _ => {
                inner.cancelled.push(op.to_string());
                CancelOp::Remembered
            }
        }
    }

    /// Wartet, bis der Start `op` zu Ende ist (`false` nach `deadline`: er haengt noch).
    pub(crate) fn wait_start_end(&self, op: &str, deadline: Instant) -> bool {
        let mut inner = self.lock();
        loop {
            if !matches!(&inner.phase, Phase::Starting(s) if s.op == op) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            inner = self.wait(inner, deadline);
        }
    }

    /// Stopp beginnen: die Sitzung herausnehmen. Der Zustand bleibt `Stopping`, bis das
    /// Ticket beendet wird (`finish`) oder faellt (auch bei Panik). Mit laufendem Start:
    /// der Start wird abgebrochen. Eine Aufnahme, deren Start noch aufraeumt, wird
    /// abgewartet (bis `deadline`, dann `start_busy`). Mit `op` gilt der Stopp nur diesem
    /// Vorgang (Abbruch-Befehl des Frontends): laeuft inzwischen ein anderer, ist es
    /// `not_recording`, nie ein Stopp der falschen Aufnahme.
    pub(crate) fn begin_stop(
        &self,
        op: Option<&str>,
        deadline: Instant,
    ) -> Result<StopStep<'_, S>, String> {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (begin_stop)");
        enum Next {
            NotRecording,
            Cancel(String),
            Take,
            Wait,
        }
        let mut inner = self.lock();
        loop {
            let other = |o: &str| op.is_some_and(|op| op != o);
            let next = match &inner.phase {
                Phase::Idle | Phase::Stopping { .. } => Next::NotRecording,
                Phase::Starting(s) if other(&s.op) => Next::NotRecording,
                Phase::Recording(run) if other(&run.op) => Next::NotRecording,
                Phase::Starting(s) => {
                    s.cancel.store(true, Ordering::Release);
                    Next::Cancel(s.op.clone())
                }
                Phase::Recording(run) if !run.start_tail => Next::Take,
                Phase::Recording(_) => Next::Wait,
            };
            match next {
                Next::NotRecording => return Err("not_recording".to_string()),
                Next::Cancel(op) => return Ok(StopStep::CancelledStart(op)),
                Next::Take => {
                    let (session, ticket) = self.take_session(&mut inner);
                    return Ok(StopStep::Running(session, ticket));
                }
                Next::Wait => {}
            }
            if Instant::now() >= deadline {
                return Err(START_BUSY.to_string());
            }
            inner = self.wait(inner, deadline);
        }
    }

    /// Nimmt die Sitzung der laufenden Aufnahme (Phase `Recording`, Start fertig).
    fn take_session<'a>(&'a self, inner: &mut Inner<S>) -> (S, StopTicket<'a, S>) {
        let Phase::Recording(run) = std::mem::replace(&mut inner.phase, Phase::Idle) else {
            unreachable!("take_session nur in der Phase Recording");
        };
        inner.phase = Phase::Stopping {
            op: run.op,
            meeting_id: run.meeting_id.clone(),
        };
        (
            run.session,
            StopTicket {
                core: self,
                meeting_id: run.meeting_id,
                done: false,
            },
        )
    }

    /// Reserviert das Loeschen der Besprechung `meeting_id`: ab jetzt nimmt kein Start mehr
    /// in sie auf. Fehler: `delete_in_progress`, wenn schon ein Loeschen laeuft.
    pub(crate) fn reserve_delete(&self, meeting_id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.deleting.iter().any(|d| d == meeting_id) {
            return Err("delete_in_progress".to_string());
        }
        inner.deleting.push(meeting_id.to_string());
        Ok(())
    }

    /// Gibt die Reservierung frei (nach dem Tombstone, oder wenn das Loeschen scheitert).
    pub(crate) fn release_delete(&self, meeting_id: &str) {
        let mut inner = self.lock();
        inner.deleting.retain(|d| d != meeting_id);
        drop(inner);
        self.notify();
    }

    /// Bringt die Besprechung `meeting_id` (mit Reservierung) in einen Zustand, in dem ihre
    /// Dateien geloescht werden duerfen: ein laufender Start in sie wird abgebrochen und
    /// abgewartet, ein laufender Stopp samt Abschluss abgewartet. Eine laufende Aufnahme von
    /// ihr kommt als `Running` zurueck (der Aufrufer beendet sie und fragt erneut). Laeuft
    /// das Warten ab, ist es ein Fehler (`recording_busy`), nie ein Erfolg.
    pub(crate) fn delete_step(
        &self,
        meeting_id: &str,
        deadline: Instant,
    ) -> Result<DeleteStep<'_, S>, String> {
        enum Next {
            Free,
            Busy,
            Take,
        }
        let mut inner = self.lock();
        loop {
            let finalizing = inner.finalizing.iter().any(|f| f == meeting_id);
            let next = match &inner.phase {
                Phase::Starting(s) if s.meeting_id.as_deref() == Some(meeting_id) => {
                    s.deleted.store(true, Ordering::Release);
                    s.cancel.store(true, Ordering::Release);
                    Next::Busy
                }
                Phase::Recording(run) if run.meeting_id == meeting_id => {
                    if run.start_tail {
                        Next::Busy
                    } else {
                        Next::Take
                    }
                }
                Phase::Stopping { meeting_id: id, .. } if id == meeting_id => Next::Busy,
                _ if finalizing => Next::Busy,
                _ => Next::Free,
            };
            match next {
                Next::Free => return Ok(DeleteStep::Free),
                Next::Take => {
                    let (session, ticket) = self.take_session(&mut inner);
                    return Ok(DeleteStep::Running(session, ticket));
                }
                Next::Busy => {}
            }
            if Instant::now() >= deadline {
                return Err(RECORDING_BUSY.to_string());
            }
            inner = self.wait(inner, deadline);
        }
    }

    /// Pause/Weiter: nur mit laufender Sitzung. `apply` setzt das Merkzeichen der
    /// Capture-Callbacks unter derselben Sperre.
    pub(crate) fn set_paused(
        &self,
        paused: bool,
        apply: impl FnOnce(&S),
    ) -> Result<String, String> {
        crate::managers::planning::ohne_tor_sperre("Zustand der Aufnahme (set_paused)");
        let mut inner = self.lock();
        // Ohne Sitzung (Startfenster, Stopp laeuft) gibt es nichts zu pausieren; ein
        // Zustandsereignis "pausiert" ohne wirkende Pause waere falsch.
        let Phase::Recording(run) = &mut inner.phase else {
            return Err("not_recording".to_string());
        };
        run.paused = paused;
        apply(&run.session);
        Ok(run.meeting_id.clone())
    }

    /// Ist die laufende Aufnahme pausiert? (`None`: es laeuft keine.)
    #[cfg(test)]
    pub(crate) fn paused(&self) -> Option<bool> {
        match &self.lock().phase {
            Phase::Recording(run) => Some(run.paused),
            _ => None,
        }
    }

    /// Lesezugriff auf die Sitzung (Audioposition fuer den Notizblock).
    pub(crate) fn with_session<R>(&self, f: impl FnOnce(Option<&S>) -> R) -> R {
        let inner = self.lock();
        match &inner.phase {
            Phase::Recording(run) => f(Some(&run.session)),
            _ => f(None),
        }
    }

    /// Das Ticket eines Stopps endet: `Idle`; mit `finalizing` merkt sich der Kern, dass
    /// der Abschluss dieser Besprechung noch laeuft.
    fn release_stopping(&self, meeting_id: &str, finalizing: bool) {
        let mut inner = self.lock();
        if matches!(&inner.phase, Phase::Stopping { meeting_id: id, .. } if id == meeting_id) {
            inner.phase = Phase::Idle;
        }
        if finalizing {
            inner.finalizing.push(meeting_id.to_string());
        }
        drop(inner);
        self.notify();
    }

    fn release_finalizing(&self, meeting_id: &str) {
        let mut inner = self.lock();
        if let Some(at) = inner.finalizing.iter().position(|f| f == meeting_id) {
            inner.finalizing.remove(at);
        }
        drop(inner);
        self.notify();
    }
}

/// Das Ticket eines laufenden Starts: traegt seine Vorgangs-Id und seinen Abbruchmerker
/// und gibt den Kern mit seinem Ende (auch bei jedem Fehlerweg und bei Panik) wieder frei.
pub(crate) struct StartTicket<'a, S> {
    core: &'a RunCore<S>,
    op: String,
    cancel: Arc<AtomicBool>,
    deleted: Arc<AtomicBool>,
}

impl<S> StartTicket<'_, S> {
    /// Die Vorgangs-Id dieses Starts.
    #[cfg(test)]
    pub(crate) fn op(&self) -> &str {
        &self.op
    }

    /// Der Abbruchmerker: bis in das Warten auf den Treiber.
    pub(crate) fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Wurde der Start abgebrochen (Stopp, Loeschen)?
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Der Fehlercode eines abgebrochenen Starts: das Loeschen der Besprechung heisst
    /// `meeting_not_found`, ein Stopp `start_cancelled`.
    pub(crate) fn cancel_code(&self) -> &'static str {
        if self.deleted.load(Ordering::Acquire) {
            MEETING_NOT_FOUND
        } else {
            START_CANCELLED
        }
    }

    /// `Err(code)`, wenn der Start abgebrochen wurde: vor jedem Schritt, der etwas anlegt.
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(self.cancel_code().to_string())
        } else {
            Ok(())
        }
    }

    /// Merkt die Besprechung, in die dieser Start aufnimmt (das Loeschen sucht sie). Wird
    /// sie gerade geloescht, ist der Start abgebrochen (`meeting_not_found`).
    pub(crate) fn set_target(&self, meeting_id: &str) -> Result<(), String> {
        let mut inner = self.core.lock();
        if inner.deleting.iter().any(|d| d == meeting_id) {
            self.deleted.store(true, Ordering::Release);
            self.cancel.store(true, Ordering::Release);
            return Err(MEETING_NOT_FOUND.to_string());
        }
        if let Phase::Starting(s) = &mut inner.phase {
            if s.op == self.op {
                s.meeting_id = Some(meeting_id.to_string());
            }
        }
        Ok(())
    }
}

impl<S> Drop for StartTicket<'_, S> {
    fn drop(&mut self) {
        let mut inner = self.core.lock();
        let mine = match &mut inner.phase {
            Phase::Starting(s) if s.op == self.op => Some(true),
            Phase::Recording(run) if run.op == self.op => {
                run.start_tail = false;
                Some(false)
            }
            _ => None,
        };
        if mine == Some(true) {
            inner.phase = Phase::Idle;
        }
        drop(inner);
        self.core.notify();
    }
}

/// Haelt die Phase `Stopping`, bis der Stopp die Capture wirklich beendet hat. Faellt es
/// (Panik im Stopp), wird der Zustand trotzdem frei: sonst bliebe eine "Geisteraufnahme"
/// zurueck, die Diktat und Neustart bis zum App-Neustart sperrt.
pub(crate) struct StopTicket<'a, S> {
    core: &'a RunCore<S>,
    meeting_id: String,
    done: bool,
}

impl<'a, S> StopTicket<'a, S> {
    /// Die Capture ist zu: der Zustand wird `Idle`, der Abschluss (Pipeline leeren, Writer
    /// schliessen) laeuft aber weiter und haelt `Finalizing`, bis der Aufrufer es fallen
    /// laesst: ein Loeschen dieser Besprechung wartet darauf.
    pub(crate) fn finish(mut self) -> Finalizing<'a, S> {
        self.done = true;
        self.core.release_stopping(&self.meeting_id, true);
        Finalizing {
            core: self.core,
            meeting_id: self.meeting_id.clone(),
        }
    }
}

impl<S> Drop for StopTicket<'_, S> {
    fn drop(&mut self) {
        if !self.done {
            self.core.release_stopping(&self.meeting_id, false);
        }
    }
}

/// Der Abschluss eines Stopps (nach geschlossenen Captures) laeuft noch.
pub(crate) struct Finalizing<'a, S> {
    core: &'a RunCore<S>,
    meeting_id: String,
}

impl<S> Drop for Finalizing<'_, S> {
    fn drop(&mut self) {
        self.core.release_finalizing(&self.meeting_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    type Core = RunCore<&'static str>;

    /// Wartet (ohne Schlafzeit), bis `n` Threads in der Condvar des Kerns stehen: danach geht
    /// kein Aufwecken mehr verloren, und das Ergebnis haengt von keiner Zeit ab.
    fn until_waiting(core: &Core, n: usize) {
        let started = Instant::now();
        while core.waiters.load(Ordering::SeqCst) < n {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "niemand wartet in der Condvar"
            );
            std::thread::yield_now();
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_millis(40)
    }

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    /// Ein Stopp, der eine laufende Aufnahme vorfindet (sonst schlaegt der Test fehl).
    fn running(core: &Core) -> (&'static str, StopTicket<'_, &'static str>) {
        match core.begin_stop(None, later()) {
            Ok(StopStep::Running(session, ticket)) => (session, ticket),
            Ok(StopStep::CancelledStart(_)) => panic!("kein Start, sondern eine Aufnahme erwartet"),
            Err(e) => panic!("Stopp abgewiesen: {e}"),
        }
    }

    /// Startet und veroeffentlicht eine Aufnahme `m1`; das Ticket des Starts ist danach zu Ende.
    fn recording(core: &Core, meeting: &str) {
        let starting = core.begin_start(None, Some(meeting)).unwrap();
        core.publish_started(&starting, meeting, "sitzung").unwrap();
        drop(starting);
    }

    // ---- #15: Zustandsfenster, Vergiftung, Geisteraufnahme -----------------------------

    #[test]
    fn a_poisoned_lock_does_not_take_the_hotkey_path_down() {
        let core = Arc::new(Core::new());
        let poisoner = Arc::clone(&core);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.lock().unwrap();
            panic!("Absturz mit gehaltener Sperre");
        })
        .join();
        assert!(core.inner.is_poisoned(), "Voraussetzung des Tests");
        // Der Diktat-Hotkey fragt genau das: keine Panik, richtige Antwort.
        assert!(!core.is_recording());
        assert!(core.is_idle());
        assert!(
            core.begin_start(None, None).is_ok(),
            "ein Start bleibt moeglich"
        );
    }

    #[test]
    fn a_poisoned_lock_does_not_break_pause_or_position() {
        let core = Arc::new(Core::new());
        recording(&core, "m1");
        let poisoner = Arc::clone(&core);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.lock().unwrap();
            panic!("Absturz mit gehaltener Sperre");
        })
        .join();
        assert_eq!(core.with_session(|s| s.copied()), Some("sitzung"));
        assert_eq!(core.set_paused(true, |_| {}), Ok("m1".to_string()));
    }

    #[test]
    fn the_state_stays_stopping_until_the_stop_has_closed_the_capture() {
        let core = Core::new();
        recording(&core, "m1");
        let (session, ticket) = running(&core);
        assert_eq!(session, "sitzung");
        // Das Mikrofon ist noch offen: Diktat-Hotkey und neuer Start sehen es.
        assert!(
            core.is_recording(),
            "Fenster 'Zustand Idle vor Capture-Stopp' ist zu"
        );
        assert_eq!(
            core.begin_start(None, None).err(),
            Some("already_recording".to_string())
        );
        // Ein zweiter Stopp hat nichts mehr zu stoppen.
        assert_eq!(
            core.begin_stop(None, soon()).err(),
            Some("not_recording".to_string())
        );
        let finalizing = ticket.finish();
        assert!(!core.is_recording());
        assert!(core.begin_start(None, None).is_ok());
        drop(finalizing);
    }

    #[test]
    fn a_panic_while_stopping_still_frees_the_state() {
        let core = Core::new();
        recording(&core, "m1");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_session, _ticket) = running(&core);
            panic!("Capture-Stopp stuerzt ab");
        }));
        assert!(result.is_err());
        assert!(
            !core.is_recording(),
            "keine Geisteraufnahme, die Diktat und Neustart sperrt"
        );
        // Ein Stopp, der nie abschloss, haelt auch kein `Finalizing` fest.
        assert!(matches!(
            core.delete_step("m1", soon()),
            Ok(DeleteStep::Free)
        ));
    }

    #[test]
    fn a_start_in_progress_counts_as_recording_and_refuses_a_second_start() {
        let core = Core::new();
        let ticket = core.begin_start(None, None).unwrap();
        assert!(
            core.is_recording(),
            "waehrend des Starts gilt es schon als Aufnahme"
        );
        assert_eq!(
            core.begin_start(None, None).err(),
            Some("already_recording".to_string())
        );
        drop(ticket);
        assert!(
            !core.is_recording(),
            "auch ein gescheiterter Start gibt es frei"
        );
    }

    #[test]
    fn pause_needs_a_running_session_not_just_a_state() {
        let core = Core::new();
        // Weder vor dem Start noch im Startfenster (Capture laeuft, Sitzung fehlt noch).
        assert_eq!(
            core.set_paused(true, |_| {}),
            Err("not_recording".to_string())
        );
        let starting = core.begin_start(None, Some("m1")).unwrap();
        assert_eq!(
            core.set_paused(true, |_| {}),
            Err("not_recording".to_string())
        );
        core.publish_started(&starting, "m1", "sitzung").unwrap();
        drop(starting);
        let applied = std::cell::Cell::new(false);
        assert_eq!(
            core.set_paused(true, |s| {
                assert_eq!(*s, "sitzung");
                applied.set(true);
            }),
            Ok("m1".to_string())
        );
        assert!(applied.get());
        assert_eq!(core.paused(), Some(true));
        assert_eq!(core.set_paused(false, |_| {}), Ok("m1".to_string()));
        assert_eq!(
            core.paused(),
            Some(false),
            "Fortsetzen nimmt die Pause zurueck"
        );
        // Nach dem Stopp-Beginn gibt es nichts mehr zu pausieren (kein falsches Ereignis).
        let (_s, ticket) = running(&core);
        assert_eq!(
            core.set_paused(false, |_| {}),
            Err("not_recording".to_string())
        );
        drop(ticket.finish());
    }

    /// Start und Stopp im Wettlauf: nie darf eine Aufnahme ohne Sitzung (oder eine Sitzung ohne
    /// Zustand) zurueckbleiben.
    #[test]
    fn start_and_stop_racing_never_leave_a_ghost_recording() {
        for round in 0..200 {
            let core = Arc::new(Core::new());
            let starting = core.begin_start(None, Some("m1")).unwrap();
            let stopper = Arc::clone(&core);
            let stop = std::thread::spawn(move || {
                // Mehrere Versuche, damit mindestens einer nach dem Start liegt.
                for _ in 0..50 {
                    if let Ok(StopStep::Running(_s, ticket)) = stopper.begin_stop(None, soon()) {
                        drop(ticket.finish());
                        return true;
                    }
                    std::thread::yield_now();
                }
                false
            });
            let _ = core.publish_started(&starting, "m1", "sitzung");
            drop(starting);
            let stopped = stop.join().unwrap();
            if !stopped {
                // Der Stopp kam nie dazwischen: aufraeumen wie der echte Stopp.
                if let Ok(StopStep::Running(_s, ticket)) = core.begin_stop(None, soon()) {
                    drop(ticket.finish());
                }
            }
            assert!(
                !core.is_recording(),
                "Runde {round}: Zustand Recording ohne Sitzung (Geisteraufnahme)"
            );
        }
    }

    /// Befund 4, Runde 3: die Reservierung ueberdauert den Abschluss der Aufnahme und endet erst
    /// NACH dem Tombstone. Ausdruecklich sequenziert (explizite Barriere statt Schlafzeit): der
    /// Abschluss wird vom Test gehalten, der Loeschwartepunkt ist beobachtet (`until_waiting`),
    /// ein Start in dieselbe Besprechung wird abgelehnt; dann gibt der Test den Abschluss frei,
    /// setzt den Tombstone und laesst die Reservierung erst danach fallen.
    #[test]
    fn the_reservation_outlives_the_finalizing_and_ends_after_the_tombstone() {
        let core = Arc::new(Core::new());
        let starting = core.begin_start(Some("op-1"), Some("a")).unwrap();
        core.publish_started(&starting, "a", "sitzung").unwrap();
        drop(starting);
        let (_session, ticket) = running(&core);
        let finalizing = ticket.finish();

        core.reserve_delete("a").unwrap();
        let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let waiter = Arc::clone(&core);
        let order = Arc::clone(&log);
        let deleting = std::thread::spawn(move || {
            let free = matches!(
                waiter.delete_step("a", Instant::now() + Duration::from_secs(60)),
                Ok(DeleteStep::Free)
            );
            order.lock().unwrap().push("delete_frei");
            free
        });
        until_waiting(&core, 1);
        assert_eq!(
            core.begin_start(Some("op-2"), Some("a")).err(),
            Some(MEETING_NOT_FOUND.to_string()),
            "waehrend des Abschlusses nimmt kein Start in diese Besprechung auf"
        );

        log.lock().unwrap().push("abschluss_fertig");
        drop(finalizing);
        assert!(
            deleting.join().unwrap(),
            "das Loeschen kam erst nach dem Abschluss zum Zug"
        );

        log.lock().unwrap().push("tombstone");
        assert_eq!(
            core.begin_start(Some("op-3"), Some("a")).err(),
            Some(MEETING_NOT_FOUND.to_string()),
            "auch nach dem Abschluss und dem Tombstone haelt die Reservierung noch"
        );
        core.release_delete("a");
        log.lock().unwrap().push("reservierung_frei");
        assert!(core.begin_start(Some("op-4"), Some("a")).is_ok());
        assert_eq!(
            *log.lock().unwrap(),
            [
                "abschluss_fertig",
                "delete_frei",
                "tombstone",
                "reservierung_frei"
            ]
        );
    }

    // ---- lva#538: Stopp und Abbruch waehrend des Starts --------------------------------

    /// Ein Stopp, der waehrend des Starts kommt (noch keine Sitzung), wird nicht als "nicht
    /// aufgenommen" abgewiesen: er setzt den Abbruchmerker, und die Veroeffentlichung danach
    /// gibt die Sitzung zurueck, statt eine Aufnahme zu veroeffentlichen, die der Nutzer schon
    /// abgebrochen hat.
    #[test]
    fn a_stop_during_a_start_cancels_it_and_nothing_is_published() {
        let core = Core::new();
        let starting = core.begin_start(None, None).unwrap();
        assert!(!starting.cancelled());

        assert!(matches!(
            core.begin_stop(None, soon()),
            Ok(StopStep::CancelledStart(op)) if op == starting.op()
        ));
        assert!(starting.cancelled(), "der Start sieht den Abbruch");
        assert_eq!(starting.check(), Err(START_CANCELLED.to_string()));

        // Der Start war schon bei der Veroeffentlichung: er bekommt seine Sitzung zurueck.
        assert_eq!(
            core.publish_started(&starting, "m1", "sitzung"),
            Err("sitzung")
        );
        assert!(
            core.with_session(|s| s.is_none()),
            "keine Sitzung veroeffentlicht"
        );
        assert!(
            core.is_recording(),
            "der Start laeuft, bis er aufgeraeumt hat"
        );

        drop(starting);
        assert!(core.is_idle());
        // Ohne Start und ohne Aufnahme bleibt es bei `not_recording`.
        assert_eq!(
            core.begin_stop(None, soon()).err(),
            Some("not_recording".to_string())
        );
        // Der naechste Start ist unberuehrt vom alten Abbruch.
        let next = core.begin_start(None, None).unwrap();
        assert!(!next.cancelled());
        assert!(core.publish_started(&next, "m2", "neu").is_ok());
    }

    /// Stopp und Veroeffentlichung im Wettlauf: entweder sieht der Stopp die Sitzung (und kann
    /// sie beenden), oder die Veroeffentlichung sieht den Abbruch. Nie laeuft eine Aufnahme
    /// weiter, obwohl der Stopp "erfolgreich" war.
    #[test]
    fn a_stop_racing_the_publish_either_stops_the_recording_or_cancels_the_start() {
        for round in 0..300 {
            let core = Arc::new(Core::new());
            let starting = core.begin_start(None, Some("m1")).unwrap();
            let stopper = Arc::clone(&core);
            let stop = std::thread::spawn(move || {
                matches!(
                    stopper.begin_stop(None, Instant::now() + Duration::from_millis(200)),
                    Ok(StopStep::CancelledStart(_))
                )
            });
            let published = core.publish_started(&starting, "m1", "sitzung").is_ok();
            // Der Start ist mit der Veroeffentlichung fertig: ein Stopp, der auf sein Ende
            // wartet, kommt jetzt zum Zug (kein Warten bis zur Frist).
            drop(starting);
            let cancelled_start = stop.join().unwrap();
            if published {
                // Der Stopp kam NACH der Veroeffentlichung (er wartete auf das Ende des Starts):
                // die Sitzung ist da und stoppbar.
                assert!(!cancelled_start, "Runde {round}");
                if let Ok(StopStep::Running(_s, ticket)) = core.begin_stop(None, soon()) {
                    drop(ticket.finish());
                }
            } else {
                assert!(cancelled_start, "Runde {round}: abgelehnt ohne Abbruch");
            }
            assert!(!core.is_recording(), "Runde {round}: Geisteraufnahme");
        }
    }

    /// Eine Aufnahme, deren Start noch aufraeumt (Anzeige, Ereignis), wird nicht schon
    /// gestoppt: der Stopp wartet (sonst schaltete der Start die Anzeige NACH dem Stopp wieder
    /// ein). Laeuft das Warten ab, ist es ein Fehler (`start_busy`).
    #[test]
    fn a_stop_right_after_the_publish_waits_for_the_start_to_finish() {
        let core = Arc::new(Core::new());
        let starting = core.begin_start(None, Some("m1")).unwrap();
        core.publish_started(&starting, "m1", "sitzung").unwrap();
        assert_eq!(
            core.begin_stop(None, soon()).err(),
            Some(START_BUSY.to_string()),
            "ein Warten, das ablaeuft, ist ein Fehler"
        );
        assert!(
            core.with_session(|s| s.is_some()),
            "die Sitzung blieb stehen"
        );
        // Mit dem Ende des Starts kommt der wartende Stopp zum Zug (Condvar, kein Polling).
        let stopper = Arc::clone(&core);
        let waiting = std::thread::spawn(move || {
            matches!(
                stopper.begin_stop(None, Instant::now() + Duration::from_secs(10)),
                Ok(StopStep::Running(..))
            )
        });
        until_waiting(&core, 1);
        drop(starting);
        assert!(waiting.join().unwrap());
    }

    /// Befund 2: der Abbruch kommt VOR der Registrierung des Starts (der Stopp-Befehl ueberholt
    /// den Start-Befehl). Er wird vermerkt, und der Start endet bei der Registrierung mit
    /// `start_cancelled`: er geht nicht verloren, und es entsteht keine Aufnahme.
    #[test]
    fn a_cancel_before_the_registration_is_remembered_and_refuses_that_start() {
        let core = Core::new();
        assert_eq!(core.cancel_op("op-1"), CancelOp::Remembered);
        assert_eq!(
            core.begin_start(Some("op-1"), Some("m1")).err(),
            Some(START_CANCELLED.to_string())
        );
        assert!(core.is_idle(), "nichts wurde registriert");
        // Der Vermerk ist verbraucht, andere Vorgaenge waren nie betroffen.
        assert!(core.begin_start(Some("op-2"), None).is_ok());
    }

    /// Abbruch und Registrierung im Wettlauf: nie "registriert UND nicht abgebrochen".
    #[test]
    fn a_cancel_racing_the_registration_never_gets_lost() {
        for round in 0..500 {
            let core = Arc::new(Core::new());
            let canceller = Arc::clone(&core);
            let cancel = std::thread::spawn(move || canceller.cancel_op("op"));
            let started = core.begin_start(Some("op"), Some("m1"));
            let cancel_found = cancel.join().unwrap();
            match started {
                Err(e) => {
                    assert_eq!(e, START_CANCELLED, "Runde {round}");
                    assert_eq!(cancel_found, CancelOp::Remembered, "Runde {round}");
                }
                Ok(ticket) => {
                    assert!(
                        ticket.cancelled(),
                        "Runde {round}: der Start laeuft, obwohl er abgebrochen wurde"
                    );
                    assert_eq!(cancel_found, CancelOp::Starting, "Runde {round}");
                }
            }
        }
    }

    /// Hat der Start schon veroeffentlicht, ist der Abbruch ein Stopp GENAU dieser Aufnahme.
    #[test]
    fn a_cancel_of_a_published_start_stops_that_recording_only() {
        let core = Core::new();
        let starting = core.begin_start(Some("op-1"), Some("m1")).unwrap();
        core.publish_started(&starting, "m1", "sitzung").unwrap();
        drop(starting);
        assert_eq!(core.cancel_op("op-1"), CancelOp::Running);
        // Eine fremde Vorgangs-Id stoppt nichts.
        assert_eq!(
            core.begin_stop(Some("op-2"), soon()).err(),
            Some("not_recording".to_string())
        );
        assert!(core.with_session(|s| s.is_some()));
        let Ok(StopStep::Running(_, ticket)) = core.begin_stop(Some("op-1"), soon()) else {
            panic!("die Aufnahme von op-1 muss stoppbar sein");
        };
        drop(ticket.finish());
    }

    /// Befund A (Runde 3): ein BESTAETIGTER Abbruch geht nie verloren, wie viele andere auch
    /// danach kommen (vorher verdraengte der 33. den aeltesten, und dessen Start lief an).
    /// Ausdruecklich sequenziert, ohne Zeit.
    #[test]
    fn a_confirmed_cancel_survives_any_number_of_later_cancels() {
        let core = Core::new();
        assert_eq!(core.cancel_op("op-0"), CancelOp::Remembered);
        for k in 1..=300 {
            assert_eq!(core.cancel_op(&format!("op-{k}")), CancelOp::Remembered);
        }
        assert_eq!(
            core.begin_start(Some("op-0"), Some("m1")).err(),
            Some(START_CANCELLED.to_string()),
            "der erste Abbruch gilt noch nach 300 weiteren"
        );
        for k in 1..=300 {
            assert_eq!(
                core.begin_start(Some(&format!("op-{k}")), None).err(),
                Some(START_CANCELLED.to_string()),
                "op-{k}"
            );
        }
        assert!(core.lock().cancelled.is_empty(), "jeder Merker verbraucht");
        assert!(core.is_idle(), "keiner dieser Starts wurde registriert");
    }

    /// 33 ausstehende Vorgaenge (eins mehr als die alte Grenze): alle werden abgewiesen.
    #[test]
    fn thirty_three_pending_operations_are_all_cancelled() {
        let core = Core::new();
        for k in 0..33 {
            core.cancel_op(&format!("op-{k}"));
        }
        for k in 0..33 {
            assert_eq!(
                core.begin_start(Some(&format!("op-{k}")), None).err(),
                Some(START_CANCELLED.to_string()),
                "op-{k}"
            );
        }
    }

    /// An der Grenze wird der naechste Abbruch NICHT bestaetigt (Fehler, neuer Versuch), und
    /// keiner der gemerkten wird verdraengt. Verbraucht ein Start einen, ist wieder Platz.
    #[test]
    fn at_the_limit_a_new_cancel_is_refused_and_no_old_one_is_forgotten() {
        let core = Core::new();
        for k in 0..MAX_PENDING_CANCELS {
            assert_eq!(core.cancel_op(&format!("op-{k}")), CancelOp::Remembered);
        }
        assert_eq!(core.cancel_op("zu-viel"), CancelOp::Refused);
        assert_eq!(
            core.cancel_op("op-0"),
            CancelOp::Remembered,
            "ein bekannter Abbruch bleibt bestaetigt"
        );
        assert_eq!(
            core.begin_start(Some("op-0"), None).err(),
            Some(START_CANCELLED.to_string()),
            "der aelteste ist nicht verdraengt"
        );
        assert_eq!(
            core.cancel_op("zu-viel"),
            CancelOp::Remembered,
            "wieder Platz"
        );
        assert_eq!(
            core.begin_start(Some("zu-viel"), None).err(),
            Some(START_CANCELLED.to_string())
        );
    }

    /// Ein Abbruch fuer einen Start, der schon kam und ging, wird nicht als "kommt noch"
    /// vorgemerkt (sonst fuellte jeder spaete Abbruch die Merkliste).
    #[test]
    fn a_cancel_for_a_finished_start_is_not_remembered() {
        let core = Core::new();
        for k in 0..(3 * MAX_PENDING_CANCELS) {
            let op = format!("done-{k}");
            drop(core.begin_start(Some(&op), None).unwrap());
            assert_eq!(core.cancel_op(&op), CancelOp::Finished, "{op}");
        }
        assert!(core.lock().cancelled.is_empty());
    }

    /// Der Abbruch NACH der Registrierung trifft den laufenden Start, ausdruecklich sequenziert:
    /// registriert, abgebrochen, der Start sieht es, die Veroeffentlichung gibt die Sitzung zurueck.
    #[test]
    fn a_cancel_after_the_registration_hits_the_running_start() {
        let core = Core::new();
        let starting = core.begin_start(Some("op-1"), Some("m1")).unwrap();
        assert_eq!(core.cancel_op("op-1"), CancelOp::Starting);
        assert!(starting.cancelled());
        assert_eq!(
            core.publish_started(&starting, "m1", "sitzung"),
            Err("sitzung")
        );
        drop(starting);
        assert!(core.is_idle());
    }

    #[test]
    fn waiting_for_a_start_ends_with_it_or_with_the_deadline() {
        let core = Arc::new(Core::new());
        let starting = core.begin_start(Some("op"), None).unwrap();
        assert!(!core.wait_start_end("op", soon()), "er haengt noch");
        let waiter = Arc::clone(&core);
        let ended = std::thread::spawn(move || {
            waiter.wait_start_end("op", Instant::now() + Duration::from_secs(10))
        });
        until_waiting(&core, 1);
        drop(starting);
        assert!(ended.join().unwrap());
    }

    // ---- lva#538: Loeschen ----------------------------------------------------------------

    #[test]
    fn a_delete_cancels_only_the_start_of_that_meeting() {
        let core = Arc::new(Core::new());
        let starting = core.begin_start(None, None).unwrap();
        starting.set_target("a").unwrap();

        // Eine fremde Besprechung: nichts zu tun, der Start bleibt.
        core.reserve_delete("b").unwrap();
        assert!(matches!(
            core.delete_step("b", soon()),
            Ok(DeleteStep::Free)
        ));
        assert!(!starting.cancelled());

        core.reserve_delete("a").unwrap();
        let waiter = Arc::clone(&core);
        let deleting = std::thread::spawn(move || {
            matches!(
                waiter.delete_step("a", Instant::now() + Duration::from_secs(10)),
                Ok(DeleteStep::Free)
            )
        });
        // Das Loeschen setzt den Abbruchmerker des Starts und wartet auf sein Ende.
        let waited = Instant::now();
        while !starting.cancelled() && waited.elapsed() < Duration::from_secs(5) {
            std::thread::yield_now();
        }
        assert!(starting.cancelled());
        assert_eq!(
            starting.cancel_code(),
            MEETING_NOT_FOUND,
            "geloescht, nicht gestoppt"
        );
        assert_eq!(
            core.publish_started(&starting, "a", "sitzung"),
            Err("sitzung")
        );
        drop(starting);
        assert!(
            deleting.join().unwrap(),
            "das Loeschen wartete auf das Ende"
        );
    }

    #[test]
    fn a_stop_gives_the_start_the_code_start_cancelled_not_meeting_not_found() {
        let core = Core::new();
        let starting = core.begin_start(None, Some("a")).unwrap();
        assert!(matches!(
            core.begin_stop(None, soon()),
            Ok(StopStep::CancelledStart(_))
        ));
        assert_eq!(starting.cancel_code(), START_CANCELLED);
    }

    /// Befund 4, erstes Gegenbeispiel: ein Start, der NACH der Pruefung des Loeschens kaeme, wird
    /// abgewiesen, solange die Reservierung gilt (`meeting_not_found`); eine andere Besprechung
    /// bleibt startbar. Mit der Reservierung fuer ein Ziel, das ein Start schon traegt, endet er.
    #[test]
    fn a_reserved_delete_refuses_every_start_into_that_meeting() {
        let core = Core::new();
        core.reserve_delete("a").unwrap();
        assert_eq!(
            core.begin_start(None, Some("a")).err(),
            Some(MEETING_NOT_FOUND.to_string())
        );
        // Ein Start ohne Ziel legt erst spaeter an: `set_target` weist ihn dann ab.
        let starting = core.begin_start(None, None).unwrap();
        assert_eq!(starting.set_target("a"), Err(MEETING_NOT_FOUND.to_string()));
        assert!(starting.cancelled());
        assert_eq!(starting.cancel_code(), MEETING_NOT_FOUND);
        drop(starting);
        // Eine andere Besprechung ist nicht betroffen.
        assert!(core.begin_start(None, Some("b")).is_ok());
        // Ein zweites Loeschen derselben Besprechung wird abgewiesen.
        assert_eq!(
            core.reserve_delete("a").err(),
            Some("delete_in_progress".to_string())
        );
        // Erst mit dem Ende des Loeschens nimmt wieder ein Start in sie auf.
        core.release_delete("a");
        assert!(core.begin_start(None, Some("a")).is_ok());
    }

    /// Befund 4, zweites Gegenbeispiel: ein normaler Stopp setzt `Idle`, bevor Pipeline und
    /// Writer fertig sind. Das Loeschen wartet bis zum Ende des Abschlusses.
    #[test]
    fn a_delete_waits_for_the_stop_to_finish_closing_the_recording() {
        let core = Arc::new(Core::new());
        recording(&core, "a");
        core.reserve_delete("a").unwrap();
        let (_session, ticket) = running(&core);
        // Captures noch offen: Stopping.
        assert_eq!(
            core.delete_step("a", soon()).err(),
            Some(RECORDING_BUSY.to_string())
        );
        // Captures zu, `Idle`, aber der Abschluss laeuft noch (Pipeline leeren, Writer zu).
        let finalizing = ticket.finish();
        assert!(core.is_idle());
        assert_eq!(
            core.delete_step("a", soon()).err(),
            Some(RECORDING_BUSY.to_string()),
            "Idle bedeutet nicht, dass die Writer zu sind"
        );
        // Andere Besprechungen warten nicht darauf.
        assert!(matches!(
            core.delete_step("z", soon()),
            Ok(DeleteStep::Free)
        ));
        // Mit dem Ende des Abschlusses ist das Loeschen frei (Condvar, nicht Polling).
        let waiter = Arc::clone(&core);
        let deleting = std::thread::spawn(move || {
            matches!(
                waiter.delete_step("a", Instant::now() + Duration::from_secs(10)),
                Ok(DeleteStep::Free)
            )
        });
        until_waiting(&core, 1);
        drop(finalizing);
        assert!(deleting.join().unwrap());
    }

    /// Befund 4, drittes Gegenbeispiel: das Warten laeuft ab. Das ist ein FEHLER, nie ein
    /// Erfolg, damit die Dateien nicht unter einer laufenden Aufnahme verschwinden.
    #[test]
    fn a_wait_that_runs_out_is_an_error_never_a_success() {
        let core = Core::new();
        let starting = core.begin_start(None, Some("a")).unwrap();
        core.reserve_delete("a").unwrap();
        assert_eq!(
            core.delete_step("a", soon()).err(),
            Some(RECORDING_BUSY.to_string())
        );
        // Der Start wurde dabei abgebrochen (das Loeschen ist verbindlich) ...
        assert!(starting.cancelled());
        drop(starting);
        // ... und mit seinem Ende ist der naechste Versuch frei.
        assert!(matches!(
            core.delete_step("a", soon()),
            Ok(DeleteStep::Free)
        ));
    }

    #[test]
    fn a_delete_takes_the_running_session_of_that_meeting_only() {
        let core = Core::new();
        recording(&core, "m1");
        core.reserve_delete("m2").unwrap();
        assert!(
            matches!(core.delete_step("m2", soon()), Ok(DeleteStep::Free)),
            "andere Besprechung: nichts"
        );
        assert!(core.with_session(|s| s.is_some()));

        core.reserve_delete("m1").unwrap();
        let Ok(DeleteStep::Running(session, ticket)) = core.delete_step("m1", soon()) else {
            panic!("die laufende Aufnahme gehoert zu m1");
        };
        assert_eq!(session, "sitzung");
        // Bis das Ticket endet, gilt die Aufnahme als laufend (Captures noch offen).
        assert!(core.is_recording());
        let finalizing = ticket.finish();
        assert!(!core.is_recording());
        // Der Abschluss laeuft noch: erst danach ist die Besprechung frei.
        assert_eq!(
            core.delete_step("m1", soon()).err(),
            Some(RECORDING_BUSY.to_string())
        );
        drop(finalizing);
        assert!(matches!(
            core.delete_step("m1", soon()),
            Ok(DeleteStep::Free)
        ));
    }

    /// Eine Aufnahme, deren Start noch aufraeumt, wird vom Loeschen abgewartet (nicht
    /// uebernommen), dann beendet.
    #[test]
    fn a_delete_waits_for_the_start_tail_and_then_takes_the_session() {
        let core = Arc::new(Core::new());
        let starting = core.begin_start(None, Some("a")).unwrap();
        core.publish_started(&starting, "a", "sitzung").unwrap();
        core.reserve_delete("a").unwrap();
        let waiter = Arc::clone(&core);
        let deleting = std::thread::spawn(move || {
            matches!(
                waiter.delete_step("a", Instant::now() + Duration::from_secs(10)),
                Ok(DeleteStep::Running(..))
            )
        });
        until_waiting(&core, 1);
        drop(starting);
        assert!(deleting.join().unwrap());
    }

    /// Der Abschluss einer Aufnahme haelt eine NEUE Aufnahme nicht auf, und das Loeschen der
    /// alten wartet trotzdem auf ihren Abschluss.
    #[test]
    fn a_new_recording_can_start_while_the_old_one_finalizes() {
        let core = Core::new();
        recording(&core, "old");
        let (_s, ticket) = running(&core);
        let finalizing = ticket.finish();
        let next = core.begin_start(None, Some("new")).unwrap();
        core.publish_started(&next, "new", "sitzung2").unwrap();
        drop(next);
        core.reserve_delete("old").unwrap();
        assert_eq!(
            core.delete_step("old", soon()).err(),
            Some(RECORDING_BUSY.to_string())
        );
        drop(finalizing);
        assert!(matches!(
            core.delete_step("old", soon()),
            Ok(DeleteStep::Free)
        ));
    }
}
