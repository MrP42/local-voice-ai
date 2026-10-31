import type { CallUsageEvent } from "@/components/voice-call/engine";

/**
 * Wire-Typen und reine Hilfen der gespeicherten Sprachgespräche (Paket U3).
 * Feldnamen wie im Unterhaltungs-Vertrag der Suite (snake_case); die Befehle
 * liegen im Kern unter `src-tauri/src/conversations/`.
 */

/** Kennung der Person, die das Gespräch führt (lokal immer der Besitzer). */
export const LOCAL_USER_ID = "local";
/** Kennung der KI-Stimme als Teilnehmer. */
export const VOICEBOT_ID = "voicebot";
/** Mehr Text nimmt der Speicher je Beitrag nicht an. */
export const MAX_ENTRY_CHARS = 200_000;
/** Titel aus der ersten Nutzernachricht: so lang höchstens. */
export const TITLE_MAX_CHARS = 60;

/** Schlüssel in `ext` für alles, was nicht zum Vertrag gehört (Verbrauch). */
export const EXT_KEY = "local-voice-ai";

export interface ConversationSummary {
  id: string;
  title: string;
  updated_at: string;
  pinned: boolean;
  archived: boolean;
  entry_count: number;
  last_text: string;
}

export interface VoiceExt {
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  usage_event_ids?: number[];
  provider_id?: string | null;
  local?: boolean | null;
}

export interface EntryExt {
  [EXT_KEY]?: VoiceExt | null;
}

export interface StoredEntry {
  id: string;
  conversation_id: string;
  role: string;
  mode: string;
  text: string;
  status: string;
  created_at: string;
  ext?: EntryExt | null;
}

export interface ConversationRecord {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  pinned: boolean;
  archived: boolean;
}

export interface ConversationDetail {
  conversation: ConversationRecord;
  entries: StoredEntry[];
}

/** Anhang eines Beitrags (Vertrag `Attachment`); der Dateiinhalt wird nie gespeichert. */
export interface HistoryAttachment {
  id: string;
  name: string;
  mime: string;
  size_bytes?: number;
}

const MIME_BY_EXTENSION: Record<string, string> = {
  pdf: "application/pdf",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  txt: "text/plain",
  md: "text/markdown",
};

/** MIME-Typ aus der Dateiendung; sonst `application/octet-stream`. */
export function mimeForName(name: string): string {
  const dot = name.lastIndexOf(".");
  const ext = dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
  return MIME_BY_EXTENSION[ext] ?? "application/octet-stream";
}

export interface NewEntry {
  role: "user" | "assistant";
  mode: "typed" | "spoken";
  text: string;
  status: "complete" | "interrupted";
  created_at: string;
  author_id: string;
  triggered_by?: string;
  model?: string;
  attachments?: HistoryAttachment[];
  ext?: EntryExt;
}

/**
 * Titel aus der ersten Nutzernachricht: einzeilig, höchstens
 * `TITLE_MAX_CHARS` Zeichen, an der Wortgrenze gekürzt (mit „…“).
 * `null`, wenn kein Text da ist.
 */
export function deriveTitle(text: string): string | null {
  const flat = text.replace(/\s+/g, " ").trim();
  if (!flat) return null;
  const chars = Array.from(flat);
  if (chars.length <= TITLE_MAX_CHARS) return flat;
  const room = chars.slice(0, TITLE_MAX_CHARS - 1);
  const lastSpace = room.lastIndexOf(" ");
  const cut = lastSpace > 0 ? room.slice(0, lastSpace) : room;
  return `${cut.join("").replace(/[\s.,;:!?-]+$/, "")}…`;
}

/** Zwei Verbrauchsmeldungen zu einer (mehrere Aufrufe, eine Antwort). */
export function mergeUsage(
  a: CallUsageEvent | null,
  b: CallUsageEvent,
): CallUsageEvent {
  if (!a) return b;
  const ids = (u: CallUsageEvent): number[] =>
    u.usage_event_ids ?? (u.usage_event_id != null ? [u.usage_event_id] : []);
  return {
    prompt_tokens: a.prompt_tokens + b.prompt_tokens,
    completion_tokens: a.completion_tokens + b.completion_tokens,
    usage_event_ids: [...ids(a), ...ids(b)],
    provider_id: b.provider_id ?? a.provider_id,
    local: b.local ?? a.local,
  };
}

/** Verbrauch einer Antwort als `ext` des Beitrags (nur lokal, nie im Vertrag). */
export function usageToExt(u: CallUsageEvent): EntryExt {
  const ids =
    u.usage_event_ids ?? (u.usage_event_id != null ? [u.usage_event_id] : []);
  const ext: VoiceExt = {
    prompt_tokens: u.prompt_tokens,
    completion_tokens: u.completion_tokens,
    usage_event_ids: ids,
  };
  if (u.provider_id) ext.provider_id = u.provider_id;
  if (u.local != null) ext.local = u.local;
  return { [EXT_KEY]: ext };
}

/** Gespeicherter Verbrauch eines Beitrags, wieder als Meldung; `null` ohne Verbrauch. */
export function usageFromExt(
  ext: EntryExt | null | undefined,
): CallUsageEvent | null {
  const v = ext?.[EXT_KEY];
  if (!v) return null;
  return {
    prompt_tokens: v.prompt_tokens ?? 0,
    completion_tokens: v.completion_tokens ?? 0,
    usage_event_ids: v.usage_event_ids ?? [],
    provider_id: v.provider_id ?? null,
    local: v.local ?? undefined,
  };
}
