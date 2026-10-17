import { commands, type Theme as ThemeSetting } from "@/bindings";
import {
  AKZENT_STANDARD,
  type AppDesign,
  EINGEBAUTE_DESIGNS,
  type Erscheinungsbild,
  type ErscheinungsbildStand,
  type Modus,
  normalisiereErscheinungsbild,
  speichereStand,
  standardStand,
} from "@/erscheinungsbild";
import { THEME_STORAGE_KEY } from "@/lib/utils/theme";

/**
 * Anbindung des SDK-Bausteins „Erscheinungsbild“ an Local Voice.
 *
 * - AppSettings.appearance ist führend. FluentRoot lädt es nach dem Start
 *   und übernimmt es direkt über die Setter des Hooks (`uebernehmen`);
 *   localStorage `local-voice.erscheinungsbild.v1` ist nur der Startwert
 *   gegen Aufblitzen.
 * - Persistenz erst nach diesem Abgleich; ein Stand gilt erst als
 *   gespeichert, wenn der Befehl erfolgreich war (sonst Wiederholung beim
 *   nächsten Wechsel).
 * - Migration: ohne gespeichertes Erscheinungsbild wird das alte Farbschema
 *   übernommen (AppSettings.theme, vor dem Abgleich `handy.theme`).
 * - AppSettings.theme bleibt Spiegel des Farbschemas: Fensterrahmen unter
 *   Windows und das Meeting-Hinweisfenster lesen es weiter.
 */
export const APPEARANCE_STORAGE_KEY = "local-voice.erscheinungsbild.v1";

export const modusFromTheme = (theme: unknown): Modus =>
  theme === "light" ? "hell" : theme === "dark" ? "dunkel" : "system";

export const themeFromModus = (modus: Modus): ThemeSetting =>
  modus === "hell" ? "light" : modus === "dunkel" ? "dark" : "system";

const readLocal = (key: string): string | null => {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
};

const parse = (raw: string | null | undefined): unknown => {
  if (!raw) return undefined;
  try {
    return JSON.parse(raw);
  } catch {
    return undefined;
  }
};

const designs: readonly AppDesign[] = EINGEBAUTE_DESIGNS;

/**
 * Vor dem ersten Rendern: fehlt der neue Schlüssel, aus `handy.theme`
 * übernehmen, damit Hell/Dunkel nach dem Update gleich bleibt. Nur
 * Startwert; schlägt das Schreiben fehl, gilt nach dem Abgleich das Backend.
 */
export function migrateLocalAppearance(): void {
  if (readLocal(APPEARANCE_STORAGE_KEY) != null) return;
  const old = readLocal(THEME_STORAGE_KEY);
  if (old == null) return;
  speichereStand(APPEARANCE_STORAGE_KEY, {
    ...standardStand(),
    modus: modusFromTheme(old),
  });
}

/** Kanonische Form zum Vergleichen und Speichern (Akzente sortiert). */
export function serializeAppearance(s: ErscheinungsbildStand): string {
  const akzent = Object.fromEntries(
    Object.entries(s.akzent).sort(([a], [b]) => a.localeCompare(b)),
  );
  return JSON.stringify({
    modus: s.modus,
    kontrast: s.kontrast,
    animationen: s.animationen,
    designId: s.designId,
    akzent,
  });
}

export interface LoadedAppearance {
  stand: ErscheinungsbildStand;
  /** Kein Wert in AppSettings: Stand ist migriert und muss gespeichert werden. */
  migrated: boolean;
  /** Farbschema, wie es in AppSettings.theme steht. */
  theme: ThemeSetting;
}

/**
 * Lädt das Erscheinungsbild aus AppSettings; `null`, wenn die Einstellungen
 * nicht lesbar sind (dann bleibt der lokale Stand und nichts wird
 * geschrieben).
 */
export async function loadAppearance(
  local: ErscheinungsbildStand,
): Promise<LoadedAppearance | null> {
  try {
    const result = await commands.getAppSettings();
    if (result.status !== "ok") return null;
    const theme = result.data.theme ?? "system";
    const fromSettings = parse(result.data.appearance);
    if (fromSettings !== undefined) {
      return {
        stand: normalisiereErscheinungsbild(fromSettings, designs),
        migrated: false,
        theme,
      };
    }
    // Migration: altes Farbschema übernehmen, Rest aus dem lokalen Stand.
    return {
      stand: { ...local, modus: modusFromTheme(theme) },
      migrated: true,
      theme,
    };
  } catch (e) {
    console.warn("Failed to load appearance from settings:", e);
    return null;
  }
}

/**
 * Setzt einen Stand direkt in den Hook (ohne Umweg über localStorage und
 * storage-Ereignis). Die Setter arbeiten mit funktionalen Updates und
 * lassen sich daher nacheinander aufrufen.
 */
export function uebernehmen(z: Erscheinungsbild, stand: ErscheinungsbildStand) {
  z.setzeModus(stand.modus);
  z.setzeKontrast(stand.kontrast);
  z.setzeAnimationen(stand.animationen);
  z.setzeDesign(stand.designId);
  const ids = new Set([...Object.keys(z.akzent), ...Object.keys(stand.akzent)]);
  for (const id of ids) z.setzeAkzent(stand.akzent[id] ?? AKZENT_STANDARD, id);
}

/** Was in AppSettings nachweislich gespeichert ist. */
export interface Saved {
  appearance: string | null;
  theme: ThemeSetting | null;
}

/**
 * Schreibt die Abweichungen nach AppSettings. Gibt den neuen gespeicherten
 * Stand zurück; ein fehlgeschlagener Befehl lässt seinen Teil unverändert,
 * damit der nächste Wechsel es erneut versucht.
 */
export async function persistAppearance(
  stand: ErscheinungsbildStand,
  saved: Saved,
): Promise<{ saved: Saved; ok: boolean }> {
  const json = serializeAppearance(stand);
  const theme = themeFromModus(stand.modus);
  const next: Saved = { ...saved };
  let ok = true;
  const run = async (f: () => Promise<{ status: string }>) => {
    try {
      return (await f()).status === "ok";
    } catch (e) {
      console.warn("Failed to persist appearance:", e);
      return false;
    }
  };
  if (json !== saved.appearance) {
    if (await run(() => commands.changeAppearanceSetting(json))) next.appearance = json;
    else ok = false;
  }
  if (theme !== saved.theme) {
    if (await run(() => commands.changeThemeSetting(theme))) next.theme = theme;
    else ok = false;
  }
  return { saved: next, ok };
}
