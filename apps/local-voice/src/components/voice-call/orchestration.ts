import { invoke } from "@tauri-apps/api/core";
import {
  commands,
  type AppSettings,
  type Assessment,
  type ModelCompliance,
} from "@/bindings";
import { modelLabel } from "@/lib/modelNames";
import type { ChatMessage, ChatSide } from "@/vendor/wai-voice/backend.ts";
import {
  modelAllowed,
  type ModelMessage,
  type Souveraenitaet,
  type VoiceModel,
} from "@/vendor/wai-voice/model.ts";
import { createOrchestrator } from "@/vendor/wai-voice/orchestrator.ts";
import {
  createToolRegistry,
  type VoiceTool,
  type VoiceToolResult,
} from "@/vendor/wai-voice/tools.ts";

// Der Orchestrator aus @wai/voice (vendort) im Sprachgespräch, Stufe 2:
// Sprecher = das aktive Sprachmodell, Denker = ein zweites Modell (optional),
// Werkzeuge = die lesenden Werkzeuge der Agentenbrücke (`voice_tools_list`,
// `voice_tool_call`). Ohne Denker und ohne Werkzeuge baut die Engine keinen
// Orchestrator; der alte Weg (`voice_call_chat`) bleibt unverändert.

export type { Souveraenitaet };

/** Stufen der Souveränität, die das Gespräch anbietet (ohne Rückfall auf „frei“). */
export const SOUVERAENITAET_STUFEN: readonly Souveraenitaet[] = [
  "nur_lokal",
  "eu",
  "frei",
];

export const DEFAULT_SOUVERAENITAET: Souveraenitaet = "nur_lokal";

/** Wissensbereich der Sitzung: Local Voice AI kennt nur einen (die eigenen Daten). */
export const SESSION_BEREICH = "lva";

/** Einstellungen des Orchestrators (Teil von `CallConfig`). */
export interface OrchestrationConfig {
  /** Kennung des Denker-Modells (`llm_models[].id`); leer = kein Denker. */
  denkerModelId: string;
  souveraenitaet: Souveraenitaet;
  /** Lesende LVA-Werkzeuge nutzen. */
  useTools: boolean;
}

export const DEFAULT_ORCHESTRATION: OrchestrationConfig = {
  denkerModelId: "",
  souveraenitaet: DEFAULT_SOUVERAENITAET,
  useTools: true,
};

/** Was die Oberfläche zeigt, wenn der Orchestrator etwas tut oder nicht darf. */
export type CallStatus =
  | { kind: "tool"; tool: string }
  | { kind: "notice"; code: "speaker_blocked" | "denker_blocked" };

/** Werkzeug, wie `voice_tools_list` es liefert (Rust: `VoiceToolInfo`). */
export interface VoiceToolInfo {
  name: string;
  title: string;
  description: string;
  parameters_json: string;
  capability: string;
}

/** Ein Modell der Auswahl samt Herkunft (für die Souveränitätsprüfung). */
export interface ModelDescription {
  id: string;
  label: string;
  local: boolean;
  jurisdiction?: "eu" | "other";
}

/** Verbrauch eines Durchgangs, wie `voice_call_turn` ihn meldet. */
export interface TurnUsage {
  prompt_tokens: number;
  completion_tokens: number;
  usage_event_id?: number | null;
  usage_event_ids?: number[];
  provider_id?: string | null;
  local?: boolean;
}

export interface TurnEvent {
  kind: string;
  text?: string;
  usage?: TurnUsage;
}

export interface OrchestrationDeps {
  /** Hört auf die Ereignisse einer Anfrage (`voice-call`); gibt die Abmeldung zurück. */
  subscribe(
    requestId: string,
    handler: (e: TurnEvent) => void,
  ): Promise<() => void>;
  onUsage(usage: TurnUsage): void;
  onStatus(status: CallStatus | null): void;
}

export interface Prepared {
  /** `null`: nichts zu orchestrieren, der alte Weg gilt. */
  side: ChatSide | null;
}

/** Ländercodes der EU (ISO 3166-1 alpha-2): dort sitzende Anbieter gelten als „EU“. */
const EU_COUNTRIES = new Set([
  "AT",
  "BE",
  "BG",
  "HR",
  "CY",
  "CZ",
  "DK",
  "EE",
  "FI",
  "FR",
  "DE",
  "GR",
  "HU",
  "IE",
  "IT",
  "LV",
  "LT",
  "LU",
  "MT",
  "NL",
  "PL",
  "PT",
  "RO",
  "SK",
  "SI",
  "ES",
  "SE",
  "EU",
]);

