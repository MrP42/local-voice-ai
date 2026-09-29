// Reine Logik des Besprechungs-Chats (M4, P4e): keine React-, keine
// Tauri-Importe, damit sie sich ohne Browser pruefen laesst (Playwright-Spec
// importiert sie direkt). Typen kommen nur als Typ aus `bindings.ts`.

import type { Coverage, RecipeScope, RecipeSpec, RecipeVar } from "@/bindings";

/** Uebersetzer, wie ihn `useTranslation().t` liefert (hier nur die Form). */
export type Translate = (
  key: string,
  options?: Record<string, unknown>,
) => string;

// ---------------------------------------------------------------------------
// Antworttext
// ---------------------------------------------------------------------------

export type AnswerPart =
  { kind: "text"; text: string } | { kind: "cite"; n: number };

const CITE_MARKER = /\[(\d+(?:\s*,\s*\d+)*)\]/g;

/**
 * Zerlegt den Antworttext in Text und Zitat-Marker `[1]` (auch `[1][2]` und
 * `[1, 2]`). Ist `known` gesetzt, bleiben Nummern ohne Zitat Text - eine
 * Zahl in eckigen Klammern im Fliesstext ist dann kein Beleg.
 */
export const splitAnswer = (
  text: string,
  known?: ReadonlySet<number>,
): AnswerPart[] => {
  const parts: AnswerPart[] = [];
  const pushText = (s: string) => {
    if (s === "") return;
    const last = parts[parts.length - 1];
    if (last && last.kind === "text") last.text += s;
    else parts.push({ kind: "text", text: s });
  };
  let at = 0;
  for (const match of text.matchAll(CITE_MARKER)) {
    const start = match.index ?? 0;
    const numbers = match[1].split(",").map((s) => Number(s.trim()));
    if (known && !numbers.every((n) => known.has(n))) continue;
    pushText(text.slice(at, start));
    for (const n of numbers) parts.push({ kind: "cite", n });
    at = start + match[0].length;
  }
  pushText(text.slice(at));
  return parts;
};

// ---------------------------------------------------------------------------
// Abdeckung (Coverage-Note, M4 §6)
// ---------------------------------------------------------------------------

const K = "meetings.chat.coverage";

/**
 * Die graue Zeile unter jeder Antwort: was durchsucht und gelesen wurde und
 * was nicht. Deterministisch aus `Coverage`; `global` = Chat ueber viele
 * Besprechungen (sonst eine Besprechung bzw. die laufende).
 */
export const formatCoverage = (
  c: Coverage,
  t: Translate,
  opts: { global: boolean },
): string => {
  const parts: string[] = [];
  if (c.live) parts.push(t(`${K}.live`));
  if (opts.global) {
    parts.push(t(`${K}.searched`, { count: c.meetings_in_scope }));
    parts.push(
      t(`${K}.readGlobal`, {
        meetings: t(`${K}.meetings`, { count: c.meetings_read }),
        excerpts: t(`${K}.excerpts`, { count: c.excerpts_read }),
      }),
    );
    const unread = c.meetings_with_hits - c.meetings_read;
    if (unread > 0) parts.push(t(`${K}.unread`, { count: unread }));
    else if (c.truncated) parts.push(t(`${K}.truncated`));
  } else {
    parts.push(
      t(`${K}.read`, {
        excerpts: t(`${K}.excerpts`, { count: c.excerpts_read }),
      }),
    );
    if (c.truncated) parts.push(t(`${K}.truncated`));
  }
  if (c.rounds >= 2) parts.push(t(`${K}.secondRound`));
  if (c.lexical_only) parts.push(t(`${K}.lexicalOnly`));
  if (c.cpu_limited) parts.push(t(`${K}.cpuLimited`));
  if (c.dropped_citations > 0)
    parts.push(t(`${K}.dropped`, { count: c.dropped_citations }));
  return parts.join(" ");
};

// ---------------------------------------------------------------------------
// Recipes (M4 §7)
// ---------------------------------------------------------------------------

const PLACEHOLDER = /\{\{\s*([a-z_]{1,24})\s*\}\}/g;

export type TemplatePart =
  { kind: "text"; text: string } | { kind: "var"; name: string };

/** Text mit `{{name}}` in Text- und Variablenstuecke (fuer Inline-Chips). */
export const splitTemplate = (text: string): TemplatePart[] => {
  const parts: TemplatePart[] = [];
  let at = 0;
  for (const match of text.matchAll(PLACEHOLDER)) {
    const start = match.index ?? 0;
    if (start > at) parts.push({ kind: "text", text: text.slice(at, start) });
    parts.push({ kind: "var", name: match[1] });
    at = start + match[0].length;
  }
  if (at < text.length) parts.push({ kind: "text", text: text.slice(at) });
  return parts;
};

