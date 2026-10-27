import type {
  CloneSource,
  OwnVoice,
  PhononVoiceError,
  PhononVoiceErrorKind,
} from "@/bindings";

/** Längste Namenslänge, die das Backend annimmt. */
export const NAME_MAX = 40;
/** Sprachen der Aufnahme, in der Reihenfolge der Anzeige (Gradium: en, fr, de, es, pt). */
export const CLONE_LANGUAGES = ["de", "en", "fr", "es", "pt"] as const;
/** Länger läuft eine Mikrofonaufnahme für die Stimme nicht (dann stoppt sie von selbst). */
export const MAX_RECORDING_S = 30;
/** Stufen des laufenden Auftrags (Ereignis `phonon-voice-clone-phase`). */
export const PHASE_EVENT = "phonon-voice-clone-phase";
export type ClonePhase = "uploading" | "waiting" | "downloading";

export type SourceKind = "file" | "recording" | "fish";

/** Dateiendungen im Auswahldialog der Audiodatei (wie im Backend erlaubt). */
export const AUDIO_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "aac",
  "flac",
  "ogg",
  "opus",
  "wma",
];

/** Fehler aus einem Befehl, auch wenn er ausnahmsweise wirft. */
export const asPhononError = (e: unknown): PhononVoiceError => {
  if (e && typeof e === "object" && "kind" in e && "message" in e)
    return e as PhononVoiceError;
  return {
    kind: "io" satisfies PhononVoiceErrorKind,
    message: e instanceof Error ? e.message : String(e),
  };
};

/** Dateiname ohne Ordner (für die Anzeige: der Pfad selbst bleibt unsichtbar). */
export const fileBaseName = (path: string): string =>
  path.split(/[\\/]/).pop() ?? path;

/** Vorschlag für den Namen einer Stimme aus einem Dateinamen. */
export const nameFromFile = (path: string): string => {
  const base = fileBaseName(path).replace(/\.[^.]*$/, "");
  return base
    .replace(/[-_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, NAME_MAX);
};

/**
 * Startsekunde aus dem Eingabefeld: leer = 0, sonst eine Zahl von 0 aufwärts
 * (Komma oder Punkt). `null` = unbrauchbar.
 */
export const parseStart = (raw: string): number | null => {
  const text = raw.trim().replace(",", ".");
  if (text === "") return 0;
  if (!/^\d+(\.\d+)?$/.test(text)) return null;
  const n = Number(text);
  return Number.isFinite(n) && n <= 36000 ? n : null;
};

export interface CloneInput {
  kind: SourceKind;
  path: string;
  fishId: string;
  /** Mikrofonaufnahme liegt bereit. */
  recorded: boolean;
}

/** Die Quelle für den Befehl; `null`, solange sie nicht vollständig gewählt ist. */
export const buildSource = (input: CloneInput): CloneSource | null => {
  switch (input.kind) {
    case "file":
      return input.path ? { kind: "file", path: input.path } : null;
    case "fish":
      return input.fishId ? { kind: "fish", voice_id: input.fishId } : null;
    case "recording":
      return input.recorded ? { kind: "recording" } : null;
  }
};

export interface CloneReadiness extends CloneInput {
  name: string;
  start: string;
  consent: boolean;
  keyPresent: boolean;
  busy: boolean;
}

/**
 * Darf „Stimme erstellen“ angeboten werden? Ohne Einwilligung nie: das Backend
 * prüft sie noch einmal, hier sperrt die Oberfläche schon den Knopf.
 */
export const canRunClone = (s: CloneReadiness): boolean =>
  s.consent &&
  s.keyPresent &&
  !s.busy &&
  s.name.trim() !== "" &&
  s.name.trim().length <= NAME_MAX &&
  buildSource(s) !== null &&
  parseStart(s.start) !== null;

/** Herkunft einer eigenen Stimme für die Anzeige. */
export const isCloned = (voice: OwnVoice): boolean =>
  voice.origin === "gradium-clone";

/** Namen der Modelle für die Anzeige („phonon-40m-de“ → „40m-de“). */
export const modelLabel = (id: string): string => id.replace(/^phonon-/, "");
