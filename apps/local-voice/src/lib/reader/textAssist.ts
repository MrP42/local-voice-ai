/**
 * KI-Textarbeit mit wenig Token (Editor-Menü).
 *
 * Das Modell bekommt nur nummerierte Zeilen OHNE Zeitstempel, Sprecher und
 * Markdown-Präfix (`12|Text`) und antwortet nur mit dem, was sich ändert:
 *
 * - `grammar`:   `12|korrigierte Zeile` – nur Zeilen mit Fehlern.
 * - `structure`: `12|P` (neuer Absatz ab Zeile 12) oder `12|H2` (Überschrift).
 *
 * So kostet ein fehlerfreier Text fast keine Ausgabe-Token, und auch ein
 * kleines Modell (Haiku, Qwen 3 4B lokal) kommt mit dem Format zurecht.
 * Alles, was das Modell anders liefert, wird verworfen; Präfixe, Tags und
 * Sprecher setzt die App selbst wieder ein.
 */
import { splitLinePrefix } from "./textCleanup";

export type AssistTask = "grammar" | "structure";

export interface AssistInput {
  /** Nummerierte Zeilen für das Modell, eine je Zeile. */
  prompt: string;
  /** Gesendete Zeilen (Index im Gesamttext). */
  sent: number[];
  /** Erste und letzte Zeile des Bereichs (inklusive). */
  first: number;
  last: number;
}

/** Zeilenbereich zu einer Markierung; ohne Markierung der ganze Text. */
export function lineSpan(
  text: string,
  start: number,
  end: number,
): { first: number; last: number } {
  const total = text.split("\n").length;
  if (start >= end) return { first: 0, last: total - 1 };
  const first = text.slice(0, start).split("\n").length - 1;
  const last = text.slice(0, Math.max(start, end - 1)).split("\n").length - 1;
  return { first, last: Math.min(last, total - 1) };
}

const SKIP = /^\s*(?:-{3,}|\*{3,}|_{3,}|⁂|\{\{S\.[^}]*\}\})\s*$/;

export function buildAssistInput(
  text: string,
  start = 0,
  end = 0,
): AssistInput {
  const lines = text.split("\n");
  const { first, last } = lineSpan(text, start, end);
  const sent: number[] = [];
  const rows: string[] = [];
  for (let i = first; i <= last; i++) {
    const line = lines[i];
    if (!line.trim() || SKIP.test(line)) continue;
    const { body } = splitLinePrefix(line);
    if (!body.trim()) continue;
    sent.push(i);
    // 1-basiert: kleine Modelle zählen lieber ab 1.
    rows.push(`${i + 1}|${body.trim()}`);
  }
  return { prompt: rows.join("\n"), sent, first, last };
}

/** Antwortzeilen `N|wert` lesen; alles andere (Erklärungen, `OK`) fällt weg. */
export function parseAssistAnswer(answer: string): Map<number, string> {
  const out = new Map<number, string>();
  for (const raw of answer.split("\n")) {
    const m = /^\s*(?:L)?(\d{1,6})\s*\|\s?(.*)$/.exec(
      raw.replace(/^[-*]\s+/, ""),
    );
    if (!m) continue;
    const value = m[2].trimEnd();
    if (!value.trim()) continue;
    out.set(Number(m[1]) - 1, value);
  }
  return out;
}

const count = (s: string, re: RegExp) => (s.match(re) ?? []).length;

/** Eine Korrektur darf den Sinn nicht verändern: Länge in engen Grenzen,
 *  Tags, Sprecher und Seitenmarken unverändert in der Zahl. */
