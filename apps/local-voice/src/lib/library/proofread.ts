import type { SyncPoint } from "@/bindings";

/**
 * Korrekturlesen des Transkripts (lva#208 B), der reine Teil ohne Tauri:
 * Absatzgrenzen, Umrechnen von Zeichenstellen und Zeitachse nach der
 * Korrektur und der Vergleich alt/neu für „Änderungen anzeigen“.
 *
 * Das Modell darf keine Absätze zusammenlegen oder teilen (der Kern prüft das):
 * Absatz n des alten Textes ist Absatz n des neuen. Darauf beruht die
 * Umrechnung.
 */

/** Zeichenbereich eines Absatzes (`end` ausschließlich). */
export interface Span {
  start: number;
  end: number;
}

/** Absätze: durch eine Leerzeile getrennt, wie im Kern (`split("\n\n")`). */
export function paragraphSpans(text: string): Span[] {
  const spans: Span[] = [];
  let start = 0;
  for (;;) {
    const at = text.indexOf("\n\n", start);
    if (at < 0) {
      spans.push({ start, end: text.length });
      return spans;
    }
    spans.push({ start, end: at });
    start = at + 2;
  }
}

/**
 * Stelle im alten Text → dieselbe Stelle im korrigierten Text: gleicher Absatz,
 * im Absatz im gleichen Verhältnis. Absatzanfänge bleiben Absatzanfänge.
 * Stimmt die Absatzzahl nicht, wird im Verhältnis zur Textlänge gerechnet.
 */
export function remapOffset(
  oldText: string,
  newText: string,
  offset: number,
): number {
  if (offset <= 0) return 0;
  if (offset >= oldText.length) return newText.length;
  const a = paragraphSpans(oldText);
  const b = paragraphSpans(newText);
  if (a.length !== b.length)
    return Math.min(
      newText.length,
      Math.round((offset / oldText.length) * newText.length),
    );
  let i = a.length - 1;
  while (i > 0 && a[i].start > offset) i--;
  const from = a[i];
  const to = b[i];
  // In der Lücke zwischen zwei Absätzen: Abstand zum Absatzende bleibt.
  if (offset >= from.end) return to.end + (offset - from.end);
  const oldLen = from.end - from.start;
  const newLen = to.end - to.start;
  if (oldLen <= 0) return to.start;
  const rel = offset - from.start;
  return to.start + Math.min(newLen, Math.round((rel * newLen) / oldLen));
}

/** Zeitachse nach der Korrektur: je Stützpunkt die Stelle im neuen Text. */
export function remapSync(
  oldText: string,
  newText: string,
  sync: SyncPoint[],
): SyncPoint[] {
  const out: SyncPoint[] = [];
  let last = 0;
  for (const p of sync) {
    const chars = Math.max(last, remapOffset(oldText, newText, p.chars));
    out.push({ ...p, chars });
    last = chars;
  }
  return out;
}

/** Ein Stück des Vergleichs auf Wortebene. */
export interface DiffOp {
  kind: "same" | "del" | "add";
  text: string;
}

const tokens = (s: string) => s.split(/(\s+)/).filter((t) => t !== "");

/**
 * Wortvergleich (längste gemeinsame Teilfolge). Leerraum gehört zum
 * jeweiligen Wort. Aufeinanderfolgende gleiche Arten werden vereint.
 */
export function wordDiff(before: string, after: string): DiffOp[] {
  const a = tokens(before);
  const b = tokens(after);
  const n = a.length;
  const m = b.length;
  const table: Uint32Array[] = Array.from(
    { length: n + 1 },
    () => new Uint32Array(m + 1),
  );
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      table[i][j] =
        a[i] === b[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
  const ops: DiffOp[] = [];
  const push = (kind: DiffOp["kind"], text: string) => {
    const last = ops[ops.length - 1];
    if (last && last.kind === kind) last.text += text;
    else ops.push({ kind, text });
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      push("same", a[i]);
      i++;
      j++;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      push("del", a[i++]);
    } else {
      push("add", b[j++]);
    }
  }
  while (i < n) push("del", a[i++]);
  while (j < m) push("add", b[j++]);
  return ops;
}

/** Ein geänderter Absatz. */
export interface ParagraphChange {
  /** Index des Absatzes (gleich im alten und im neuen Text). */
  index: number;
  before: string;
  after: string;
  ops: DiffOp[];
}

/** Die Absätze, die sich unterscheiden (nur bei gleicher Absatzzahl sinnvoll). */
export function paragraphChanges(
  oldText: string,
  newText: string,
): ParagraphChange[] {
  const a = oldText.split("\n\n");
  const b = newText.split("\n\n");
  const changes: ParagraphChange[] = [];
  for (let index = 0; index < Math.min(a.length, b.length); index++) {
    if (a[index] === b[index]) continue;
    changes.push({
      index,
      before: a[index],
      after: b[index],
      ops: wordDiff(a[index], b[index]),
    });
  }
  return changes;
}

/** Textseite: Feld `text` im Arbeitsstand (JSON), leer bei kaputtem Stand. */
export function pageText(stateJson: string): string {
  try {
    const v = JSON.parse(stateJson) as { text?: unknown };
    return typeof v.text === "string" ? v.text : "";
  } catch {
    return "";
  }
}

/**
 * Zahl der Absätze vor dem eigentlichen Text: Titelzeile (`# Titel`) und
 * Autorzeile (`von …`) des Transkripts. Sie zählen in der Anzeige nicht mit.
 */
export function leadingParagraphs(text: string): number {
  const [first, second] = text.split("\n\n", 2);
  if (!first?.startsWith("# ")) return 0;
  return /^(von|by) /i.test(second ?? "") ? 2 : 1;
}
