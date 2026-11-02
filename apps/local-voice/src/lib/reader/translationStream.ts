/**
 * Hilfen für die streamende Buchübersetzung: Abschnitte, die das Backend
 * einzeln meldet (`tts-translate-chunk`), werden in Index-Reihenfolge zum
 * Gesamttext zusammengesetzt; dazu die Fortschrittsangabe.
 */

export interface TranslateChunk {
  index: number;
  total: number;
  chapter: number;
  chapters: number;
  chapterTitle: string | null;
  text: string;
  cached: boolean;
}

export interface TranslateStreamState {
  /** Fertige Abschnitte nach Index; Lücken (noch nicht da) sind undefined. */
  parts: (string | undefined)[];
  /** Anzahl der lückenlos vorhandenen Abschnitte ab Index 0. */
  contiguous: number;
}

export const emptyStream = (): TranslateStreamState => ({
  parts: [],
  contiguous: 0,
});

/** Abschnitt einfügen (unveränderlich). Doppelte Meldungen überschreiben sich. */
export function addChunk(
  state: TranslateStreamState,
  chunk: Pick<TranslateChunk, "index" | "text">,
): TranslateStreamState {
  if (!Number.isInteger(chunk.index) || chunk.index < 0) return state;
  const parts = state.parts.slice();
  parts[chunk.index] = chunk.text;
  let contiguous = state.contiguous;
  while (contiguous < parts.length && parts[contiguous] !== undefined) {
    contiguous += 1;
  }
  return { parts, contiguous };
}

/** Der bisher lückenlose Text: alle Abschnitte in Reihenfolge, mit "\n" verbunden. */
export const streamText = (state: TranslateStreamState): string =>
  state.parts.slice(0, state.contiguous).join("\n");

export interface TransformProgress {
  kind: "translation" | "summary";
  done: number;
  total: number;
  chapter?: number | null;
  chapters?: number | null;
  chapterTitle?: string | null;
  doneChars?: number | null;
  totalChars?: number | null;
}

/** Anteil 0..1: bevorzugt nach Zeichen, sonst nach Abschnitten. */
export function progressRatio(p: TransformProgress): number {
  const byChars =
    p.totalChars && p.totalChars > 0 && p.doneChars != null
      ? p.doneChars / p.totalChars
      : null;
  const raw = byChars ?? (p.total > 0 ? p.done / p.total : 0);
  return Math.min(1, Math.max(0, Number.isFinite(raw) ? raw : 0));
}

export const progressPercent = (p: TransformProgress): number =>
  Math.round(progressRatio(p) * 100);

/** Kapiteltitel auf eine anzeigbare Länge kürzen. */
export function shortTitle(title: string | null | undefined, max = 40): string {
  const clean = (title ?? "").replace(/\s+/g, " ").trim();
  if (clean.length <= max) return clean;
  return `${clean.slice(0, Math.max(1, max - 1)).trimEnd()}…`;
}

export interface ProgressParts {
  /** 1-basiert; null, wenn das Backend keine Kapitel meldet. */
  chapter: number | null;
  chapters: number | null;
  /** 1-basiert, auf total begrenzt. */
  section: number;
  total: number;
  percent: number;
  title: string;
}

/** Zahlen für „Kapitel 3 von 18 · Abschnitt 12 von 41 · 29 %“. `chapter` im Event ist 0-basiert. */
export function progressParts(p: TransformProgress): ProgressParts {
  const hasChapters = (p.chapters ?? 0) > 0 && p.chapter != null;
  return {
    chapter: hasChapters
      ? Math.min((p.chapter as number) + 1, p.chapters as number)
      : null,
    chapters: hasChapters ? (p.chapters as number) : null,
    section: Math.min(p.done + 1, Math.max(p.total, 1)),
    total: p.total,
    percent: progressPercent(p),
    title: shortTitle(p.chapterTitle),
  };
}