/** Ein Cloud-Modell zählt als EU, wenn alle bekannten Verarbeitungsländer in der EU liegen. */
export function jurisdictionOf(a: Assessment): "eu" | "other" {
  return a.countries.length > 0 &&
    a.countries.every((c) => EU_COUNTRIES.has(c.toUpperCase()))
    ? "eu"
    : "other";
}

/** Die freigegebenen Modelle der Einstellungen mit Herkunft. */
export function describeModels(
  settings: Pick<AppSettings, "llm_models" | "llm_connections">,
  assessed: Record<string, Assessment>,
): ModelDescription[] {
  const connections = new Map(
    (settings.llm_connections ?? []).map((c) => [c.id, c]),
  );
  return (settings.llm_models ?? []).flatMap((m): ModelDescription[] => {
    const connection = connections.get(m.connection_id);
    if (!connection || m.enabled === false || connection.enabled === false)
      return [];
    const assessment = assessed[m.id];
    const local = assessment ? !assessment.cloud : connection.kind === "local";
    return [
      {
        id: m.id,
        label: modelLabel(m.remote_id, m.label),
        local,
        ...(local || !assessment
          ? {}
          : { jurisdiction: jurisdictionOf(assessment) }),
      },
    ];
  });
}

export function assessedById(
  list: ModelCompliance[] | null | undefined,
): Record<string, Assessment> {
  const out: Record<string, Assessment> = {};
  for (const m of list ?? []) out[m.model_id] = m.assessment;
  return out;
}

/** Bei „nur lokal“ sperrt die Stufe jedes Cloud-Modell; so meldet es die Oberfläche. */
export function blockedBy(
  model: ModelDescription,
  stufe: Souveraenitaet,
): boolean {
  return !modelAllowed(toVoiceModelShape(model), stufe);
}

function toVoiceModelShape(m: ModelDescription): VoiceModel {
  return {
    id: m.id,
    label: m.label,
    local: m.local,
    jurisdiction: m.jurisdiction,
    supportsTools: false,
    turn: () => Promise.reject(new Error("nicht aufrufbar")),
  };
}

const ANNOUNCE: Record<string, string> = {
  meetings_list: "Moment, ich schaue in den Besprechungen nach.",
  meetings_search: "Moment, ich suche in den Besprechungen.",
  meeting_transcript: "Moment, ich lese im Transkript nach.",
};

/** Wie lange nach dem Rückgabewert noch auf den Verbrauch gewartet wird (ms). */
const USAGE_WAIT_MS = 1500;

function wireMessages(
  messages: ModelMessage[],
): { role: string; content: string }[] {
  return messages.map((m) => ({
    role: m.role === "tool" ? "user" : m.role,
    content: m.content,
  }));
}

let turnCounter = 0;

/**
 * Ein Modell als `VoiceModel`: jeder Durchgang ist ein `voice_call_turn` mit eigener
 * Kennung (so bricht der Abbruch genau ihn ab). `modelId` leer = aktives Sprachmodell.
 */
function makeModel(
  info: ModelDescription,
  modelId: string,
  deps: OrchestrationDeps,
): VoiceModel {
  return {
    id: info.id,
    label: info.label,
    local: info.local,
    jurisdiction: info.jurisdiction,
    supportsTools: false,
    async turn(req) {
      const turnId = `${req.requestId}~${turnCounter++}`;
      let usageSeen!: () => void;
      const usageP = new Promise<void>((r) => (usageSeen = r));
      let onText = req.onText;
      const off = await deps.subscribe(turnId, (e) => {
        if (e.kind === "delta" && e.text) onText(e.text);
        if (e.kind === "usage" && e.usage) deps.onUsage(e.usage);
        if (e.kind === "usage" || e.kind === "failed") usageSeen();
      });
      const cancel = () =>
        void invoke("voice_call_cancel", { requestId: turnId }).catch(
          () => undefined,
        );
      if (req.signal.aborted) cancel();
      req.signal.addEventListener("abort", cancel, { once: true });
      try {
        const text = await invoke<string>("voice_call_turn", {
          requestId: turnId,
          messages: wireMessages(req.messages),
          modelId: modelId || null,
        });
        if (!req.signal.aborted)
          await Promise.race([
            usageP,
            new Promise<void>((r) => setTimeout(r, USAGE_WAIT_MS)),
          ]);
        onText = () => undefined;
        return { text, toolCalls: [] };
      } finally {
        req.signal.removeEventListener("abort", cancel);
        off();
      }
    },
  };
}

