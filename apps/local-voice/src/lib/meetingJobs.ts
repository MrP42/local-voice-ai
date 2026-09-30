import type { JobProgress, MeetingEvent } from "@/bindings";

/**
 * P8a: Fortschritt der Verarbeitung einer Besprechung (Import, Enddurchlauf,
 * Neu-Transkription, Sprecher, KI-Notizen, Protokoll). Reine Funktionen ohne
 * React und Tauri, damit Rechnung und Anzeige getrennt bleiben.
 */

/** Stand vom Backend plus der Zeitpunkt, zu dem er hier ankam (fuer die Anzeige zwischen zwei Ereignissen). */
export type LiveProgress = JobProgress & { receivedAt: number };

export type ProgressMap = Record<string, LiveProgress>;

type ProgressEvent = Extract<MeetingEvent, { kind: "progress" }>;

const fromEvent = (event: ProgressEvent, now: number): LiveProgress => ({
  meeting_id: event.meeting_id,
  phase: event.phase,
  done: event.done,
  total: event.total,
  elapsed_ms: event.elapsed_ms,
  eta_ms: event.eta_ms,
  state: event.state,
  pausable: event.pausable,
  receivedAt: now,
});

/**
 * Wendet ein Besprechungs-Ereignis auf die Karte der laufenden Auftraege an:
 * `progress` setzt den Stand, `job_ended` und jeder Endzustand nehmen ihn weg
 * (auch wenn ein Ende verloren ginge, bleibt so kein Balken stehen).
 */
export const applyMeetingEvent = (
  map: ProgressMap,
  event: MeetingEvent,
  now: number,
): ProgressMap => {
  if (event.kind === "progress") {
    return { ...map, [event.meeting_id]: fromEvent(event, now) };
  }
  const ended =
    event.kind === "job_ended" ||
    (event.kind === "state" &&
      (event.status === "ready" ||
        event.status === "failed" ||
        event.status === "cancelled"));
  if (ended && event.meeting_id in map) {
    // Ein Endzustand des Imports beendet auch dessen Phase, ein `state ready`
    // aber nie einen Auftrag, der erst danach begann (Notizen): der Auftrag
    // dort meldet sich mit einem neuen `progress`.
    const { [event.meeting_id]: _removed, ...rest } = map;
    return rest;
  }
  return map;
};

/**
 * Ergaenzt die Karte um die Abfrage beim Oeffnen. Was schon per Ereignis da
 * ist, ist neuer als die Abfrage und bleibt; `skip` sind Auftraege, die
 * seit der Abfrage geendet haben.
 */
export const hydrateProgress = (
  map: ProgressMap,
  list: JobProgress[],
  now: number,
  skip: ReadonlySet<string> = new Set(),
): ProgressMap => {
  const next = { ...map };
  for (const item of list) {
    if (item.meeting_id in next || skip.has(item.meeting_id)) continue;
    next[item.meeting_id] = { ...item, receivedAt: now };
  }
  return next;
};

/** Anteil in ganzen Prozent, oder `null`, wenn die Groesse unbekannt ist. */
export const percentOf = (p: Pick<JobProgress, "done" | "total">) =>
  p.total > 0 ? Math.min(100, Math.floor((p.done * 100) / p.total)) : null;

/**
 * Balken ohne Prozentwert: Groesse unbekannt, Vorbereitung, oder die
 * Sprechertrennung, die je Kanal ein einziger Modelllauf ohne Zwischenstand ist.
 */
export const isIndeterminate = (p: JobProgress) =>
  p.total === 0 ||
  p.phase === "prepare" ||
  (p.phase === "speakers" && p.done === 0);

/** Zaehlt `done`/`total` Millisekunden Audio (sonst Schritte oder Bloecke)? */
export const countsAudio = (p: Pick<JobProgress, "phase">) =>
  p.phase === "transcription" ||
  p.phase === "final_pass" ||
  p.phase === "speakers";

/** Steht die Zeit? (Pause haelt Laufzeit und Restdauer an.) */
const clockStopped = (p: JobProgress) => p.state === "paused";

/** Laufzeit ohne Pausen, zwischen zwei Ereignissen weitergezaehlt. */
export const liveElapsedMs = (p: LiveProgress, now: number) =>
  p.elapsed_ms + (clockStopped(p) ? 0 : Math.max(0, now - p.receivedAt));

/** Restdauer, zwischen zwei Ereignissen heruntergezaehlt (nie unter 0). */
export const liveEtaMs = (p: LiveProgress, now: number): number | null => {
  if (p.eta_ms === null) return null;
  if (clockStopped(p)) return p.eta_ms;
  return Math.max(0, p.eta_ms - Math.max(0, now - p.receivedAt));
};

/**
 * `m:ss` wie ueberall in den Besprechungen (Dauer, Zeitstempel): die Minuten
 * zaehlen ueber 59 hinaus, 70 Minuten Audio sind `70:00`.
 */
export const formatClock = (ms: number) => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

/** Wie eine Restdauer angezeigt wird: Einheit und Werte (die Texte kommen aus i18n). */
export type EtaText =
  | { unit: "few" }
  | { unit: "seconds"; seconds: number }
  | { unit: "minutes"; minutes: number }
  | { unit: "hours"; hours: number; minutes: number };

/**
 * Grobe Restdauer statt Sekundengenauigkeit: unter 10 s "wenige Sekunden",
 * unter einer Minute auf 10 s, unter einer Stunde auf ganze Minuten
 * (aufgerundet), danach Stunden und Minuten. Eine Schaetzung, die auf die
 * Sekunde genau aussaehe, waere falsche Genauigkeit.
 */
export const etaText = (ms: number): EtaText => {
  const seconds = Math.ceil(ms / 1000);
  if (seconds <= 10) return { unit: "few" };
  if (seconds < 60) return { unit: "seconds", seconds: Math.ceil(seconds / 10) * 10 };
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return { unit: "minutes", minutes };
  return { unit: "hours", hours: Math.floor(minutes / 60), minutes: minutes % 60 };
};

/** Fehlercodes der Steuerbefehle als i18n-Schluessel. */
export const JOB_ERROR_KEYS: Record<string, string> = {
  no_job: "meetings.progress.errors.noJob",
  not_pausable: "meetings.progress.errors.notPausable",
  job_stopping: "meetings.progress.errors.stopping",
  job_busy: "meetings.progress.errors.busy",
  meeting_busy: "meetings.progress.errors.busy",
  not_cancelled: "meetings.progress.errors.notCancelled",
  audio_missing: "meetings.errors.audioMissing",
};
