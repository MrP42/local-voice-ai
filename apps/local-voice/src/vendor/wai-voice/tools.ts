// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
import { type ObjectSchema, SchemaError, validateToolSchema } from "./schema.ts";

/**
 * Werkzeugvertrag für Funktionen, die eine App der Gesprächs-KI anbietet (Vertrag 1).
 * Das Anmelden prüft das Schema (`createToolRegistry`); Ausführung, Rechteprüfung und Audit
 * bleiben im Host (Orchestrierung: `orchestrator.ts`).
 */

/** Aktuelle Fassung des Werkzeugvertrags. */
export const TOOL_CONTRACT_VERSION = 1;

/** Name des eingebauten Werkzeugs, das den Denker ruft; für Hosts reserviert. */
export const DENKER_TOOL_NAME = "denker_fragen";

/** Wie weit die KI ein Werkzeug ohne Rückfrage nutzen darf. */
export type VoiceToolLevel =
  /** Nur lesen: ohne Rückfrage erlaubt. */
  | "lesen"
  /** Ändert Daten: erst nach gesprochener Bestätigung des Nutzers. */
  | "schreiben_mit_bestaetigung"
  /** Für die Sprach-KI nicht verfügbar. */
  | "gesperrt";

export interface VoiceTool {
  /** Vertragsversion; andere Fassungen werden beim Anmelden abgewiesen. */
  vertrag: 1;
  /** Eindeutiger Name, a-z, Ziffern, Unterstrich. */
  name: string;
  /** Beschreibung für das Sprachmodell (Deutsch, ein Satz). */
  description: string;
  /** JSON-Schema der Argumente, nur die Teilmenge aus `schema.ts`. */
  parameters: { type: "object"; properties: Record<string, unknown>; required?: string[] };
  level: VoiceToolLevel;
  /** Wissensbereich (Mandant/Projekt), auf den das Werkzeug beschränkt ist. */
  bereich: string;
  /** Satz, den die KI vor dem Aufruf spricht („Ich schaue kurz nach …“). */
  ankuendigung?: string;
}

export interface VoiceToolResult {
  /** Kurzer, sprechbarer Text für das Modell. */
  text: string;
  /** Bereichslabel des Ergebnisses; weicht es vom Bereich der Sitzung ab, wird es verworfen. */
  bereich: string;
}

/** Führt ein Werkzeug im Host aus; `signal` bricht den Lauf ab (bei `cancel`). */
export type VoiceToolRunner = (
  tool: VoiceTool,
  args: Record<string, unknown>,
  signal?: AbortSignal,
) => Promise<VoiceToolResult>;

/** Ob ein Aufruf ohne Rückfrage laufen darf. */
export function needsConfirmation(tool: VoiceTool): boolean {
  return tool.level === "schreiben_mit_bestaetigung";
}

/**
 * Ob die Sprach-KI das Werkzeug aufrufen darf. Bis S9 (Nachtrag zu ADR-0019) ist nur `lesen`
 * aufrufbar; `schreiben_mit_bestaetigung` lässt sich anmelden, bleibt aber gesperrt.
 */
export function isCallable(tool: VoiceTool): boolean {
  return tool.level === "lesen";
}

/** Friert ein Objekt samt allen Unterobjekten ein und gibt es zurück. */
export function freezeDeep<T>(value: T): T {
  if (typeof value !== "object" || value === null || Object.isFrozen(value)) return value;
  for (const v of Object.values(value)) freezeDeep(v);
  return Object.freeze(value);
}

/** Das Werkzeug wurde beim Anmelden abgewiesen. */
export class ToolRegistrationError extends Error {
  constructor(name: string, grund: string) {
    super(`Werkzeug "${name}" abgewiesen: ${grund}`);
    this.name = "ToolRegistrationError";
  }
}

export interface ToolRegistry {
  /** Meldet ein Werkzeug an; wirft `ToolRegistrationError`. */
  register(tool: VoiceTool): void;
  list(): VoiceTool[];
  /** Nur Werkzeuge, die aufrufbar sind (Schutzstufe `lesen`). */
  callable(): VoiceTool[];
  get(name: string): VoiceTool | undefined;
  /** Das beim Anmelden geprüfte Schema (bereinigte Kopie). */
  schemaOf(name: string): ObjectSchema | undefined;
}

const NAME_PATTERN = /^[a-z][a-z0-9_]{0,63}$/;
const LEVELS: readonly string[] = ["lesen", "schreiben_mit_bestaetigung", "gesperrt"];

function checkTool(tool: VoiceTool): ObjectSchema {
  const name = typeof tool.name === "string" ? tool.name : "?";
  const reject = (grund: string): never => {
    throw new ToolRegistrationError(name, grund);
  };
  if ((tool.vertrag as number) !== TOOL_CONTRACT_VERSION) {
    reject(
      `Vertrag ${String(tool.vertrag)} wird nicht unterstützt (erwartet ${TOOL_CONTRACT_VERSION})`,
    );
  }
  if (!NAME_PATTERN.test(name)) reject("Name muss aus a-z, Ziffern und Unterstrich bestehen");
  if (name === DENKER_TOOL_NAME) reject("Name ist für das eingebaute Werkzeug reserviert");
  if (typeof tool.description !== "string" || !tool.description.trim()) {
    reject("Beschreibung fehlt oder ist kein Text");
  }
  if (typeof tool.bereich !== "string" || !tool.bereich.trim()) {
    reject("Bereich fehlt oder ist kein Text");
  }
  if (!LEVELS.includes(tool.level)) reject("unbekannte Schutzstufe");
  try {
    return validateToolSchema(tool.parameters);
  } catch (e) {
    if (e instanceof SchemaError) return reject(e.message);
    throw e;
  }
}

/** Werkzeugregister: prüft beim Anmelden Vertrag, Name und Schema und weist Abweichungen ab. */
export function createToolRegistry(tools: readonly VoiceTool[] = []): ToolRegistry {
  const entries = new Map<string, { tool: VoiceTool; schema: ObjectSchema }>();
  const registry: ToolRegistry = {
    register(tool) {
      const schema = checkTool(tool);
      if (entries.has(tool.name))
        throw new ToolRegistrationError(tool.name, "Name ist schon vergeben");
      let copy: VoiceTool;
      try {
        copy = freezeDeep(structuredClone(tool));
      } catch {
        throw new ToolRegistrationError(tool.name, "Werkzeug ist nicht kopierbar");
      }
      entries.set(tool.name, { tool: copy, schema: freezeDeep(schema) });
    },
    list: () => [...entries.values()].map((e) => e.tool),
    callable: () => [...entries.values()].map((e) => e.tool).filter(isCallable),
    get: (name) => entries.get(name)?.tool,
    schemaOf: (name) => entries.get(name)?.schema,
  };
  for (const t of tools) registry.register(t);
  return registry;
}
