import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import i18n from "@/i18n";
import { commands, type TtsPhase, type TtsStatus } from "@/bindings";

/**
 * Die Vorlese-Sitzung der Aufnahmen.
 *
 * Keine eigene Sprachausgabe: dieselbe Sitzung des Backends wie auf der Vorlesen-Seite
 * (`tts_speak_text` legt sie an, `tts_speak_resume`, `tts_speak_seek`, `tts_cancel`,
 * `tts_voice_changed` und `tts_speak_to_file` bedienen sie). Sie ist EINE fuer die ganze
 * App; wer sie gestartet hat, steht in `key`. Hier steht nur, was die Oberflaeche dazu
 * wissen muss: ob gerade gespielt wird, wo der Satzzaehler steht, ob pausiert ist.
 */
export interface ReadAloudSession {
  /** Wer vorliest, z. B. `minutes:<Besprechung>:<Dokument>`; je Inhalt verschieden. */
  key: string;
  /** Unterscheidet zwei Laeufe desselben Knopfes. */
  token: number;
  /** Der vorgelesene Klartext (fuer "Audio speichern"). */
  text: string;
  /** Laufende Backend-Aufrufe (Start, Fortsetzen, Satzsprung): >0 = es wird gesprochen. */
  runs: number;
  /** Pause angefordert (sofort sichtbar, auch bevor der Aufruf zurueckkehrt). */
  paused: boolean;
  /**
   * Stimmwechsel waehrend der Wiedergabe: das Backend (`tts_voice_changed`) liest im
   * Hintergrund mit der neuen Stimme weiter, der urspruengliche Aufruf ist schon zurueck.
   * Solange das Backend dann in der Phase `speaking` ist, gilt die Sitzung als laufend.
   */
  handover: boolean;
  /** Saetze, die vollstaendig gesprochen sind, und die Gesamtzahl. */
  progress: { position: number; total: number } | null;
  /** Der Satz, der gerade zu hoeren ist (nullbasiert) und sein Text. */
  sentence: { index: number; text: string } | null;
}

/**
 * Die gemerkte Stelle einer Wiedergabe, die nicht mehr die globale Sitzung ist (eine
 * andere Wiedergabe hat sie ersetzt): je Knopf-Schluessel (Aufnahme + Dokument) der Text
 * und der Satz, ab dem "Weiter" neu ansetzt. Lebt, solange die App laeuft.
 */
export interface SavedReading {
  key: string;
  text: string;
  /** Nullbasierter Satz, mit dem die Wiedergabe fortgesetzt wird (= schon gehoerte Saetze). */
  position: number;
  /** Anzahl der Saetze, 0 = unbekannt. */
  total: number;
}

export interface ReadAloudExport {
  key: string;
  position: number;
  total: number;
  /** Der Pfad, den der Lauf schreibt: erst das Ziel des Dialogs, dann der Pfad, den der
   *  Befehl meldet (bei MP3 mit korrigierter Endung). */
  path: string;
}

let session: ReadAloudSession | null = null;
const saved = new Map<string, SavedReading>();
let exporting: ReadAloudExport | null = null;
let tokenCounter = 0;
const subscribers = new Set<() => void>();

const notify = () => subscribers.forEach((fn) => fn());
export const subscribeReadAloud = (fn: () => void) => {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
};
export const getReadAloudSession = () => session;
export const getReadAloudExport = () => exporting;
/** Die gemerkte Stelle fuer `key`; dasselbe Objekt, solange sie sich nicht aendert. */
export const getSavedReading = (key: string) => saved.get(key) ?? null;

const forget = (key: string) => {
  if (saved.delete(key)) notify();
};
/**
 * Die Stelle einer Sitzung merken, die gleich ersetzt wird. Zu Ende gelesen oder
 * noch kein Satz gehoert: nichts fortzusetzen.
 */
const remember = (old: ReadAloudSession) => {
  const total = old.progress?.total ?? 0;
  const position = old.progress?.position ?? 0;
  // Zu Ende gelesen oder noch nichts gehoert: nichts fortzusetzen, ein Neustart genuegt.
  if (position <= 0 || (total > 0 && position >= total)) return;
  saved.set(old.key, { key: old.key, text: old.text, position, total });
};

const setSession = (next: ReadAloudSession | null) => {
  session = next;
  notify();
};
const patchSession = (token: number, patch: Partial<ReadAloudSession>) => {
  if (session?.token !== token) return;
  session = { ...session, ...patch };
  notify();
};
const setExport = (next: ReadAloudExport | null) => {
  exporting = next;
  notify();
};

