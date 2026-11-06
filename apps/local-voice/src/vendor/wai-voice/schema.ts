// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
/**
 * Feste Teilmenge von JSON Schema für Werkzeugargumente (ADR-0021, Codex-Auflage 1).
 * Erlaubt: `type`, `properties`, `required`, `enum`, `minimum`/`maximum`, `maxLength`, `maxItems`, `items`
 * (dazu die Anmerkung `description` und `additionalProperties: false`). Alles andere wird beim
 * Anmelden abgewiesen, nicht gekürzt. Die Fehlermeldungen nennen Pfad und Schlüssel, nie Werte.
 */

export type SchemaNode = StringSchema | NumberSchema | BooleanSchema | ArraySchema | ObjectSchema;

export interface StringSchema {
  type: "string";
  description?: string;
  enum?: string[];
  maxLength?: number;
}

export interface NumberSchema {
  type: "number" | "integer";
  description?: string;
  enum?: number[];
  minimum?: number;
  maximum?: number;
}

export interface BooleanSchema {
  type: "boolean";
  description?: string;
}

export interface ArraySchema {
  type: "array";
  description?: string;
  items: SchemaNode;
  maxItems?: number;
}

export interface ObjectSchema {
  type: "object";
  description?: string;
  properties: Record<string, SchemaNode>;
  required?: string[];
  additionalProperties?: false;
}

export type ValidationResult = { ok: true } | { ok: false; reason: string };

/** Das Schema liegt außerhalb der erlaubten Teilmenge. */
export class SchemaError extends Error {
  readonly pfad: string;
  readonly grund: string;
  constructor(pfad: string, grund: string) {
    super(`Schema abgewiesen bei ${pfad}: ${grund}`);
    this.name = "SchemaError";
    this.pfad = pfad;
    this.grund = grund;
  }
}

const MAX_DEPTH = 6;
/** Standardgrenze für Texte ohne `maxLength`. */
export const DEFAULT_MAX_LENGTH = 2000;
/** Standardgrenze für Listen ohne `maxItems`. */
export const DEFAULT_MAX_ITEMS = 50;
/** Namen, die der Host setzt und die das Modell nie als Argument liefern darf. */
const RESERVED_NAMES = new Set([
  "bereich",
  "tenant",
  "mandant",
  "scope",
  "souveraenitaet",
  "user",
  "nutzer",
]);
const UNSUPPORTED = new Set(["$ref", "$defs", "definitions", "oneOf", "anyOf", "allOf", "not"]);
const COMMON_KEYS = ["type", "description"];
const KEYS_BY_TYPE: Record<SchemaNode["type"], string[]> = {
  string: ["enum", "maxLength"],
  number: ["enum", "minimum", "maximum"],
  integer: ["enum", "minimum", "maximum"],
  boolean: [],
  array: ["items", "maxItems"],
  object: ["properties", "required", "additionalProperties"],
};

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isNodeType(v: unknown): v is SchemaNode["type"] {
  return typeof v === "string" && Object.hasOwn(KEYS_BY_TYPE, v);
}

function finiteNumber(v: unknown, pfad: string, key: string): number {
  if (typeof v !== "number" || !Number.isFinite(v)) {
    throw new SchemaError(pfad, `"${key}" muss eine endliche Zahl sein`);
  }
  return v;
}

function parseNode(raw: unknown, pfad: string, depth: number): SchemaNode {
  if (!isRecord(raw)) throw new SchemaError(pfad, "Schema muss ein Objekt sein");
  if (depth > MAX_DEPTH) throw new SchemaError(pfad, `Verschachtelung tiefer als ${MAX_DEPTH}`);
  const type = raw.type;
  if (!isNodeType(type)) {
    const hinweis = Object.keys(raw).find((k) => UNSUPPORTED.has(k));
    throw new SchemaError(
      pfad,
      hinweis
        ? `"${hinweis}" wird nicht unterstützt`
        : `"type" muss string, number, integer, boolean, array oder object sein`,
    );
  }
  const allowed = new Set([...COMMON_KEYS, ...KEYS_BY_TYPE[type]]);
  for (const key of Object.keys(raw)) {
    if (allowed.has(key)) continue;
    const warum = UNSUPPORTED.has(key)
      ? "wird nicht unterstützt"
      : `ist für type "${type}" nicht erlaubt`;
    throw new SchemaError(pfad, `Schlüssel "${key}" ${warum}`);
  }
  const base: { description?: string } = {};
  if (raw.description !== undefined) {
    if (typeof raw.description !== "string") {
      throw new SchemaError(pfad, `"description" muss ein Text sein`);
    }
    base.description = raw.description;
  }
  switch (type) {
    case "string":
      return { type, ...base, maxLength: DEFAULT_MAX_LENGTH, ...parseStringKeys(raw, pfad) };
    case "number":
    case "integer":
      return { type, ...base, ...parseNumberKeys(raw, pfad) };
    case "boolean":
      return { type, ...base };
    case "array":
      if (raw.items === undefined) throw new SchemaError(pfad, `"items" fehlt`);
      return {
        type,
        ...base,
        items: parseNode(raw.items, `${pfad}.items`, depth + 1),
        maxItems: parseMaxItems(raw, pfad),
      };
    case "object":
      return { type, ...base, ...parseObjectKeys(raw, pfad, depth) };
  }
}

