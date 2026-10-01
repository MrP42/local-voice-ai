// Reine Logik des Projekt-Protokolls (G3, #70, U9): keine React-, keine Tauri-
// Importe, damit sie sich ohne Browser pruefen laesst (`node`, Type-Stripping).
//
// Die Typen kommen aus `bindings.ts` (nur als Typ importiert).

import type {
  Citation,
  EntrySource,
  ProjectCandidate,
  ProjectKind,
  ProjectMinutes,
  SourceRecording,
} from "@/bindings";

/**
 * Praefix des Auftragsschluessels, unter dem der Lauf eines Projekts im
 * Verzeichnis der Verarbeitungen steht (Fortschritt, Stopp). Muss zu
 * `project::run_key` im Backend passen (dort durch einen Test gesichert).
 */
export const PROJECT_MINUTES_JOB_PREFIX = "project-minutes:";

export const projectMinutesJobKey = (folderId: string): string =>
  `${PROJECT_MINUTES_JOB_PREFIX}${folderId}`;

/** Wie viele Aufnahmen mindestens zu einem GEMEINSAMEN Protokoll gehoeren. */
export const MIN_RECORDINGS = 2;

/** Codes der Fehler des Projekt-Protokolls (`minutes::ALL_CODES` + `project::EXTRA_CODES`). */
export const PROJECT_MINUTES_ERROR_CODES = [
  "minutes_busy",
  "minutes_cancelled",
  "folder_not_found",
  "no_selection",
  "too_many_recordings",
  "not_in_project",
  "kind_invalid",
  "meeting_not_found",
  "meeting_not_finished",
  "no_transcript",
  "template_not_found",
  "no_provider",
  "no_model",
  "memory_low",
  "llm_failed",
  "store_failed",
] as const;

export type ProjectMinutesErrorCode =
  (typeof PROJECT_MINUTES_ERROR_CODES)[number];

/** Der Code einer Fehlermeldung (`"<code>"` oder `"<code>: <detail>"`); Unbekanntes ist `llm_failed`. */
export const projectMinutesErrorCode = (
  error: string,
): ProjectMinutesErrorCode =>
  PROJECT_MINUTES_ERROR_CODES.find(
    (code) => error === code || error.startsWith(`${code}:`),
  ) ?? "llm_failed";

/** Der Text hinter dem Code, sonst leer. */
export const projectMinutesErrorDetail = (error: string): string => {
  const code = PROJECT_MINUTES_ERROR_CODES.find((c) =>
    error.startsWith(`${c}:`),
  );
  return code ? error.slice(code.length + 1).trim() : "";
};

/** IDs der waehlbaren Aufnahmen, in der Reihenfolge der Antwort (chronologisch). */
export const eligibleIds = (candidates: ProjectCandidate[]): string[] =>
  candidates.filter((c) => c.eligible).map((c) => c.meeting_id);

/** Die Auswahl, bereinigt um Aufnahmen, die nicht (mehr) waehlbar sind. */
export const pruneSelection = (
  selected: string[],
  candidates: ProjectCandidate[],
): string[] => {
  const ok = new Set(eligibleIds(candidates));
  return selected.filter((id) => ok.has(id));
};

/** `03:15` oder `1:03:15` aus Millisekunden. */
export const clock = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
};

/** Titel gekuerzt fuer eine Quellen-Marke. */
export const shortTitle = (title: string, max = 22): string => {
  const chars = Array.from(title.trim());
  return chars.length <= max
    ? chars.join("")
    : `${chars.slice(0, max - 1).join("")}…`;
};

/** Die Aufnahme zu einem Beleg (aus der Liste, die mit dem Dokument gespeichert wurde). */
export const recordingOf = (
  doc: Pick<ProjectMinutes, "recordings">,
  source: EntrySource,
): SourceRecording | undefined =>
  doc.recordings.find((r) => r.index === source.recording);

/**
 * Ein Beleg als Chat-Beleg: so oeffnet die vorhandene Sprungmechanik der
 * Einzelansicht (`openCitation`, `jumpToSource`) die Aufnahme und springt zur
 * Stelle im Transkript und im Ton.
 */
export const sourceToCitation = (
  source: EntrySource,
  title: string,
): Citation => ({
  n: source.recording,
  meeting_id: source.meeting_id,
  meeting_title: title,
  started_at: null,
  source: "transcript",
  epoch: 0,
  segment_index: source.segment_index,
  start_ms: source.start_ms,
  ref_key: null,
  quote: "",
});

/** Der Titel des Dokuments in der Liste und im Kopf. */
export const kindTitleKey = (kind: ProjectKind): string =>
  kind === "summary"
    ? "meetings.projectMinutes.list.titleSummary"
    : "meetings.projectMinutes.list.titleMinutes";

/** Lokale Meldung "Projekt-Protokolle haben sich geaendert" (Loeschen, Ende eines Laufs). */
const EVENT = "lva:project-minutes-changed";

export const notifyProjectMinutesChanged = () => {
  window.dispatchEvent(new CustomEvent(EVENT));
};

export const PROJECT_MINUTES_CHANGED_EVENT = EVENT;