function parseParameters(json: string): VoiceTool["parameters"] | null {
  try {
    const parsed: unknown = JSON.parse(json);
    if (typeof parsed !== "object" || parsed === null) return null;
    return parsed as VoiceTool["parameters"];
  } catch {
    return null;
  }
}

/** Werkzeuge der Brücke als Registrierung; was der Vertrag nicht annimmt, fehlt. */
export function toolsFrom(infos: VoiceToolInfo[]): VoiceTool[] {
  const registry = createToolRegistry();
  const accepted: VoiceTool[] = [];
  for (const info of infos) {
    const parameters = parseParameters(info.parameters_json);
    if (!parameters) continue;
    const tool: VoiceTool = {
      vertrag: 1,
      name: info.name,
      description: info.description,
      parameters,
      level: "lesen",
      bereich: SESSION_BEREICH,
      ankuendigung: ANNOUNCE[info.name],
    };
    try {
      registry.register(tool);
      accepted.push(tool);
    } catch {
      // Ein Werkzeug außerhalb der erlaubten Schema-Teilmenge wird nicht angeboten.
    }
  }
  return accepted;
}

async function listTools(): Promise<VoiceToolInfo[]> {
  try {
    const list = await invoke<VoiceToolInfo[] | null>("voice_tools_list");
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

async function loadModels(): Promise<{
  active: ModelDescription | null;
  all: ModelDescription[];
}> {
  const [settings, compliance] = await Promise.all([
    commands.getAppSettings().catch(() => null),
    commands.complianceAssessModels().catch(() => null),
  ]);
  if (!settings || settings.status !== "ok") return { active: null, all: [] };
  const all = describeModels(settings.data, assessedById(compliance));
  const activeId = settings.data.llm_active_model_id ?? null;
  return { active: all.find((m) => m.id === activeId) ?? null, all };
}

async function runTool(
  tool: VoiceTool,
  args: Record<string, unknown>,
): Promise<VoiceToolResult> {
  const text = await invoke<string>("voice_tool_call", {
    name: tool.name,
    args: JSON.stringify(args),
  });
  return { text, bereich: tool.bereich };
}

/**
 * Baut den Orchestrator für einen Gesprächsschritt, oder `side: null`, wenn nichts zu
 * orchestrieren ist (kein Denker, keine Werkzeuge) oder der Sprecher bei der gewählten
 * Souveränität nicht zulässig ist (dann gilt der alte Weg ohne Werkzeuge und Denker, damit
 * Werkzeugdaten nie zu einem gesperrten Cloud-Modell gehen).
 */
export async function prepareOrchestration(
  cfg: OrchestrationConfig & { systemPrompt: string },
  deps: OrchestrationDeps,
): Promise<Prepared> {
  const wantsDenker = cfg.denkerModelId !== "";
  const offered = cfg.useTools ? toolsFrom(await listTools()) : [];
  if (offered.length === 0 && !wantsDenker) return { side: null };

  const { active, all } = await loadModels();
  if (!active || blockedBy(active, cfg.souveraenitaet)) {
    deps.onStatus({ kind: "notice", code: "speaker_blocked" });
    return { side: null };
  }
  const denkerInfo = wantsDenker
    ? all.find((m) => m.id === cfg.denkerModelId)
    : undefined;
  if (denkerInfo && blockedBy(denkerInfo, cfg.souveraenitaet)) {
    deps.onStatus({ kind: "notice", code: "denker_blocked" });
  }
  const denker =
    denkerInfo && !blockedBy(denkerInfo, cfg.souveraenitaet)
      ? makeModel(denkerInfo, denkerInfo.id, deps)
      : undefined;
  if (offered.length === 0 && !denker) return { side: null };

  const names = new Set(offered.map((t) => t.name));
  const side = createOrchestrator({
    sprecher: makeModel(active, "", deps),
    denker,
    tools: offered,
    bereich: SESSION_BEREICH,
    souveraenitaet: cfg.souveraenitaet,
    systemPrompt: cfg.systemPrompt,
    authorize: async (tool, _args, bereich) =>
      bereich === SESSION_BEREICH &&
      tool.bereich === bereich &&
      names.has(tool.name) &&
      !blockedBy(active, cfg.souveraenitaet),
    run: async (tool, args) => {
      deps.onStatus({ kind: "tool", tool: tool.name });
      return runTool(tool, args);
    },
  });
  return { side };
}

export type { ChatMessage };