/** Zu Ende gelesen? Dann ist die Sitzung vorbei, sonst bleibt sie fuer "Weiter". */
const settle = (token: number) => {
  if (session?.token !== token) return;
  const progress = session.progress;
  const interrupted =
    session.paused || (progress !== null && progress.position < progress.total);
  if (interrupted) patchSession(token, { runs: 0, handover: false });
  else setSession(null);
};

/** Letzte Phase des Backends; der Stimmwechsel-Lauf endet, wenn sie `speaking` verlaesst. */
let lastPhase: TtsPhase | null = null;
/**
 * Zwischen altem und neuem Lauf meldet das Backend kurz eine andere Phase; erst wenn sie
 * so lange anhaelt, gilt der Stimmwechsel-Lauf als beendet.
 */
const HANDOVER_GRACE_MS = 600;

let listening = false;
/** Fortschritt, Satz und Export kommen als Ereignisse; einmal anmelden, dauerhaft. */
const ensureListeners = () => {
  if (listening) return;
  listening = true;
  void listen<TtsStatus>("tts-state-changed", (e) => {
    lastPhase = e.payload.phase;
    const current = session;
    if (!current?.handover || current.runs > 0) return;
    if (lastPhase === "speaking" || lastPhase === "starting") return;
    const token = current.token;
    window.setTimeout(() => {
      if (lastPhase === "speaking" || lastPhase === "starting") return;
      if (session?.token === token && session.handover && session.runs === 0) {
        settle(token);
      }
    }, HANDOVER_GRACE_MS);
  });
  void listen<{ position: number; total: number }>(
    "tts-speak-progress",
    (e) => {
      if (session) {
        session = { ...session, progress: e.payload };
        notify();
      }
    },
  );
  void listen<{ context: string; index: number; text: string }>(
    "tts-current-sentence",
    (e) => {
      if (session && e.payload.context === "speak") {
        session = {
          ...session,
          sentence: { index: e.payload.index, text: e.payload.text },
        };
        notify();
      }
    },
  );
  void listen<{ position: number; total: number; cancelled: boolean }>(
    "tts-export-progress",
    (e) => {
      if (!exporting) return;
      const { position, total, cancelled } = e.payload;
      if (cancelled || (total > 0 && position >= total)) {
        const done = exporting;
        setExport(null);
        // Fertige Datei: die Dateileiste einer Vorlesen-Seite soll sie sofort zeigen.
        window.dispatchEvent(new CustomEvent("lv-files-changed"));
        // Der Pfad kommt vom Befehl, nicht aus dem Dialog: bei MP3 hat der Lauf die
        // Endung korrigiert, und die Meldung soll die Datei nennen, die es gibt.
        if (!cancelled) {
          toast.success(i18n.t("tts.exportSaved", { path: done.path }));
        }
        return;
      }
      setExport({ ...exporting, position, total });
    },
  );
  void listen<{ message: string }>("tts-export-error", (e) => {
    if (!exporting) return;
    setExport(null);
    toast.error(
      i18n.t("meetings.readAloud.saveError", { error: e.payload.message }),
    );
  });
};

type CommandResult = { status: "ok" } | { status: "error"; error: string };

/**
 * Einen Befehl der Sitzung ausfuehren (Start, Fortsetzen, Satzsprung). Der Befehl kehrt
 * erst zurueck, wenn die Wiedergabe endet -- aus welchem Grund, entscheidet sich danach:
 * Pause oder von aussen angehalten (Satzzaehler nicht am Ende) = Sitzung bleibt, damit
 * Weiter geht; sonst gelesen bis zum Ende = Sitzung vorbei.
 */
const run = async (token: number, command: () => Promise<CommandResult>) => {
  if (session?.token !== token) return;
  patchSession(token, { runs: session.runs + 1, paused: false, handover: false });
  let failure: string | null = null;
  try {
    const result = await command();
    if (result.status === "error") failure = result.error;
  } catch (e) {
    failure = String(e);
  }
  // Beendet oder ersetzt, waehrend der Befehl lief: nichts mehr zu melden.
  if (session?.token !== token) return;
  if (failure !== null) {
    toast.error(i18n.t("meetings.readAloud.error", { error: failure }));
    setSession(null);
    return;
  }
  const runs = session.runs - 1;
  if (runs > 0) {
    patchSession(token, { runs });
    return;
  }
  // Stimmwechsel: das Backend liest weiter; das Ende meldet dann `tts-state-changed`.
  if (session.handover && !session.paused) {
    patchSession(token, { runs: 0 });
    return;
  }
  settle(token);
};

