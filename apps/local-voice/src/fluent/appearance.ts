import { commands, type Theme as ThemeSetting } from "@/bindings";
import {
  type AppDesign,
  EINGEBAUTE_DESIGNS,
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
 * - localStorage `local-voice.erscheinungsbild.v1` gilt sofort beim Start
 *   (kein Aufblitzen), AppSettings.appearance ist die Quelle der Wahrheit.
 * - Migration: ohne gespeichertes Erscheinungsbild wird das alte Farbschema
 *   übernommen (AppSettings.theme, sonst localStorage `handy.theme`).
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
 * übernehmen, damit Hell/Dunkel nach dem Update gleich bleibt.
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

const serialize = (s: ErscheinungsbildStand) => JSON.stringify(s);

/** Zuletzt mit AppSettings abgeglichener Stand; `null` bis zum ersten Abgleich. */
let synced: string | null = null;

/**
 * Abgleich mit AppSettings beim Start. Liefert den gültigen Stand und
 * schreibt ihn nach localStorage; der Hook liest ihn über ein
 * storage-Ereignis nach.
 */
export async function syncAppearanceFromSettings(): Promise<void> {
  let stand: ErscheinungsbildStand | null = null;
  let persist = false;
  try {
    const result = await commands.getAppSettings();
    if (result.status === "ok") {
      const fromSettings = parse(result.data.appearance);
      if (fromSettings !== undefined) {
        stand = normalisiereErscheinungsbild(fromSettings, designs);
      } else {
        // Migration: altes Farbschema übernehmen, Rest aus localStorage.
        const local = parse(readLocal(APPEARANCE_STORAGE_KEY));
        stand = {
          ...normalisiereErscheinungsbild(local ?? {}, designs),
          modus: modusFromTheme(result.data.theme),
        };
        persist = true;
      }
    }
  } catch (e) {
    console.warn("Failed to sync appearance from settings:", e);
  }
  if (!stand) {
    stand = normalisiereErscheinungsbild(
      parse(readLocal(APPEARANCE_STORAGE_KEY)) ?? {},
      designs,
    );
  }
  const json = serialize(stand);
  if (readLocal(APPEARANCE_STORAGE_KEY) !== json) {
    speichereStand(APPEARANCE_STORAGE_KEY, stand);
    window.dispatchEvent(
      new StorageEvent("storage", { key: APPEARANCE_STORAGE_KEY }),
    );
  }
  synced = json;
  if (persist) await persistAppearance(stand, true);
}

/**
 * Nach einer Änderung in der Oberfläche: AppSettings fortschreiben. Vor dem
 * ersten Abgleich nichts schreiben, sonst überschriebe ein veralteter
 * localStorage-Stand die Einstellung.
 */
export async function persistAppearance(
  stand: ErscheinungsbildStand,
  force = false,
): Promise<void> {
  if (synced === null && !force) return;
  const json = serialize(stand);
  if (json === synced && !force) return;
  const previous = parse(synced) as ErscheinungsbildStand | undefined;
  synced = json;
  try {
    await commands.changeAppearanceSetting(json);
    if (force || previous?.modus !== stand.modus) {
      await commands.changeThemeSetting(themeFromModus(stand.modus));
    }
  } catch (e) {
    console.warn("Failed to persist appearance:", e);
  }
}
