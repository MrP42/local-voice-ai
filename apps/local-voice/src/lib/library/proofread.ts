import type { PostProcessProvider, SyncPoint } from "@/bindings";

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

// ---------------------------------------------------------------- Prüfsumme --

/**
 * Prüfsumme eines Textes (zwei FNV-1a-Läufe, synchron). Sie dient nur dazu zu
 * erkennen, ob der Text nach dem Korrekturlesen bearbeitet wurde, nicht der
 * Sicherheit.
 */
export function hashText(text: string): string {
  let a = 0x811c9dc5;
  let b = 0x01000193 ^ text.length;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    a = Math.imul(a ^ c, 0x01000193);
    b = Math.imul(b ^ c, 0x85ebca6b) + i;
  }
  const hex = (n: number) => (n >>> 0).toString(16).padStart(8, "0");
  return `${hex(a)}${hex(b)}-${text.length.toString(16)}`;
}

// ---------------------------------------------------- Ersetzen je Absatz --

/** Ein Absatz soll von `expect` auf `next` gesetzt werden. */
export interface ParagraphEdit {
  index: number;
  /** Nur ersetzen, wenn der Absatz gerade noch genau so lautet. */
  expect: string;
  next: string;
}

export interface Replaced {
  text: string;
  applied: number[];
  /** Absätze, die inzwischen anders lauten (bearbeitet) oder fehlen. */
  skipped: number[];
}

/**
 * Absätze ersetzen, aber nur dort, wo der Absatz noch dem erwarteten Text
 * entspricht. Alles andere (vom Nutzer bearbeitet) bleibt unangetastet.
 */
export function replaceParagraphs(
  page: string,
  edits: ParagraphEdit[],
): Replaced {
  const parts = page.split("\n\n");
  const applied: number[] = [];
  const skipped: number[] = [];
  for (const e of edits) {
    if (parts[e.index] !== e.expect) {
      skipped.push(e.index);
      continue;
    }
    parts[e.index] = e.next;
    applied.push(e.index);
  }
  return { text: parts.join("\n\n"), applied, skipped };
}

// ------------------------------------------------------ Änderungsansicht --

/** Zustand eines geänderten Absatzes gegenüber dem Text der Seite. */
export type ItemState = "corrected" | "rejected" | "edited";

export interface ReviewItem {
  index: number;
  original: string;
  corrected: string;
  current: string;
  state: ItemState;
  ops: DiffOp[];
}

/** Zustand eines verworfenen Vorschlags. */
export type ProposalState = "open" | "adopted" | "edited";

export interface ProposalItem {
  index: number;
  original: string;
  proposal: string;
  current: string;
  state: ProposalState;
  ops: DiffOp[];
}

export interface Review {
  items: ReviewItem[];
  proposals: ProposalItem[];
  /** Absätze, die nach dem Korrekturlesen von Hand geändert wurden. */
  edited: number;
}

/**
 * Was die Änderungsansicht zeigt: je Absatz, den das Modell geändert hat,
 * ob der Text der Seite noch die Korrektur, wieder das Original oder etwas
 * Eigenes ist. `null`, wenn die Absatzzahlen nicht mehr zusammenpassen.
 */
export function buildReview(
  original: string,
  corrected: string,
  proposals: { index: number; text: string }[],
  page: string,
): Review | null {
  const o = original.split("\n\n");
  const c = corrected.split("\n\n");
  const p = page.split("\n\n");
  if (o.length !== c.length || o.length !== p.length) return null;
  const items: ReviewItem[] = [];
  const proposed = new Map(proposals.map((x) => [x.index, x.text] as const));
  let edited = 0;
  for (let i = 0; i < o.length; i++) {
    if (o[i] === c[i]) {
      if (p[i] !== o[i] && p[i] !== proposed.get(i)) edited++;
      continue;
    }
    const state: ItemState =
      p[i] === c[i] ? "corrected" : p[i] === o[i] ? "rejected" : "edited";
    if (state === "edited") edited++;
    items.push({
      index: i,
      original: o[i],
      corrected: c[i],
      current: p[i],
      state,
      ops: wordDiff(o[i], c[i]),
    });
  }
  const open: ProposalItem[] = [];
  for (const prop of proposals) {
    if (prop.index < 0 || prop.index >= o.length) continue;
    const state: ProposalState =
      p[prop.index] === o[prop.index]
        ? "open"
        : p[prop.index] === prop.text
          ? "adopted"
          : "edited";
    open.push({
      index: prop.index,
      original: o[prop.index],
      proposal: prop.text,
      current: p[prop.index],
      state,
      ops: wordDiff(o[prop.index], prop.text),
    });
  }
  return { items, proposals: open, edited };
}

/** Edits, die jede Korrektur zurücknehmen, die noch unverändert im Text steht. */
export function rejectEdits(review: Review): ParagraphEdit[] {
  return review.items
    .filter((i) => i.state === "corrected")
    .map((i) => ({ index: i.index, expect: i.corrected, next: i.original }));
}

/** Edits, die jede abgelehnte Korrektur wieder einsetzen. */
export function acceptEdits(review: Review): ParagraphEdit[] {
  return review.items
    .filter((i) => i.state === "rejected")
    .map((i) => ({ index: i.index, expect: i.original, next: i.corrected }));
}

// ------------------------------------------------------------- Datenschutz --

/**
 * Ob ein Anbieter auf diesem Rechner läuft, entscheidet allein die Adresse
 * (Schleifenadresse), nicht die Kennung: ein „Ollama“ im Netz ist nicht lokal,
 * auch ein Rechner im LAN nicht. Einzige Ausnahme ist das Modell der App
 * selbst (Kennung `local`, kein Netz). Entspricht `proofread::endpoint_is_local`.
 */
export function isLocalEndpoint(
  provider: Pick<PostProcessProvider, "id" | "base_url">,
): boolean {
  if (provider.id === "local") return true;
  return isLoopbackUrl(provider.base_url ?? "");
}

export function isLoopbackUrl(url: string): boolean {
  const lower = url.trim().toLowerCase();
  const rest = lower.includes("://") ? lower.split("://")[1] : lower;
  const authority = (rest.split(/[/?#]/)[0] ?? "").split("@").pop() ?? "";
  const host = authority.startsWith("[")
    ? authority.slice(1).split("]")[0]
    : authority.split(":")[0];
  if (host === "localhost" || host === "::1") return true;
  const m = /^127\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  return !!m && m.slice(1).every((o) => Number(o) <= 255);
}
