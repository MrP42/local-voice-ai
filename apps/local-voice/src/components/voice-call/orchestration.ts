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
  /** Das laufende Gespräch (es bleibt den Gesprächswerkzeugen verborgen); `null` bis zum ersten Beitrag; wartet auf ein laufendes Speichern. */
  conversationId?(): string | null | Promise<string | null>;
}

export interface Prepared {
  /** `null`: nichts zu orchestrieren, der alte Weg gilt (außer bei `blocked`). */
  side: ChatSide | null;
  /** Der Sprecher ist bei der Souveränität gesperrt: es gibt kein Gespräch mit diesem Modell. */
  blocked: boolean;
  /** Systemanweisung samt Hinweis auf nicht freigegebene Wissensbereiche (gilt auch für den alten Weg). */
  systemPrompt: string;
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
  dictations_recent: "Moment, ich schaue in den Diktaten nach.",
  dictations_search: "Moment, ich suche in den Diktaten.",
  conversations_list: "Moment, ich schaue in den früheren Gesprächen nach.",
  conversations_search: "Moment, ich suche in den früheren Gesprächen.",
  conversation_get: "Moment, ich lese das frühere Gespräch.",
  calendar_events: "Moment, ich schaue in den Terminen nach.",
  day_overview: "Moment, ich verschaffe mir einen Überblick über den Tag.",
  tts_pages_list: "Moment, ich schaue in den Vorlesen-Seiten nach.",
  tts_page_text: "Moment, ich lese die Seite.",
};

/** Wissensbereiche des Gesprächs (Schlüssel der Einstellung `voice_call_knowledge`). */
export const KNOWLEDGE_AREAS = [
  "meetings",
  "dictations",
  "conversations",
  "calendar",
  "tts",
] as const;
export type KnowledgeArea = (typeof KNOWLEDGE_AREAS)[number];

/** Deutsche Namen der Bereiche für den Hinweis an das Modell (Transkripte hängen an den Besprechungen). */
const AREA_LABEL: Record<KnowledgeArea | "transcripts", string> = {
  meetings: "Besprechungen",
  transcripts: "Transkripte",
  dictations: "Diktate",
  conversations: "frühere Gespräche",
  calendar: "Termine",
  tts: "Vorlesen-Seiten",
};

/** Die freigegebenen Bereiche samt Unterschalter, wie die Einstellungen sie halten. */
export interface KnowledgeState {
  areas: KnowledgeArea[];
  /** `voice_call_read_transcripts`: wirkt nur mit „Besprechungen“. */
  transcripts: boolean;
}

export const NO_KNOWLEDGE: KnowledgeState = { areas: [], transcripts: false };

export function knowledgeFrom(
  settings: Pick<
    AppSettings,
    "voice_call_knowledge" | "voice_call_read_transcripts"
  > | null,
): KnowledgeState {
  const raw = settings?.voice_call_knowledge ?? [];
  return {
    areas: KNOWLEDGE_AREAS.filter((a) => raw.includes(a)),
    transcripts: settings?.voice_call_read_transcripts ?? false,
  };
}

/** Kein Bereich frei (der Standard): das Gespräch bekommt keine Wissenswerkzeuge. */
export function noKnowledge(k: KnowledgeState): boolean {
  return k.areas.length === 0;
}

const HINT_ORDER = [
  "meetings",
  "transcripts",
  "dictations",
  "conversations",
  "calendar",
  "tts",
] as const;

/** Namen der Bereiche, die das Modell nicht sieht (Transkripte nur, wenn die Besprechungen frei sind). */
export function missingAreas(k: KnowledgeState): string[] {
  return HINT_ORDER.filter((a) =>
    a === "transcripts"
      ? k.areas.includes("meetings") && !k.transcripts
      : !k.areas.includes(a),
  ).map((a) => AREA_LABEL[a]);
}

/**
 * Kurzer Hinweis an das Modell, welche Bereiche es nicht sieht. Leer, wenn alles frei ist. Das Modell
 * soll nie behaupten, die App habe solche Daten nicht, sondern auf die Freigabe verweisen.
 */
export function knowledgeHint(k: KnowledgeState): string {
  const missing = missingAreas(k);
  if (missing.length === 0) return "";
  const list =
    missing.length === 1
      ? missing[0]
      : `${missing.slice(0, -1).join(", ")} und ${missing[missing.length - 1]}`;
  return `Die App kennt Besprechungen, Diktate, frühere Gespräche, Termine und Vorlesen-Seiten des Nutzers. Dir sind davon nicht freigegeben: ${list}. Behaupte nie, die App habe solche Daten nicht oder du hättest keinen Zugriff auf sie. Sage stattdessen kurz, dass der Nutzer sie unter „Gesprächseinstellungen → Wissen“ freigeben kann.`;
}

