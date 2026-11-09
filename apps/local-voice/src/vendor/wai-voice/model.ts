// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
import type { ObjectSchema } from "./schema.ts";

/** Souveränitätsstufe des Chats bzw. Bereichs (ADR-0021, 4.). */
export type Souveraenitaet = "nur_lokal" | "eu" | "frei";

/** Werkzeug, wie es einem Modell angeboten wird. */
export interface ToolSpec {
  name: string;
  description: string;
  parameters: ObjectSchema;
}

export interface ModelToolCall {
  id: string;
  name: string;
  args: unknown;
}

export type ModelMessage =
  | { role: "system" | "user"; content: string }
  | { role: "assistant"; content: string; tool_calls?: ModelToolCall[] }
  | { role: "tool"; tool_call_id: string; content: string };

export interface ModelTurnRequest {
  requestId: string;
  messages: ModelMessage[];
  /** Nur bei Modellen mit `supportsTools`; sonst führt der Orchestrator das Textprotokoll. */
  tools?: ToolSpec[];
  signal: AbortSignal;
  /** Textstrom des Modells, in beliebig kleinen Stücken. */
  onText(delta: string): void;
}

export interface ModelTurnResult {
  text: string;
  toolCalls: ModelToolCall[];
}

/** Ein Sprach- oder Denkermodell, das der Host (oder ein Adapter) liefert. */
export interface VoiceModel {
  id: string;
  label: string;
  local: boolean;
  jurisdiction?: "eu" | "other";
  supportsTools: boolean;
  turn(req: ModelTurnRequest): Promise<ModelTurnResult>;
}

/** Ob die Souveränitätsstufe das Modell zulässt (Cloud-Modelle nur bei `eu` mit EU-Sitz oder `frei`). */
export function modelAllowed(model: VoiceModel, stufe: Souveraenitaet): boolean {
  if (stufe === "frei") return true;
  if (model.local) return true;
  return stufe === "eu" && model.jurisdiction === "eu";
}