function parseMaxItems(raw: Record<string, unknown>, pfad: string): number {
  if (raw.maxItems === undefined) return DEFAULT_MAX_ITEMS;
  const n = finiteNumber(raw.maxItems, pfad, "maxItems");
  if (!Number.isInteger(n) || n < 0) {
    throw new SchemaError(pfad, `"maxItems" muss eine ganze Zahl ≥ 0 sein`);
  }
  return n;
}

function parseEnum<T extends string | number>(
  raw: unknown,
  pfad: string,
  kind: "string" | "number",
): T[] {
  if (!Array.isArray(raw) || raw.length === 0) {
    throw new SchemaError(pfad, `"enum" muss eine nichtleere Liste sein`);
  }
  for (const v of raw) {
    const ok =
      kind === "string" ? typeof v === "string" : typeof v === "number" && Number.isFinite(v);
    if (!ok) throw new SchemaError(pfad, `"enum" enthält einen Wert vom falschen Typ (${kind})`);
  }
  return [...raw] as T[];
}

function parseStringKeys(raw: Record<string, unknown>, pfad: string): Partial<StringSchema> {
  const out: Partial<StringSchema> = {};
  if (raw.enum !== undefined) out.enum = parseEnum<string>(raw.enum, pfad, "string");
  if (raw.maxLength !== undefined) {
    const n = finiteNumber(raw.maxLength, pfad, "maxLength");
    if (!Number.isInteger(n) || n < 0) {
      throw new SchemaError(pfad, `"maxLength" muss eine ganze Zahl ≥ 0 sein`);
    }
    out.maxLength = n;
  }
  return out;
}

function parseNumberKeys(raw: Record<string, unknown>, pfad: string): Partial<NumberSchema> {
  const out: Partial<NumberSchema> = {};
  if (raw.enum !== undefined) out.enum = parseEnum<number>(raw.enum, pfad, "number");
  if (raw.minimum !== undefined) out.minimum = finiteNumber(raw.minimum, pfad, "minimum");
  if (raw.maximum !== undefined) out.maximum = finiteNumber(raw.maximum, pfad, "maximum");
  if (out.minimum !== undefined && out.maximum !== undefined && out.minimum > out.maximum) {
    throw new SchemaError(pfad, `"minimum" ist größer als "maximum"`);
  }
  return out;
}

function parseObjectKeys(
  raw: Record<string, unknown>,
  pfad: string,
  depth: number,
): Pick<ObjectSchema, "properties" | "required" | "additionalProperties"> {
  if (!isRecord(raw.properties))
    throw new SchemaError(pfad, `"properties" fehlt oder ist kein Objekt`);
  if (raw.additionalProperties !== undefined && raw.additionalProperties !== false) {
    throw new SchemaError(pfad, `freie "additionalProperties" sind nicht erlaubt (nur false)`);
  }
  const properties: Record<string, SchemaNode> = {};
  for (const [name, sub] of Object.entries(raw.properties)) {
    if (RESERVED_NAMES.has(name.toLowerCase())) {
      throw new SchemaError(pfad, `Eigenschaftsname "${name}" ist reserviert (setzt der Host)`);
    }
    if (name === "" || name === "__proto__") {
      throw new SchemaError(pfad, `Eigenschaftsname "${name}" ist nicht erlaubt`);
    }
    properties[name] = parseNode(sub, `${pfad}.${name}`, depth + 1);
  }
  const out: Pick<ObjectSchema, "properties" | "required" | "additionalProperties"> = {
    properties,
    additionalProperties: false,
  };
  if (raw.required !== undefined) {
    if (!Array.isArray(raw.required) || raw.required.some((r) => typeof r !== "string")) {
      throw new SchemaError(pfad, `"required" muss eine Liste von Namen sein`);
    }
    const required = raw.required as string[];
    const missing = required.find((r) => !Object.hasOwn(properties, r));
    if (missing !== undefined) {
      throw new SchemaError(pfad, `"required" nennt "${missing}", das in "properties" fehlt`);
    }
    out.required = [...required];
  }
  return out;
}

