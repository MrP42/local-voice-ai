import { BaseDirectory, readTextFile } from "@tauri-apps/plugin-fs";

/**
 * Diktatmodell, das eine Erstinstallation ohne weitere Frage einrichtet
 * („Nemotron 3.5 ASR Streaming“, 28 Sprachen inkl. Deutsch, OpenMDW-1.1 —
 * kommerziell nutzbar). Muss mit dem Installer-Vertrag übereinstimmen.
 */
export const DEFAULT_DICTATION_MODEL =
  "handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf";

/** Datei, die der Windows-Installer bei einer Neuinstallation schreibt. */
export const FIRST_RUN_FILE = "erststart.ini";

/**
 * Liest `[diktat] modell=…` aus dem INI-Text. `undefined` heißt: kein Eintrag
 * (Vorgabe gilt), `null` heißt: ausdrücklich abgewählt (`modell=aus`).
 */
export const parseFirstRunIni = (text: string): string | null | undefined => {
  let section = "";
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/^\uFEFF/, "").trim();
    if (!line || line.startsWith(";") || line.startsWith("#")) continue;
    const header = line.match(/^\[(.+)\]$/);
    if (header) {
      section = header[1].trim().toLowerCase();
      continue;
    }
    if (section !== "diktat") continue;
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    if (line.slice(0, eq).trim().toLowerCase() !== "modell") continue;
    const value = line.slice(eq + 1).trim();
    if (!value) return undefined;
    return value.toLowerCase() === "aus" ? null : value;
  }
  return undefined;
};

/**
 * Welches Diktatmodell der erste Start automatisch einrichtet, oder `null`,
 * wenn die Auswahl dem Nutzer überlassen bleibt. Ohne Installer-Datei
 * (macOS, Linux, ältere Installer) gilt die Vorgabe.
 */
export const readFirstRunDictationModel = async (): Promise<string | null> => {
  try {
    const text = await readTextFile(FIRST_RUN_FILE, {
      baseDir: BaseDirectory.Resource,
    });
    const value = parseFirstRunIni(text);
    return value === undefined ? DEFAULT_DICTATION_MODEL : value;
  } catch (e) {
    // Nur „Datei fehlt“ heißt Vorgabe; eine unlesbare Datei darf ein „aus“ nicht überstimmen.
    return isNotFound(e) ? DEFAULT_DICTATION_MODEL : null;
  }
};

const isNotFound = (e: unknown) =>
  /os error 2\b|not found|no such file|cannot find/i.test(String(e));
