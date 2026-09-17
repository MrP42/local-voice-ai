import { save } from "@tauri-apps/plugin-dialog";
import { commands } from "@/bindings";

/** Dateiendung der Stimmen-Archive (Referenz, Transkript, Metadaten, Stile). */
export const ARCHIVE_EXT = "lvvoice";

export type ExportOutcome =
  | { status: "done"; path: string }
  | { status: "cancelled" }
  | { status: "error"; message: string };

const asMessage = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

/**
 * Stimme als Archiv sichern: Speichern-Dialog, dann Backend-Export. Wird
 * von der Stimmenzeile (Symbol) und vom Bearbeiten-Panel gleich benutzt,
 * damit beide Wege dieselbe Datei erzeugen.
 */
export const exportVoiceArchive = async (
  id: string,
  filterLabel: string,
): Promise<ExportOutcome> => {
  let target: string | null = null;
  try {
    target = await save({
      defaultPath: `${id}.${ARCHIVE_EXT}`,
      filters: [{ name: filterLabel, extensions: [ARCHIVE_EXT] }],
    });
  } catch (e) {
    return { status: "error", message: asMessage(e) };
  }
  if (typeof target !== "string") return { status: "cancelled" };
  try {
    const res = await commands.ttsExportVoice(id, target);
    if (res.status === "error") return { status: "error", message: res.error };
    return { status: "done", path: target };
  } catch (e) {
    return { status: "error", message: asMessage(e) };
  }
};
