/**
 * Reine Logik der Oberflaeche „Automationen“ (B7): die Definition `lva-workflow@1` als
 * bearbeitbarer Entwurf, Befunde der Pruefung je Feld, Werte der Formularfelder.
 * Ohne React und ohne Tauri, damit sie sich mit `node` pruefen laesst.
 */

export interface VariableDecl {
  type: "string" | "number" | "bool" | "list";
  default?: unknown;
  description?: string;
}

export interface StepDef {
  id: string;
  action: string;
  label?: string;
  when?: string;
  params?: Record<string, unknown>;
  on_error?: "fail" | "continue";
  retry?: { max_attempts?: number; backoff_ms?: number };
}

export interface Def {
  schema: string;
  name: string;
  description?: string;
  trigger: { type: string; [key: string]: unknown };
  variables?: Record<string, VariableDecl>;
  steps: StepDef[];
}

export interface Issue {
  path: string;
  message: string;
}

export const SCHEMA_ID = "lva-workflow@1";

/** Ein leerer Ablauf (manueller Ausloeser, keine Schritte). */
export const emptyDef = (): Def => ({
  schema: SCHEMA_ID,
  name: "",
  trigger: { type: "manual" },
  steps: [],
});

/** Liest den JSON-Text einer Definition; `null`, wenn er keine Definition ist. */
export const parseDef = (text: string): Def | null => {
  try {
    const value = JSON.parse(text) as unknown;
    if (!value || typeof value !== "object" || Array.isArray(value))
      return null;
    const v = value as Partial<Def>;
    if (
      typeof v.schema !== "string" ||
      typeof v.name !== "string" ||
      !v.trigger ||
      typeof v.trigger !== "object" ||
      !Array.isArray(v.steps)
    ) {
      return null;
    }
    return v as Def;
  } catch {
    return null;
  }
};

export const serializeDef = (def: Def): string => JSON.stringify(def);

/** Tiefe Kopie (der Entwurf wird nie an Ort und Stelle veraendert). */
export const cloneDef = (def: Def): Def =>
  JSON.parse(JSON.stringify(def)) as Def;

const STEP_ID = /^[a-z][a-z0-9_]{0,31}$/;
export const isValidIdent = (s: string): boolean => STEP_ID.test(s);

/** Kennung fuer einen neuen Schritt: `<Baustein>` aus dem Namen, sonst `schritt<n>`; nie doppelt. */
export const nextStepId = (def: Def, action: string): string => {
  const used = new Set(def.steps.map((s) => s.id));
  const base =
    action
      .split(".")
      .pop()
      ?.toLowerCase()
      .replace(/[^a-z0-9_]/g, "") || "schritt";
  const start = /^[a-z]/.test(base) ? base : `s${base}`;
  if (!used.has(start)) return start.slice(0, 32);
  for (let n = 2; n < 1000; n++) {
    const candidate = `${start.slice(0, 28)}_${n}`;
    if (!used.has(candidate)) return candidate;
  }
  return `schritt_${used.size + 1}`;
};

export const isTemplate = (value: unknown): boolean =>
  typeof value === "string" && value.includes("{{");

/** Setzt (oder entfernt bei leerem Wert) einen Parameter, ohne das Original zu aendern. */
export const withParam = (
  params: Record<string, unknown> | undefined,
  name: string,
  value: unknown,
): Record<string, unknown> => {
  const next = { ...(params ?? {}) };
  if (
    value === undefined ||
    value === null ||
    value === "" ||
    (Array.isArray(value) && value.length === 0)
  ) {
    delete next[name];
  } else {
    next[name] = value;
  }
  return next;
};

/** Ausloeser mit anderem Typ: nur die Art bleibt, Felder der alten Art entfallen. */
export const retypeTrigger = (type: string): Def["trigger"] => ({ type });

/** Verschiebt einen Schritt um `delta` Stellen (-1 hoch, +1 runter); aendert das Original nicht. */
export const moveStep = (steps: StepDef[], index: number, delta: -1 | 1) => {
  const to = index + delta;
  if (to < 0 || to >= steps.length) return steps;
  const next = steps.slice();
  [next[index], next[to]] = [next[to], next[index]];
  return next;
};

