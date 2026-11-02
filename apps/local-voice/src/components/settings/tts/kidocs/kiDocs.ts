import { save } from "@tauri-apps/plugin-dialog";
import type { TFunction } from "i18next";
import { commands, type BookTemplate, type KiDocument } from "@/bindings";
import {
  docProgressKey,
  loadProgress,
  progressFraction,
} from "@/lib/library/progress";
import { exportFileName } from "@/lib/utils/exportName";

/** Gruppen der Vorlagen (wie im Rust-Kern) plus übernommene Dokumente. */
export const KI_CATEGORIES = [
  "ueberblick",
  "lernen",
  "analyse",
  "anwenden",
] as const;
export type KiCategory = (typeof KI_CATEGORIES)[number] | "import";
export type KiFilter = "all" | KiCategory;

export function categoryOf(
  doc: KiDocument,
  templates: BookTemplate[],
): KiCategory {
  const tpl = templates.find((x) => x.id === doc.template_id);
  const cat = tpl?.category as KiCategory | undefined;
  if (cat && (KI_CATEGORIES as readonly string[]).includes(cat)) return cat;
  return "import";
}

/** Hörfortschritt eines Dokuments (0–1) aus dem Bibliotheksfortschritt. */
export const listenedFraction = (pageId: string, docId: string): number =>
  progressFraction(loadProgress(docProgressKey(pageId, docId)));

export type KiExportFormat = "md" | "docx" | "pdf";

/** Speichern-Dialog und Export; `false` bei Abbruch durch den Nutzer. */
export async function exportKiDocument(
  pageId: string,
  doc: KiDocument,
  format: KiExportFormat,
  t: TFunction,
): Promise<boolean> {
  const name = t(`tts.kiDocs.export.${format}`);
  const target = await save({
    filters: [{ name, extensions: [format] }],
    defaultPath: exportFileName(doc.title, "KI-Dokument", format),
  });
  if (typeof target !== "string") return false;
  const result = await commands.bookAiDocumentExport(pageId, doc.id, target);
  if (result.status === "error") throw new Error(result.error);
  return true;
}

/** Fehlercodes des Kerns in lesbaren Text. */
export function kiErrorText(code: string, t: TFunction): string {
  const known: Record<string, string> = {
    "book_ai:too_long": "tts.kiDocs.errors.tooLong",
    "book_ai:busy": "tts.kiDocs.errors.busy",
    "book_ai:empty_source": "tts.kiDocs.errors.empty",
    "book_ai:cancelled": "tts.kiDocs.errors.cancelled",
  };
  const key = known[code];
  return key ? t(key) : t("tts.kiDocs.errors.generic", { code });
}