/** Systemanweisung samt Hinweis auf fehlende Bereiche. */
export function withKnowledgeHint(prompt: string, k: KnowledgeState): string {
  const hint = knowledgeHint(k);
  return hint ? `${prompt}\n\n${hint}` : prompt;
}

/** Wie lange nach dem Rückgabewert noch auf den Verbrauch gewartet wird (ms). */
const USAGE_WAIT_MS = 1500;
/** Wächter: ein Modell-Durchgang darf höchstens so lange laufen (ms), ein Werkzeug so lange. */
export const TURN_TIMEOUT_MS = 20_000;
export const TOOL_TIMEOUT_MS = 10_000;
/** Gesprochen, wenn der Wächter anschlägt. */
export const TURN_TIMEOUT_TEXT = "Das Modell antwortet gerade nicht.";
export const TOOL_TIMEOUT_TEXT =
  "Das Werkzeug hat nicht rechtzeitig geantwortet.";

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
 * Ein Modell als `VoiceModel`: jeder Durchgang ist ein `voice_call_turn` mit eigener Kennung
 * (so bricht der Abbruch genau ihn ab) und ausdrücklicher Modellkennung samt Souveränitätsstufe
 * (Rust lehnt ein unzulässiges Modell selbst ab). Ein Wächter bricht einen hängenden Durchgang
 * nach `TURN_TIMEOUT_MS` ab; die Meldung wird gesprochen.
 */
function makeModel(
  info: ModelDescription,
  stufe: Souveraenitaet,
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
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        cancel();
      }, TURN_TIMEOUT_MS);
      if (req.signal.aborted) cancel();
      req.signal.addEventListener("abort", cancel, { once: true });
      try {
        const text = await invoke<string>("voice_call_turn", {
          requestId: turnId,
          messages: wireMessages(req.messages),
          modelId: info.id,
          stufe,
        });
        // Auch nach Abbruch noch kurz auf den Verbrauch warten: er zählt trotzdem.
        await Promise.race([
          usageP,
          new Promise<void>((r) => setTimeout(r, USAGE_WAIT_MS)),
        ]);
        if (timedOut && !req.signal.aborted) {
          req.onText(TURN_TIMEOUT_TEXT);
          return { text: TURN_TIMEOUT_TEXT, toolCalls: [] };
        }
        onText = () => undefined;
        return { text, toolCalls: [] };
      } finally {
        clearTimeout(timer);
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

async function listTools(): Promise<VoiceTool[]> {
  try {
    const list = await invoke<VoiceToolInfo[] | null>("voice_tools_list");
    return toolsFrom(Array.isArray(list) ? list : []);
  } catch {
    return [];
  }
}

interface LoadedModels {
  active: ModelDescription | null;
  all: ModelDescription[];
  /** Freigegebene Wissensbereiche (Einstellungen), gelesen mit den Modellen. */
  knowledge: KnowledgeState;
}

async function loadModels(): Promise<LoadedModels> {
  const [settings, compliance] = await Promise.all([
    commands.getAppSettings().catch(() => null),
    commands.complianceAssessModels().catch(() => null),
  ]);
  if (!settings || settings.status !== "ok")
    return { active: null, all: [], knowledge: NO_KNOWLEDGE };
  const all = describeModels(settings.data, assessedById(compliance));
  const activeId = settings.data.llm_active_model_id ?? null;
  return {
    active: all.find((m) => m.id === activeId) ?? null,
    all,
    knowledge: knowledgeFrom(settings.data),
  };
}

/** Fehlertext eines Werkzeugs für das Modell: kurz, ohne Steuerzeichen. */
function errorText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  return raw.replace(/[\u0000-\u001f]/g, " ").slice(0, 500);
}

/**
 * Ruft ein Werkzeug der Brücke. Ein Fehler oder Zeitablauf kommt als Text-Ergebnis zurück
 * (das Modell spricht ihn aus), nie als Ausnahme.
 */