// ---------------------------------------------------------------------------
// Befunde
// ---------------------------------------------------------------------------

/** Meldungen genau zu diesem JSON-Zeiger. */
export const issuesAt = (issues: Issue[], path: string): string[] =>
  issues.filter((i) => i.path === path).map((i) => i.message);

/** Meldungen zu diesem Zeiger oder darunter (`/steps/1` umfasst `/steps/1/params/to`). */
export const issuesUnder = (issues: Issue[], path: string): Issue[] =>
  issues.filter((i) => i.path === path || i.path.startsWith(`${path}/`));

/** Zeiger lesbar: `/steps/1/params/to` -> `Schritt 2 › params › to`. */
export const prettyPath = (path: string, steps: StepDef[]): string => {
  if (!path) return "";
  const parts = path.split("/").slice(1);
  const out: string[] = [];
  for (let i = 0; i < parts.length; i++) {
    if (parts[i] === "steps" && parts[i + 1] !== undefined) {
      const idx = Number(parts[i + 1]);
      const id = steps[idx]?.id;
      out.push(`Schritt ${idx + 1}${id ? ` (${id})` : ""}`);
      i++;
    } else {
      out.push(parts[i]);
    }
  }
  return out.join(" › ");
};

// ---------------------------------------------------------------------------
// Feldwerte
// ---------------------------------------------------------------------------

/** Liste als Text fuers Eingabefeld: `wav, mp3`. */
export const listToText = (value: unknown): string =>
  Array.isArray(value) ? value.map(String).join(", ") : "";

export const textToList = (text: string): string[] =>
  text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

/** Beliebiger Wert (`any`) als Text: Strings unveraendert, sonst JSON. */
export const anyToText = (value: unknown): string =>
  value === undefined || value === null
    ? ""
    : typeof value === "string"
      ? value
      : JSON.stringify(value);

/** Text -> Wert (`any`): gueltiges JSON (Zahl, Liste, Objekt) wird geparst, sonst bleibt es Text. */
export const textToAny = (text: string): unknown => {
  const trimmed = text.trim();
  if (!trimmed) return undefined;
  if (/^[[{]/.test(trimmed)) {
    try {
      return JSON.parse(trimmed) as unknown;
    } catch {
      return text;
    }
  }
  return text;
};

/** Ganze Zahl aus einem Eingabefeld; leer oder ungueltig ist `undefined`. */
export const textToInt = (text: string): number | undefined => {
  const t = text.trim();
  if (!/^-?\d+$/.test(t)) return undefined;
  return Number(t);
};

/** Fehlt der Ablauf Name und Schritte? Erst dann ist „Speichern“ nicht sinnlos. */
export const isBlank = (def: Def): boolean =>
  !def.name.trim() && def.steps.length === 0;

/** Zwei Entwuerfe gleich (fuer „ungespeicherte Aenderungen“)? */
export const sameDef = (a: Def, b: Def): boolean =>
  JSON.stringify(a) === JSON.stringify(b);

// ---------------------------------------------------------------------------
// Anzeige
// ---------------------------------------------------------------------------

export const OPEN_RUN_STATES = ["queued", "running", "awaiting_approval"];

export type Tone = "green" | "amber" | "red" | "gray" | "blue";

/** Farbe der Zustandsmarke eines Laufs oder Schritts. */
export const stateTone = (state: string): Tone => {
  switch (state) {
    case "done":
      return "green";
    case "failed":
    case "denied":
      return "red";
    case "awaiting_approval":
    case "retrying":
    case "waiting":
    case "uncertain":
    case "interrupted":
      return "amber";
    case "running":
    case "queued":
    case "planned":
      return "blue";
    default:
      return "gray";
  }
};

/** JSON-Text huebsch (zwei Leerzeichen); kein JSON bleibt, wie es ist. */
export const prettyJson = (text: string | null | undefined): string => {
  if (!text) return "";
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
};