/**
 * Prüft ein Argumentschema gegen die Teilmenge und liefert eine bereinigte Kopie.
 * Wirft `SchemaError` mit Pfad und Grund. Die Wurzel muss `type: "object"` sein.
 */
export function validateToolSchema(schema: unknown): ObjectSchema {
  const node = parseNode(schema, "$", 0);
  if (node.type !== "object") throw new SchemaError("$", `die Wurzel muss type "object" sein`);
  return node;
}

function fail(reason: string): ValidationResult {
  return { ok: false, reason };
}

function checkNumber(schema: NumberSchema, v: number, pfad: string): ValidationResult {
  if (schema.type === "integer" && !Number.isInteger(v))
    return fail(`${pfad}: muss ganzzahlig sein`);
  if (schema.minimum !== undefined && v < schema.minimum) {
    return fail(`${pfad}: muss ≥ ${schema.minimum} sein`);
  }
  if (schema.maximum !== undefined && v > schema.maximum) {
    return fail(`${pfad}: muss ≤ ${schema.maximum} sein`);
  }
  return { ok: true };
}

function checkObject(schema: ObjectSchema, v: unknown, pfad: string): ValidationResult {
  if (!isRecord(v)) return fail(`${pfad}: muss ein Objekt sein`);
  for (const key of Object.keys(v)) {
    if (!Object.hasOwn(schema.properties, key)) return fail(`${pfad}.${key}: unbekanntes Argument`);
  }
  for (const key of schema.required ?? []) {
    if (!Object.hasOwn(v, key) || v[key] === undefined) return fail(`${pfad}.${key}: fehlt`);
  }
  for (const [key, sub] of Object.entries(schema.properties)) {
    if (!Object.hasOwn(v, key) || v[key] === undefined) continue;
    const r = check(sub, v[key], `${pfad}.${key}`);
    if (!r.ok) return r;
  }
  return { ok: true };
}

function check(schema: SchemaNode, v: unknown, pfad: string): ValidationResult {
  switch (schema.type) {
    case "string":
      if (typeof v !== "string") return fail(`${pfad}: muss ein Text sein`);
      if (schema.maxLength !== undefined && v.length > schema.maxLength) {
        return fail(`${pfad}: höchstens ${schema.maxLength} Zeichen`);
      }
      if (schema.enum && !schema.enum.includes(v)) return fail(`${pfad}: kein erlaubter Wert`);
      return { ok: true };
    case "number":
    case "integer":
      if (typeof v !== "number" || !Number.isFinite(v)) return fail(`${pfad}: muss eine Zahl sein`);
      if (schema.enum && !schema.enum.includes(v)) return fail(`${pfad}: kein erlaubter Wert`);
      return checkNumber(schema, v, pfad);
    case "boolean":
      return typeof v === "boolean" ? { ok: true } : fail(`${pfad}: muss wahr oder falsch sein`);
    case "array": {
      if (!Array.isArray(v)) return fail(`${pfad}: muss eine Liste sein`);
      if (schema.maxItems !== undefined && v.length > schema.maxItems) {
        return fail(`${pfad}: höchstens ${schema.maxItems} Einträge`);
      }
      for (let i = 0; i < v.length; i++) {
        const r = check(schema.items, v[i], `${pfad}[${i}]`);
        if (!r.ok) return r;
      }
      return { ok: true };
    }
    case "object":
      return checkObject(schema, v, pfad);
  }
}

/** Prüft Argumente vor jedem Aufruf; unbekannte Argumente gelten als Fehler. Nennt nie Werte. */
export function validateArgs(schema: SchemaNode, args: unknown): ValidationResult {
  return check(schema, args, "$");
}