/**
 * Die Variablen eines Recipes in der Reihenfolge ihres ersten Auftretens im
 * Prompt, danach deklarierte, die der Prompt nicht nennt. Ein Platzhalter
 * ohne Deklaration wird als Pflicht-Textvariable gefuehrt (das Backend lehnt
 * ihn ohnehin ab; die UI soll ihn wenigstens ausfuellbar zeigen).
 */
export const recipePlaceholders = (spec: RecipeSpec): RecipeVar[] => {
  const declared = spec.variables ?? [];
  const seen = new Set<string>();
  const out: RecipeVar[] = [];
  for (const match of spec.prompt.matchAll(PLACEHOLDER)) {
    const name = match[1];
    if (seen.has(name)) continue;
    seen.add(name);
    out.push(
      declared.find((v) => v.name === name) ?? {
        name,
        label: name,
        kind: "text",
        required: true,
      },
    );
  }
  for (const v of declared) {
    if (!seen.has(v.name)) {
      seen.add(v.name);
      out.push(v);
    }
  }
  return out;
};

/** Pflichtvariablen ohne Wert (Standardwerte zaehlen als Wert). */
export const missingValues = (
  spec: RecipeSpec,
  values: Record<string, string>,
): RecipeVar[] =>
  recipePlaceholders(spec).filter(
    (v) =>
      (v.required ?? true) && (values[v.name] ?? v.default ?? "").trim() === "",
  );

/** Werte fuer den Aufruf: leere weg, Standardwerte ergaenzt. */
export const recipeValues = (
  spec: RecipeSpec,
  values: Record<string, string>,
): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const v of recipePlaceholders(spec)) {
    const value = (values[v.name] ?? v.default ?? "").trim();
    if (value !== "") out[v.name] = value;
  }
  return out;
};

/** Wie `recipes::check_use` im Backend: passt das Recipe zu diesem Chat? */
export const recipeFits = (
  spec: RecipeSpec,
  ctx: { global: boolean; live: boolean },
): boolean => {
  const scope: RecipeScope = spec.scope;
  if (scope === "meeting" && ctx.global) return false;
  if (scope === "global" && !ctx.global) return false;
  if (ctx.live && !spec.live_ok) return false;
  return true;
};

/** Eingabe beginnt mit `/`: Suchtext fuer das Recipe-Menue, sonst `null`. */
export const slashQuery = (input: string): string | null =>
  input.startsWith("/") && !input.includes("\n")
    ? input.slice(1).trim().toLowerCase()
    : null;

// ---------------------------------------------------------------------------
// Fehler
// ---------------------------------------------------------------------------

/** Codes aus `chat::EVENT_CODES`; alles andere zeigt die UI generisch. */
export const CHAT_ERROR_CODES = [
  "no_provider",
  "no_model",
  "memory_low",
  "recording_active_cpu",
  "chat_busy",
  "empty_scope",
  "llm_failed",
  "cancelled",
  "recipe_invalid",
  "invalid_request",
  "meeting_not_found",
  "thread_not_found",
  "store_failed",
] as const;

/** `<code>` oder `<code>: <art>` (Command-Fehler) auf den Code kuerzen. */
export const chatErrorCode = (error: unknown): string =>
  String(error ?? "")
    .split(":")[0]
    .trim();

/** i18n-Schluessel fuer einen Fehlercode (unbekannte: generischer Text). */
export const chatErrorKey = (code: string): string =>
  (CHAT_ERROR_CODES as readonly string[]).includes(code)
    ? `meetings.chat.errors.${code}`
    : "meetings.chat.errors.unknown";

// ---------------------------------------------------------------------------
// Sonstiges
// ---------------------------------------------------------------------------

/** Kennung einer Anfrage (Schluessel fuer Deltas und Abbruch). */
export const newRequestId = (): string =>
  `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

/** Datum (Sekunden) als `TT.MM.JJJJ` in der Sprache der Oberflaeche. */
export const formatDay = (seconds: number | null, locale: string): string =>
  seconds === null
    ? ""
    : new Intl.DateTimeFormat(locale, {
        day: "2-digit",
        month: "2-digit",
        year: "numeric",
      }).format(new Date(seconds * 1000));

/** `JJJJ-MM-TT` (Datumsfeld) als Sekunden am Tagesbeginn (lokal). */
export const dayToSeconds = (day: string, endOfDay = false): number | null => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  const date = endOfDay
    ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 23, 59, 59)
    : new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Math.floor(date.getTime() / 1000);
};