async function runTool(
  tool: VoiceTool,
  args: Record<string, unknown>,
  conversationId: string | null,
): Promise<VoiceToolResult> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<VoiceToolResult>((resolve) => {
    timer = setTimeout(
      () => resolve({ text: TOOL_TIMEOUT_TEXT, bereich: tool.bereich }),
      TOOL_TIMEOUT_MS,
    );
  });
  const call = invoke<string>("voice_tool_call", {
    name: tool.name,
    args: JSON.stringify(args),
    conversationId,
  }).then(
    (text): VoiceToolResult => ({ text, bereich: tool.bereich }),
    (e: unknown): VoiceToolResult => ({
      text: errorText(e),
      bereich: tool.bereich,
    }),
  );
  try {
    return await Promise.race([call, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Der Orchestrator eines Gesprächs (eine je Engine). Hält Werkzeugliste, Modelle und den
 * gebauten Orchestrator zwischen den Schritten (je Gespräch statt je Satz); `invalidate`
 * verwirft alles (Einstellungen oder Gespräch gewechselt), `dispose` bricht Laufendes ab.
 */
export class OrchestrationSession {
  private tools: VoiceTool[] | null = null;
  private models: LoadedModels | null = null;
  private side: ChatSide | null = null;
  private sideKey = "";

  constructor(private readonly deps: OrchestrationDeps) {}

  /** Orchestrator beenden, Zwischenspeicher behalten (Stopp). */
  dispose(): void {
    this.side?.dispose?.();
    this.side = null;
    this.sideKey = "";
  }

  /** Alles verwerfen: nächster Schritt liest Einstellungen, Modelle und Werkzeuge neu. */
  invalidate(): void {
    this.dispose();
    this.tools = null;
    this.models = null;
  }

  /**
   * Bereitet den nächsten Schritt vor. Die Prüfung des Sprechers gilt immer, auch ohne Denker und
   * Werkzeuge: ist er bei der Souveränität gesperrt, gibt es kein Gespräch mit ihm (`blocked`).
   */
  async prepare(
    cfg: OrchestrationConfig & { systemPrompt: string },
  ): Promise<Prepared> {
    const deps = this.deps;
    this.models ??= await loadModels();
    const { active, all, knowledge } = this.models;
    const systemPrompt = withKnowledgeHint(cfg.systemPrompt, knowledge);
    if (!active) return { side: null, blocked: false, systemPrompt };
    if (blockedBy(active, cfg.souveraenitaet)) {
      deps.onStatus({ kind: "notice", code: "speaker_blocked" });
      return { side: null, blocked: true, systemPrompt };
    }
    if (cfg.useTools) this.tools ??= await listTools();
    const offered = cfg.useTools ? (this.tools ?? []) : [];
    const denkerInfo =
      cfg.denkerModelId !== ""
        ? all.find((m) => m.id === cfg.denkerModelId)
        : undefined;
    if (denkerInfo && blockedBy(denkerInfo, cfg.souveraenitaet)) {
      deps.onStatus({ kind: "notice", code: "denker_blocked" });
    }
    const denker =
      denkerInfo && !blockedBy(denkerInfo, cfg.souveraenitaet)
        ? denkerInfo
        : undefined;
    if (offered.length === 0 && !denker)
      return { side: null, blocked: false, systemPrompt };

    const key = JSON.stringify([
      cfg,
      systemPrompt,
      active.id,
      denker?.id ?? null,
      offered.map((t) => t.name),
    ]);
    if (this.side && key === this.sideKey)
      return { side: this.side, blocked: false, systemPrompt };
    this.dispose();
    const names = new Set(offered.map((t) => t.name));
    const stufe = cfg.souveraenitaet;
    this.side = createOrchestrator({
      sprecher: makeModel(active, stufe, deps),
      denker: denker ? makeModel(denker, stufe, deps) : undefined,
      tools: offered,
      bereich: SESSION_BEREICH,
      souveraenitaet: stufe,
      systemPrompt,
      authorize: async (tool, _args, bereich) =>
        bereich === SESSION_BEREICH &&
        tool.bereich === bereich &&
        names.has(tool.name) &&
        !blockedBy(active, stufe),
      run: async (tool, args, signal) => {
        if (!signal?.aborted) deps.onStatus({ kind: "tool", tool: tool.name });
        try {
          const id = (await deps.conversationId?.()) ?? null;
          return await runTool(tool, args, id);
        } finally {
          deps.onStatus(null);
        }
      },
    });
    this.sideKey = key;
    return { side: this.side, blocked: false, systemPrompt };
  }
}

export type { ChatMessage };
