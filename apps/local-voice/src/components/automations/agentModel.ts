/**
 * Reine Logik der KI-Schritte im Editor (C5, Goal Lokaler Agent, #68): Auswahl der Werkzeuge,
 * Lesen der Vorschau (`workflow_agent_preview`), Quellen-Segmente. Ohne React und ohne Tauri,
 * damit sie sich mit `node` pruefen laesst.
 */

/** Die KI-Schritte, fuer die es eine Vorschau mit Modellentscheidung gibt. */
export const AGENT_ACTIONS = ["agent.route", "agent.extract"] as const;
export type AgentAction = (typeof AGENT_ACTIONS)[number];

export const isAgentAction = (action: string): action is AgentAction =>
  (AGENT_ACTIONS as readonly string[]).includes(action);

/** So viele Werkzeuge hoechstens in der Liste eines Schritts (`agent::policy::MAX_TOOLS`). */
export const MAX_TOOLS = 6;

/** Die Listen, die `agent.extract` ziehen kann (`agent::extract::Kinds`), in Anzeigereihenfolge. */
export const EXTRACT_KINDS = ["todos", "deadlines", "decisions"] as const;

/** Der Wert einer Textliste als `string[]` (alles andere ist leer). */
export const asList = (value: unknown): string[] =>
  Array.isArray(value)
    ? value.filter((v): v is string => typeof v === "string")
    : [];

/**
 * Schaltet einen Eintrag in einer Auswahlliste um. Das Ergebnis folgt der Reihenfolge von
 * `order`; Eintraege, die `order` nicht kennt (z. B. importiert), bleiben hinten stehen.
 */
export const toggleChoice = (
  current: unknown,
  name: string,
  on: boolean,
  order: readonly string[],
): string[] => {
  const set = new Set(asList(current));
  if (on) set.add(name);
  else set.delete(name);
  const known = order.filter((n) => set.has(n));
  const extra = [...set].filter((n) => !order.includes(n));
  return [...known, ...extra];
};

/** Die Segment-Nummer und die Besprechung aus einer Quelle `<Besprechung>:S<n>`. */
export const segmentOf = (
  ref: string,
): { meetingId: string; index: number } | null => {
  const m = /^(.+):S(\d+)$/.exec(ref);
  return m ? { meetingId: m[1], index: Number(m[2]) } : null;
};

/** Konfidenz 0..1 als ganze Prozent; ohne Wert `null`. */
export const percent = (value: number | null | undefined): number | null =>
  typeof value === "number" && Number.isFinite(value)
    ? Math.round(Math.min(1, Math.max(0, value)) * 100)
    : null;

// ---------------------------------------------------------------------------
// Vorschau
// ---------------------------------------------------------------------------

export interface PreviewProvenance {
  model: string;
  local: boolean;
  attempts: number | null;
  promptTokens: number | null;
  completionTokens: number | null;
  durationMs: number | null;
  confidence: number | null;
}

export interface ExtractItem {
  text: string;
  assignee: string | null;
  due: string | null;
  duePhrase: string | null;
  segments: string[];
  quote: string;
  confidence: number | null;
}

export interface AgentPreview {
  kind: "route" | "extract";
  /** Der schwere Platz ist belegt: nichts entschieden, spaeter nochmal. */
  busy: { retryAfterMs: number; message: string } | null;
  /** Keine Entscheidung moeglich (Kontext haengt von einem Vorschritt ab, Beispieltext fehlt). */
  skipped: { reason: string; text: string } | null;
  /** `tool`, `no_action` oder `extracted`. */
  outcome: string;
  tool: string;
  action: string;
  arguments: Array<[string, string]>;
  recipients: string[];
  reason: string;
  reasonText: string;
  signals: string[];
  notes: string[];
  provenance: PreviewProvenance | null;
  todos: ExtractItem[];
  deadlines: ExtractItem[];
  decisions: ExtractItem[];
  summary: string;
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number | null =>
  typeof v === "number" && Number.isFinite(v) ? v : null;
const strings = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
const rec = (v: unknown): Record<string, unknown> =>
  v && typeof v === "object" && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : {};

const items = (v: unknown): ExtractItem[] =>
  (Array.isArray(v) ? v : []).map((raw) => {
    const o = rec(raw);
    return {
      text: str(o.text),
      assignee: str(o.assignee) || null,
      due: str(o.due) || null,
      duePhrase: str(o.due_phrase) || null,
      segments: strings(o.segments),
      quote: str(o.quote),
      confidence: num(o.confidence),
    };
  });

const provenanceOf = (v: unknown): PreviewProvenance | null => {
  const o = rec(v);
  if (Object.keys(o).length === 0) return null;
  return {
    model: str(o.model),
    local: o.local !== false && o.locality !== "remote",
    attempts: num(o.attempts),
    promptTokens: num(o.prompt_tokens),
    completionTokens: num(o.completion_tokens),
    durationMs: num(o.duration_ms),
    confidence: num(o.confidence),
  };
};

/**
 * Liest das Ergebnis von `workflow_agent_preview` oder die Ausgabe eines Agent-Schritts im Lauf
 * (JSON-Text, `kindHint`: die Art, wenn die Ausgabe sie nicht nennt); `null`, wenn es keines ist.
 */
export const parseAgentPreview = (
  text: string,
  kindHint?: AgentPreview["kind"],
): AgentPreview | null => {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  const o = rec(raw);
  const kind =
    o.kind === "extract" || o.kind === "route" ? o.kind : (kindHint ?? null);
  if (!kind) return null;
  const would = rec(o.would_run);
  const argsSource =
    Object.keys(would).length > 0 ? rec(would.arguments) : rec(o.arguments);
  return {
    kind,
    busy:
      o.busy === true
        ? { retryAfterMs: num(o.retry_after_ms) ?? 0, message: str(o.message) }
        : null,
    skipped:
      o.skipped === true
        ? { reason: str(o.reason), text: str(o.reason_text) }
        : null,
    outcome: str(o.outcome),
    tool: str(o.tool),
    action: str(o.action) || str(would.action),
    arguments: Object.entries(argsSource)
      .filter(([, v]) => typeof v === "string" && v !== "")
      .map(([k, v]) => [k, v as string] as [string, string]),
    recipients: strings(
      Object.keys(would).length > 0 ? would.recipients : o.recipients,
    ),
    reason: str(o.reason),
    reasonText: str(o.reason_text),
    signals: strings(o.signals),
    notes: Array.isArray(o.notes)
      ? o.notes.map((n) => (typeof n === "string" ? n : str(rec(n).detail)))
      : [],
    provenance: provenanceOf(o.provenance),
    todos: items(o.todos),
    deadlines: items(o.deadlines),
    decisions: items(o.decisions),
    summary: str(o.summary),
  };
};