export function acceptCorrection(before: string, after: string): boolean {
  const a = before.trim();
  const b = after.trim();
  if (!b || a === b) return false;
  const ratio = b.length / Math.max(1, a.length);
  if (a.length >= 12 && (ratio < 0.6 || ratio > 1.6)) return false;
  if (a.length < 12 && Math.abs(b.length - a.length) > 8) return false;
  for (const re of [/\[/g, /</g, /\{\{/g]) {
    if (count(a, re) !== count(b, re)) return false;
  }
  return true;
}

export interface AssistResult {
  text: string;
  changes: number;
  /** Vorschläge, die die Prüfung verworfen hat. */
  rejected: number;
}

/** Grammatik-Antwort anwenden: nur gesendete Zeilen, Präfix bleibt. */
export function applyGrammar(
  text: string,
  input: AssistInput,
  answer: string,
): AssistResult {
  const lines = text.split("\n");
  const sent = new Set(input.sent);
  let changes = 0;
  let rejected = 0;
  for (const [index, value] of parseAssistAnswer(answer)) {
    if (!sent.has(index)) continue;
    const { prefix, body } = splitLinePrefix(lines[index]);
    if (!acceptCorrection(body, value)) {
      if (value.trim() !== body.trim()) rejected += 1;
      continue;
    }
    const trailing = /\s*$/.exec(body)?.[0] ?? "";
    lines[index] = prefix + value.trim() + trailing;
    changes += 1;
  }
  return { text: lines.join("\n"), changes, rejected };
}

/** Mindestens ein Gliederungshinweis je so vieler gesendeter Zeilen. */
const MIN_LINES_PER_MARK = 25;

/**
 * Gliederungs-Antwort anwenden: im gesendeten Bereich werden die Zeilen zu
 * Absätzen gereiht; ein Absatz beginnt an `P`, an jeder Überschrift `H1–H3`,
 * an Leerzeilen, Sprecherwechseln und Markdown-Blöcken. Zeitstempel in
 * angehängten Zeilen entfallen (sie stünden sonst mitten im Absatz).
 *
 * Mit zu wenigen gültigen Hinweisen bleibt der Text unverändert – sonst
 * würde ein schwaches Modell alles zu wenigen Riesenabsätzen verkleben.
 * Vorhandene Überschriften, Zitate und Listenpunkte bleiben eigene Zeilen.
 */
export function applyStructure(
  text: string,
  input: AssistInput,
  answer: string,
): AssistResult {
  const lines = text.split("\n");
  const sent = new Set(input.sent);
  const marks = new Map<number, "P" | 1 | 2 | 3>();
  let rejected = 0;
  for (const [index, value] of parseAssistAnswer(answer)) {
    const v = value.trim().toUpperCase();
    if (!sent.has(index)) continue;
    if (v === "P") marks.set(index, "P");
    else if (/^H[123]$/.test(v)) marks.set(index, Number(v[1]) as 1 | 2 | 3);
    else rejected += 1;
  }
  // Zu wenige Hinweise (stummes oder schwaches Modell) würden alles zu
  // wenigen Riesenabsätzen verkleben: dann lieber nichts ändern.
  if (
    marks.size < Math.max(1, Math.ceil(input.sent.length / MIN_LINES_PER_MARK))
  )
    return { text, changes: 0, rejected };

  const out: string[] = [];
  let para: string | null = null;
  let changes = 0;
  const flush = () => {
    if (para !== null) out.push(para);
    para = null;
  };
  const gap = () => {
    if (out.length > 0 && out[out.length - 1].trim() !== "") out.push("");
  };
  for (let i = input.first; i <= input.last; i++) {
    const line = lines[i];
    const mark = marks.get(i);
    if (!sent.has(i)) {
      // Leerzeilen, Trenner, Seitenmarken: eigene Zeile, Absatz endet.
      flush();
      gap();
      if (line.trim() !== "") out.push(line);
      continue;
    }
    const { prefix, body } = splitLinePrefix(line);
    const speaker = /<[^<>]+>/.exec(prefix)?.[0];
    if (typeof mark === "number") {
      flush();
      gap();
      // Sprecher bleibt vor der Überschrift erhalten (eigene Zeile).
      if (speaker) out.push(speaker);
      out.push(`${"#".repeat(mark)} ${body.trim()}`);
      gap();
      changes += 1;
      continue;
    }
    // Vorhandene Überschriften, Zitate, Listenpunkte bleiben eigene Zeilen
    // und schlucken nichts.
    if (/#|>\s?$|^\s*>|^\s*[-*]\s/.test(prefix.replace(/<[^<>]+>/g, ""))) {
      flush();
      out.push(line.trimEnd());
      continue;
    }
    if (para === null || mark === "P" || speaker) {
      flush();
      gap();
      para = line.trimEnd();
      if (mark === "P") changes += 1;
      continue;
    }
    para = `${para} ${body.trim()}`;
    changes += 1;
  }
  flush();
  // Bereich ersetzen; doppelte Leerzeilen am Rand vermeiden.
  while (out.length > 0 && out[0].trim() === "") out.shift();
  while (out.length > 0 && out[out.length - 1].trim() === "") out.pop();
  const before = lines.slice(0, input.first);
  const after = lines.slice(input.last + 1);
  const next = [...before, ...out, ...after].join("\n");
  return { text: next, changes: next === text ? 0 : changes, rejected };
}