/**
 * Vor einem Stimmwechsel aufrufen: laeuft gerade eine Wiedergabe, liest das Backend nach
 * dem Wechsel im Hintergrund weiter -- die Leiste soll dabei "Pause" zeigen, nicht "Weiter".
 */
export const markVoiceHandover = () => {
  if (!session || session.paused || session.runs === 0) return;
  patchSession(session.token, { handover: true });
};

/**
 * Text vorlesen. Eine Sitzung eines anderen Knopfes wird dabei ersetzt, ihre Stelle bleibt
 * gemerkt (`getSavedReading`). `from`: nicht von vorn, sondern ab diesem Satz (gemerkte
 * Stelle fortsetzen; `total` = Satzzahl der gemerkten Stelle, damit der Zaehler gleich stimmt).
 */
export const startReading = (
  key: string,
  text: string,
  from?: { position: number; total: number },
) => {
  ensureListeners();
  if (session && session.key !== key) remember(session);
  saved.delete(key);
  const token = ++tokenCounter;
  const position = from?.position ?? 0;
  setSession({
    key,
    token,
    text,
    runs: 0,
    paused: false,
    handover: false,
    progress: from ? { position, total: from.total } : null,
    sentence: null,
  });
  void run(token, () =>
    position > 0
      ? commands.ttsSpeakTextAt(text, position)
      : commands.ttsSpeakText(text),
  );
};

/** Eine gemerkte Stelle ab ihrem Satz neu starten. */
export const resumeSavedReading = (key: string) => {
  const entry = saved.get(key);
  if (!entry) return;
  startReading(key, entry.text, {
    position: entry.position,
    total: entry.total,
  });
};

/** Die gemerkte Stelle verwerfen (Stopp an einer Leiste ohne laufende Sitzung). */
export const discardSavedReading = (key: string) => forget(key);

/** Pause: anhalten, Position behalten -- Weiter setzt genau dort fort. */
export const pauseReading = () => {
  if (!session) return;
  patchSession(session.token, { paused: true, handover: false });
  void commands.ttsCancel();
};

export const resumeReading = () => {
  if (!session) return;
  void run(session.token, () => commands.ttsSpeakResume());
};

/** Um `delta` Saetze springen und von dort weiterlesen (auch aus der Pause). */
export const seekReading = (delta: number) => {
  if (!session) return;
  void run(session.token, () => commands.ttsSpeakSeek(delta));
};

/** Beenden: anhalten und die Sitzung samt gemerkter Stelle verwerfen. */
export const stopReading = () => {
  if (!session) return;
  saved.delete(session.key);
  setSession(null);
  void commands.ttsCancel();
};

/**
 * Wechsel der Aufnahme oder des Reiters: die Wiedergabe des Knopfes `key` PAUSIEREN statt
 * verwerfen. Es laeuft kein Ton weiter; kommt man zurueck, ist die Sitzung noch da und
 * "Weiter" setzt am selben Satz fort.
 */
export const suspendReadingIfOwner = (key: string) => {
  if (session?.key !== key || session.paused) return;
  patchSession(session.token, { paused: true, handover: false });
  void commands.ttsCancel();
};

/**
 * Den Text in eine Audiodatei schreiben (`tts_speak_to_file`, wie "Audio speichern" auf
 * der Vorlesen-Seite): kehrt sofort zurueck, der Fortschritt kommt als Ereignis.
 */
export const startExport = async (
  key: string,
  text: string,
  target: string,
) => {
  ensureListeners();
  setExport({ key, position: 0, total: 0, path: target });
  try {
    const result = await commands.ttsSpeakToFile(text, target);
    if (result.status === "error") {
      setExport(null);
      toast.error(
        i18n.t("meetings.readAloud.saveError", { error: result.error }),
      );
      return;
    }
    if (exporting?.key === key) setExport({ ...exporting, path: result.data });
  } catch (e) {
    setExport(null);
    toast.error(i18n.t("meetings.readAloud.saveError", { error: String(e) }));
  }
};

export const cancelExport = () => {
  void commands.ttsExportCancel();
};
