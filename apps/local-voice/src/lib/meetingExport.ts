import type { ExportParts } from "@/bindings";

/** Formate in der Reihenfolge der Knoepfe; die Endung waehlt den Renderer. */
export const EXPORT_FORMATS = [
  "docx",
  "txt",
  "md",
  "html",
  "pdf",
  "srt",
  "vtt",
  "json",
] as const;
export type ExportFormat = (typeof EXPORT_FORMATS)[number];

export const EXPORT_PART_KEYS = [
  "notes",
  "ai_notes",
  "minutes",
  "transcript",
  "participants",
] as const satisfies readonly (keyof ExportParts)[];

/** Dateiname ohne die Zeichen, die Windows im Namen verbietet. */
export const exportFileName = (title: string, format: ExportFormat) =>
  `${title.replace(/[\/:*?"<>|]/g, "_").trim() || "besprechung"}.${format}`;

/** `pdf_unavailable`, `pdf_timeout`, `pdf_low_memory`, `pdf_failed`. */
export const isPdfFailure = (error: string) => error.startsWith("pdf_");
