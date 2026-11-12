import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import {
  DEFAULT_CALL_CONFIG,
  DEFAULT_SYSTEM_PROMPT,
  START_ABORTED,
  VoiceCallEngine,
  type CallConfig,
  type CallReady,
  type CallState,
  type CallUsageEvent,
  type TurnMetrics,
} from "@/components/voice-call/engine";
import { sendFeedback } from "@/components/voice-call/speechLearning";
import { SUBSCRIPTION_PROVIDERS } from "@/components/settings/meetings/DocUsageLine";
import { DEFAULT_TURN_CONFIG } from "@/components/voice-call/turnTaking";
import type { Jump } from "@/components/voice-call/globalContext";
import {
  DEFAULT_ORCHESTRATION,
  SOUVERAENITAET_STUFEN,
  type CallStatus,
} from "@/components/voice-call/orchestration";
import {
  zusammen,
  type ProjektBindung,
  type ProjektHinweis,
} from "@/components/voice-call/projekt";
import {
  LOCAL_USER_ID,
  MAX_ENTRY_CHARS,
  VOICEBOT_ID,
  deriveTitle,
  mergeUsage,
  usageFromExt,
  usageToExt,
  type ConversationDetail,
  type ConversationRecord,
  type ConversationSummary,
  type HistoryAttachment,
  type NewEntry,
  type StoredEntry,
} from "@/stores/voiceHistory";

/** Schlüssel der gespeicherten Gesprächseinstellungen (unverändert seit Stufe 0). */
export const VOICE_CALL_CONFIG_KEY = "voice-call.config";

/** Schlüssel des Hauptgesprächs (Kennung des gespeicherten Gesprächs). */
export const MAIN_CONVERSATION_KEY = "voice-call.main";

/**
 * Kontext des Hauptgesprächs: höchstens so viele Nachrichten (vier Wechsel) gehen an die KI.
 * Der Textverlauf bleibt vollständig; nur der Kontext der KI bleibt klein.
 */
export const MAIN_HISTORY_LIMIT = 8;

/** Nach so langer Pause beginnt das Hauptgespräch ohne alten Kontext (Verlauf bleibt). */
export const MAIN_FRESH_AFTER_MS = 30 * 60 * 1000;

function loadMainConversationId(): string | null {
  try {
    const id = window.localStorage.getItem(MAIN_CONVERSATION_KEY);
    return id && id.trim() ? id : null;
  } catch {
    return null;
  }
}

function saveMainConversationId(id: string | null): void {
  try {
    if (id) window.localStorage.setItem(MAIN_CONVERSATION_KEY, id);
    else window.localStorage.removeItem(MAIN_CONVERSATION_KEY);
  } catch {
    /* ohne Speicher beginnt das nächste Hauptgespräch neu */
  }
}

/**
 * Kontext der KI fürs Hauptgespräch: nur Zeilen der letzten `freshAfterMs`, davon die letzten
 * `limit`. Nach einer langen Pause ist er leer: keine veralteten Informationen, keine Token
 * für Altes. Rein, damit es prüfbar ist.
 */
export function compactMainHistory(
  lines: Pick<VoiceLine, "role" | "text" | "at" | "turnId" | "final">[],
  now: number,
  limit = MAIN_HISTORY_LIMIT,
  freshAfterMs = MAIN_FRESH_AFTER_MS,
): { role: "user" | "assistant"; content: string; turnId?: string }[] {
  const last = lines[lines.length - 1];
  if (!last || now - last.at > freshAfterMs) return [];
  return lines
    .filter(
      (l) => l.final !== false && l.text.trim() && now - l.at <= freshAfterMs,
    )
    .slice(-limit)
    .map((l) => ({ role: l.role, content: l.text, turnId: l.turnId }));
}

export type VoiceSettingsTab =
  "talk" | "voice" | "recognition" | "knowledge" | "models";

/** Ein geprüfter Beleg einer Antwort (Anzeige, nicht gesprochen). */
export interface VoiceSource {
  k: number;
  kind: string;
  title: string;
  page: number | null;
  jump: Jump;
}

export interface VoiceLine {
  id: number;
  role: "user" | "assistant";
  text: string;
  final?: boolean;
  interrupted?: boolean;
  /** Uhrzeit der Zeile (ms seit Epoche). */
  at: number;
  /** Zeit bis zum ersten Ton (ms), nur bei gesprochenen Antworten. */
  latencyMs?: number;
  /** Geprüfte Belege des globalen Wissens (lva#325), nur bei Antworten. */
  sources?: VoiceSource[];
  /** Gesprochen (Anruf) oder getippt; so steht es im gespeicherten Beitrag. */
  mode?: "typed" | "spoken";
  /** Kennung der Äußerung in der Engine (für spätere Korrekturen im Verlauf der KI). */
  turnId?: string;
  /** Der Text, wie ihn die Spracherkennung geliefert hat; bleibt bei Korrekturen stehen. */
  recognized?: string;
  /** Schon im Verlauf gespeichert (oder zum Speichern eingereiht). */
  persisted?: boolean;
  /** Dateien, die der Nutzer mitgeschickt hat (nur Name, Typ, Größe; kein Inhalt). */
  attachments?: HistoryAttachment[];
  /** Denkprozess vor dieser Antwort (nur in der laufenden Sitzung, nicht gespeichert). */
  denken?: Denkprozess;
}

/**
 * Ein Schritt der Denk-Anzeige: Ansage oder Überbrückung (Text, schon in der Sprache der App),
 * ein Werkzeug (Name) oder der Abruf eigener Inhalte (Zahl der Treffer).
 */
export type DenkSchritt =
  | { art: "text"; text: string }
  | { art: "sofort"; modell: string }
  | { art: "werkzeug"; name: string }
  | { art: "kontext"; treffer: number };

/** Denkprozess einer Antwort (nur Anzeige: nie gesprochen, nie gespeichert, nie an ein Modell). */
export interface Denkprozess {
  /** Äußerung, zu der der Prozess gehört; Schritte eines anderen Schritts werden verworfen. */
  turnId?: string;
  /** Beginn (ms seit Epoche). */
  seit: number;
  /** Dauer bis zur ersten Antwort (ms); fehlt, solange gedacht wird. */
  ms?: number;
  /** Der nächste Text ist der Sofort-Sprecher, der weiterspricht: die Anzeige endet damit nicht. */
  zwischen?: boolean;
  /** Der Sofort-Sprecher hat weitergesprochen; die Anzeige läuft unter seiner Zeile, bis der Denker liefert. */
  weiter?: boolean;
  schritte: DenkSchritt[];
  /** Gestreamter Denktext des Modells (Reasoning), höchstens `DENKTEXT_MAX` Zeichen. */
  gedanken?: string;
  /** Modell, das gedacht hat (bleibt nach der Antwort sichtbar). */
  modell?: string;
  /** Aufgeklappt: geht beim Übergang von der laufenden Anzeige an die Antwort mit. */
  offen?: boolean;
}

/** Höchstlänge des angezeigten Denktexts (Zeichen). */
export const DENKTEXT_MAX = 16_000;

/** Hängt Denktext an (bis `DENKTEXT_MAX`). */
export function denkText(d: Denkprozess, text: string): Denkprozess {
  const bisher = d.gedanken ?? "";
  if (bisher.length >= DENKTEXT_MAX) return d;
  return { ...d, gedanken: (bisher + text).slice(0, DENKTEXT_MAX) };
}

/** Hängt einen Schritt an; derselbe Schritt direkt hintereinander zählt einmal. */
export function denkSchritt(d: Denkprozess, s: DenkSchritt): Denkprozess {
  const last = d.schritte[d.schritte.length - 1];
  if (last && JSON.stringify(last) === JSON.stringify(s)) return d;
  return { ...d, schritte: [...d.schritte, s] };
}

/** Verbrauch des Gesprächs seit „Neues Gespräch“. */
export interface CallUsage {
  promptTokens: number;
  completionTokens: number;
  /** Alle Ledger-Ereignisse des Gesprächs. */
  usageEventIds: number[];
  /** Nur die Ereignisse von API-Aufrufen (Abo und lokal kosten nichts extra). */
  billableEventIds: number[];
  /** Anzahl Antworten je Abrechnungsart. */
  calls: { subscription: number; local: number; api: number };
  providerId: string | null;
  local: boolean;
  /** Mindestens ein Aufruf lief nicht lokal (Cloud oder Abo): „lokal“ zeigt die Infozeile dann nicht. */
  anyCloud: boolean;
}

const EMPTY_USAGE: CallUsage = {
  promptTokens: 0,
  completionTokens: 0,
  usageEventIds: [],
  billableEventIds: [],
  calls: { subscription: 0, local: 0, api: 0 },
  providerId: null,
  local: false,
  anyCloud: false,
};

/** Mikrofon offen (Anruf läuft oder startet gerade)? */
const callRunning = (state: CallState): boolean => state !== "idle";

/** Verbrauch einer weiteren Antwort zum Gesprächsverbrauch addieren. */
function addUsage(usage: CallUsage, u: CallUsageEvent): CallUsage {
  const ids =
    u.usage_event_ids ?? (u.usage_event_id != null ? [u.usage_event_id] : []);
  const subscription = SUBSCRIPTION_PROVIDERS.has(u.provider_id ?? "");
  const local = !subscription && u.local === true;
  const api = !subscription && !local;
  return {
    promptTokens: usage.promptTokens + (u.prompt_tokens ?? 0),
    completionTokens: usage.completionTokens + (u.completion_tokens ?? 0),
    usageEventIds: [...usage.usageEventIds, ...ids],
    billableEventIds: api
      ? [...usage.billableEventIds, ...ids]
      : usage.billableEventIds,
    calls: {
      subscription: usage.calls.subscription + (subscription ? 1 : 0),
      local: usage.calls.local + (local ? 1 : 0),
      api: usage.calls.api + (api ? 1 : 0),
    },
    providerId: u.provider_id ?? usage.providerId,
    local: u.local ?? usage.local,
    anyCloud: usage.anyCloud || u.local !== true,
  };
}

/** Gesprächsverbrauch aus den gespeicherten Antworten (Infozeile beim Öffnen). */
export function usageFromEntries(entries: StoredEntry[]): CallUsage {
  return entries.reduce((usage, e) => {
    const u = e.role === "assistant" ? usageFromExt(e.ext) : null;
    return u ? addUsage(usage, u) : usage;
  }, EMPTY_USAGE);
}

function lineFromEntry(e: StoredEntry, id: number): VoiceLine {
  return {
    id,
    role: e.role === "user" ? "user" : "assistant",
    text: e.text,
    final: true,
    interrupted: e.status === "interrupted",
    at: Date.parse(e.created_at) || Date.now(),
    mode: e.mode === "spoken" ? "spoken" : "typed",
    persisted: true,
    turnId: `entry-${e.id}`,
    recognized: e.role === "user" ? e.text : undefined,
  };
}

/**
 * Darf die Zeile jetzt bearbeitet werden? Nur die eigene gesprochene Zeile, und nur wenn weder
 * eine Antwort aussteht noch gerade gesprochen wird: sonst gehört der Text noch zu einem
 * laufenden Schritt der Engine.
 */
export function lineEditable(
  line: VoiceLine,
  state: CallState,
  textBusy: boolean,
): boolean {
  return (
    line.role === "user" &&
    line.mode === "spoken" &&
    !!line.turnId &&
    !textBusy &&
    (state === "idle" || state === "listening")
  );
}

function entryFromLine(
  line: VoiceLine,
  usage: CallUsageEvent | null,
  model: string | null,
): NewEntry | null {
  const text = line.text.slice(0, MAX_ENTRY_CHARS);
  if (!text.trim()) return null;
  const base: NewEntry = {
    role: line.role,
    mode: line.mode ?? "typed",
    text,
    status: line.interrupted ? "interrupted" : "complete",
    created_at: new Date(line.at).toISOString(),
    author_id: line.role === "user" ? LOCAL_USER_ID : VOICEBOT_ID,
  };
  if (line.role === "user")
    return line.attachments?.length
      ? { ...base, attachments: line.attachments }
      : base;
  return {
    ...base,
    triggered_by: LOCAL_USER_ID,
    ...(model ? { model: model.slice(0, 256) } : {}),
    ...(usage ? { ext: usageToExt(usage) } : {}),
  };
}

/** Gespeicherte Orchestrator-Einstellungen mit Vorgaben; Unbekanntes fällt auf die sichere Vorgabe. */
function normalizeOrchestration(
  stored: Partial<CallConfig["orchestration"]> | undefined,
): CallConfig["orchestration"] {
  const stufe = SOUVERAENITAET_STUFEN.find((s) => s === stored?.souveraenitaet);
  return {
    denkerModelId:
      typeof stored?.denkerModelId === "string" ? stored.denkerModelId : "",
    souveraenitaet: stufe ?? DEFAULT_ORCHESTRATION.souveraenitaet,
    useTools:
      typeof stored?.useTools === "boolean"
        ? stored.useTools
        : DEFAULT_ORCHESTRATION.useTools,
  };
}

export function loadCallConfig(): CallConfig {
  try {
    const raw = window.localStorage.getItem(VOICE_CALL_CONFIG_KEY);
    if (!raw) return DEFAULT_CALL_CONFIG;
    const parsed = JSON.parse(raw) as Partial<CallConfig>;
    return {
      systemPrompt: parsed.systemPrompt?.trim() || DEFAULT_SYSTEM_PROMPT,
      turn: { ...DEFAULT_TURN_CONFIG, ...parsed.turn },
      orchestration: normalizeOrchestration(parsed.orchestration),
    };
  } catch {
    return DEFAULT_CALL_CONFIG;
  }
}

function saveCallConfig(config: CallConfig): void {
  try {
    window.localStorage.setItem(VOICE_CALL_CONFIG_KEY, JSON.stringify(config));
  } catch {
    /* nicht speichern ist verkraftbar */
  }
}

interface VoiceCallStore {
  config: CallConfig;
  /** Die Seite Sprachgespräch ist sichtbar (die Fußleiste zeigt dann „Gespräch“). */
  active: boolean;
  state: CallState;
  ready: CallReady | null;
  error: string | null;
  /** Werkzeug, das der Orchestrator gerade aufruft (Name), für die Statuszeile. */
  toolStatus: string | null;
  /** Hinweis des Orchestrators (z. B. Cloud-Modell bei „nur lokal“ gesperrt). */
  notice: Extract<CallStatus, { kind: "notice" }>["code"] | null;
  /** Laufender Denkprozess bis zur ersten Antwort (graue Denk-Anzeige unter der letzten Nachricht). */
  denken: Denkprozess | null;
  metrics: TurnMetrics[];
  lines: VoiceLine[];
  usage: CallUsage;
  muted: boolean;
  /** Text-Antwort ohne Anruf wird gerade geholt. */
  textBusy: boolean;
  settingsOpen: boolean;
  settingsTab: VoiceSettingsTab;
  /** Gespeichertes Gespräch, das gerade offen ist; `null` bis zum ersten Beitrag. */
  conversationId: string | null;
  /**
   * Projekt des offenen Gesprächs (`scope.project_id` des gespeicherten Gesprächs); `null` in einem
   * gewöhnlichen Gespräch. Gesetzt nur durch `newProjectConversation` und beim Öffnen.
   */
  projectId: string | null;
  /** Stand der Auswahl im Projekt, den die Person gesehen hat (geht als `expectedStand` an das Tor). */
  projektStand: string | null;
  /** Hinweis aus dem Projektweg (Budgetstopp, Auswahl geändert, Verlauf zurückgesetzt …). */
  projektHinweis: ProjektHinweis | null;
  conversations: ConversationSummary[];
  /** Archivierte Gespräche in der Liste zeigen. */
  showArchived: boolean;
  /** Ein anderes Gespräch wird gerade geladen: Eingabe gesperrt. */
  switching: boolean;
  /** Letzter Fehler beim Speichern im Verlauf (getrennt von Gesprächsfehlern). */
  saveError: string | null;
  /**
   * Das feste Hauptgespräch: Tastenkürzel und Telefon in der Fußleiste sprechen immer hierhin.
   * `null` bis zum ersten Beitrag eines neuen Hauptgesprächs.
   */
  mainConversationId: string | null;
  /** Der laufende Anruf gehört zum Hauptgespräch (Tastenkürzel/Fußleiste): er überlebt das Verlassen der Seite. */
  mainCall: boolean;

  setActive: (active: boolean) => void;
  setConfig: (update: (c: CallConfig) => CallConfig) => void;
  resetConfig: () => void;
  setMuted: (muted: boolean) => void;
  openSettings: (tab?: VoiceSettingsTab) => void;
  closeSettings: () => void;
  setSettingsTab: (tab: VoiceSettingsTab) => void;
  /** Wissensbereiche geändert: das Gespräch lädt Werkzeuge und Hinweis beim nächsten Schritt neu. */
  knowledgeChanged: () => void;
  startCall: () => Promise<void>;
  stopCall: () => Promise<void>;
  /** `display`: was im Verlauf steht, wenn es vom gesendeten Text abweicht (Anhänge). */
  sendText: (
    text: string,
    display?: string,
    attachments?: HistoryAttachment[],
  ) => Promise<void>;
  /**
   * Eigene erkannte Zeile verbessern: Anzeige, Verlauf (gespeicherter Beitrag, Kontext der KI)
   * und, nur bei eingeschaltetem Lernen, die Rückmeldung an die Spracherkennung.
   */
  editLine: (lineId: number, text: string) => Promise<void>;
  /** Leeres Gespräch; gespeichert wird erst beim ersten Beitrag. */
  newConversation: () => Promise<void>;
  /**
   * Leeres Gespräch im Projekt. Es wird sofort angelegt, damit Rust die Bindung aus dem Speicher
   * kennt, bevor die erste Frage geht. `stand`: Stand der Auswahl, den die Person gerade sieht.
   */
  newProjectConversation: (
    projectId: string,
    stand: string | null,
  ) => Promise<void>;
  /** Die Person hat die aktuelle Auswahl gesehen (nach „Auswahl geändert“): der Hinweis verschwindet. */
  setProjektStand: (stand: string | null) => void;
  dismissProjektHinweis: () => void;
  loadConversations: () => Promise<void>;
  /** Gespeichertes Gespräch öffnen (beendet einen laufenden Anruf). */
  openConversation: (id: string) => Promise<void>;
  rename: (id: string, title: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  togglePin: (id: string) => Promise<void>;
  archive: (id: string, archived?: boolean) => Promise<void>;
  setShowArchived: (show: boolean) => Promise<void>;
  /** Laufende Text-Antwort abbrechen (Stopp-Knopf). */
  stopText: () => void;
  /** Denkprozess (laufend oder an einer Antwort, erkannt an `seit`) auf- oder zuklappen. */
  denkOffen: (seit: number, offen: boolean) => void;
  /** Modell des laufenden Denkprozesses festhalten (bleibt an der Antwort). */
  denkModell: (modell: string) => void;
  /** Seite verlassen: Anruf beenden, Verlauf bleibt. */
  leave: () => Promise<void>;
  /**
   * Tastenkürzel bzw. Telefon in der Fußleiste: läuft ein Anruf im Hauptgespräch, endet er;
   * sonst wechselt es ins Hauptgespräch (ein anderer Anruf endet vorher) und startet dort.
   * Nie in ein anderes Gespräch.
   */
  toggleMainCall: () => Promise<void>;
  /** Neues Hauptgespräch: das nächste Tastenkürzel beginnt ein frisches Gespräch. */
  newMainConversation: () => Promise<void>;
}

/** Pegel des Mikrofons je Bild für die Wellenanzeige (ohne Zustandsänderung). */
const levelListeners = new Set<(level: number) => void>();

export function subscribeCallLevel(fn: (level: number) => void): () => void {
  levelListeners.add(fn);
  return () => levelListeners.delete(fn);
}

let engine: VoiceCallEngine | null = null;
let lineId = 0;
/** Zählt Text-Anfragen, damit nur die jüngste `textBusy` zurücksetzt. */
let textRun = 0;
/** `toggleMainCall` läuft gerade (Tastenkürzel oder Telefon). */
let toggling = false;
/** Zählt Besuche der Seite: ein spätes `leave()` (StrictMode, schneller Rückweg) darf einen neuen Besuch nicht abmelden. */
let visit = 0;
/**
 * Ziel der Beiträge: das Gespräch, dem neue Zeilen gehören. Eingereihte Speicher-
 * aufträge halten ihr Ziel fest, damit ein Wechsel sie nicht ins falsche Gespräch
 * schreibt; `id` bleibt `null`, bis der erste Beitrag das Gespräch anlegt.
 */
interface Target {
  id: string | null;
  /** Das Gespräch trägt schon einen Titel aus der ersten Nutzernachricht (oder einen eigenen). */
  titled: boolean;
  /** Das Hauptgespräch: beim Anlegen wird seine Kennung gemerkt, der KI-Kontext bleibt kompakt. */
  main?: boolean;
}
/** Titel, mit dem der Kern leere Gespräche anlegt. */
const DEFAULT_TITLE = "Neues Gespräch";
let target: Target = { id: null, titled: false };
/** Speicheraufträge laufen nacheinander: Reihenfolge und „erst anlegen, dann anhängen“. */
let persistChain: Promise<void> = Promise.resolve();
/** Verbrauch der laufenden Antwort, bis ihr Beitrag gespeichert wird. */
let pendingUsage: CallUsageEvent | null = null;
/** Die nächste Nutzerzeile ist getippt (sonst gesprochen). */
let typedNext = false;
/** Dateien der nächsten Nutzerzeile (wie `typedNext`). */
let attachmentsNext: HistoryAttachment[] | undefined;
/** Kennung des gespeicherten Beitrags je Zeile, für spätere Änderungen (Unterbrechen). */
const entryIds = new Map<number, string>();
/** Zählt Wechsel des Gesprächs: ein spätes Laden eines alten Wechsels wird verworfen. */
let openRun = 0;
/** Zählt Listenabfragen: nur die jüngste Antwort darf die Liste setzen. */
let listRun = 0;
/** Zeilen, deren Speichern gerade läuft (kein zweites Einreihen). */
const inflight = new Set<number>();
/** Verbrauch je Zeile, damit ein erneuter Versuch ihn nicht verliert. */
const lineUsage = new Map<number, CallUsageEvent | null>();

export const useVoiceCallStore = create<VoiceCallStore>()((set, get) => {
  const getEngine = (): VoiceCallEngine => {
    if (engine) return engine;
    engine = new VoiceCallEngine(
      {
        // Endet der Anruf (auch durch einen Fehler), gehört kein Anruf mehr zum Hauptgespräch.
        onState: (state) => {
          if (state !== "idle") {
            set({ state });
            return;
          }
          // Endet ein Hauptgespräch (auch durch einen Fehler), die letzte Zeile mitspeichern.
          const wasMain = get().mainCall;
          set({ state, mainCall: false, denken: null });
          if (wasMain) flushLines(true);
        },
        // Der Pegel läuft am Store vorbei: 50 Bilder je Sekunde gehören nicht in den Zustand.
        onLevel: (level) => levelListeners.forEach((fn) => fn(level)),
        onUser: (text, turnId) => {
          const mode = typedNext ? "typed" : "spoken";
          const attachments = attachmentsNext;
          typedNext = false;
          attachmentsNext = undefined;
          set({ notice: null, toolStatus: null, projektHinweis: null });
          set((s) => {
            // Verworfene spekulative Vorarbeit (Weitersprechen) ersetzen.
            const kept = [...s.lines];
            while (
              kept.length &&
              kept[kept.length - 1].role === "assistant" &&
              !kept[kept.length - 1].final
            )
              kept.pop();
            if (kept.length && kept[kept.length - 1].role === "user")
              kept.pop();
            return {
              lines: [
                ...kept,
                {
                  id: lineId++,
                  role: "user",
                  text,
                  at: Date.now(),
                  mode,
                  attachments,
                  turnId,
                  recognized: text,
                },
              ],
            };
          });
        },
        onAssistant: (text, final, interrupted) => {
          const last = get().lines[get().lines.length - 1];
          if (
            final &&
            interrupted &&
            last?.role === "assistant" &&
            last.final &&
            !last.interrupted
          ) {
            // Schon vollständig angezeigt und gespeichert, dann beim Sprechen
            // unterbrochen: dieselbe Sprechblase, derselbe Beitrag.
            set((s) => ({
              lines: s.lines.map((l) =>
                l.id === last.id ? { ...l, text, interrupted: true } : l,
              ),
            }));
            enqueueUpdate(last.id, text);
            return;
          }
          // Mit dem ersten Text endet die Denk-Anzeige; der Denkprozess hängt an der Antwort. Spricht
          // der Sofort-Sprecher nur weiter, läuft sie unter seiner Zeile weiter, bis der Denker liefert.
          const live = get().denken;
          if (live?.zwischen && !final && text.trim())
            set({ denken: { ...live, zwischen: false, weiter: true } });
          const denken =
            live && !(live.zwischen && !final) && (text.trim() || final)
              ? {
                  ...live,
                  zwischen: false,
                  weiter: false,
                  ms: Date.now() - live.seit,
                }
              : undefined;
          if (denken) set({ denken: null });
          set((s) => {
            const l = s.lines;
            const last = l[l.length - 1];
            if (last && last.role === "assistant" && !last.final) {
              return {
                lines: [
                  ...l.slice(0, -1),
                  {
                    ...last,
                    text,
                    final,
                    interrupted,
                    ...(denken ? { denken } : {}),
                  },
                ],
              };
            }
            return {
              lines: [
                ...l,
                {
                  id: lineId++,
                  role: "assistant",
                  text,
                  final,
                  interrupted,
                  at: Date.now(),
                  mode: callRunning(s.state) ? "spoken" : "typed",
                  ...(denken ? { denken } : {}),
                },
              ],
            };
          });
          if (final) flushLines();
        },
        onMetrics: (m) =>
          set((s) => {
            const lines = [...s.lines];
            for (let i = lines.length - 1; i >= 0; i--) {
              if (lines[i].role !== "assistant") continue;
              if (lines[i].latencyMs === undefined)
                lines[i] = { ...lines[i], latencyMs: m.firstSoundMs };
              break;
            }
            return { lines, metrics: [...s.metrics.slice(-19), m] };
          }),
        // Ein durch eine Ergänzung ersetzter Schritt hinterlässt keine Messung (lva#485).
        onMetricsVerwerfen: (turnId) =>
          set((s) => ({
            metrics: s.metrics.filter((m) => m.turnId !== turnId),
          })),
        onCitations: (turnId, hits) =>
          set((s) => {
            if (hits.length === 0) return {};
            const at = s.lines.findIndex(
              (l) => l.role === "user" && l.turnId === turnId,
            );
            if (at < 0) return {};
            const i = s.lines.findIndex(
              (l, j) => j > at && l.role === "assistant",
            );
            if (i < 0) return {};
            const lines = [...s.lines];
            lines[i] = {
              ...lines[i],
              sources: hits.map((h) => ({
                k: h.k,
                kind: h.kind,
                title: h.title,
                page: h.jump.page,
                jump: h.jump,
              })),
            };
            return { lines };
          }),
        onError: (error) => set({ error, denken: null }),
        projekt: (): ProjektBindung | null => {
          const id = get().projectId;
          return id ? { id, stand: get().projektStand } : null;
        },
        onProjekt: (h) =>
          set((s) => ({ projektHinweis: zusammen(s.projektHinweis, h) })),
        onVerwerfen: () =>
          set((s) => {
            const last = s.lines[s.lines.length - 1];
            if (!last || last.role !== "assistant" || last.final)
              return { denken: null };
            return { lines: s.lines.slice(0, -1), denken: null };
          }),
        conversationId: async () => {
          // Erst fertig speichern lassen: so kennt der Aufruf das laufende Gespräch,
          // und das Backend kann es ausnehmen (ohne Kennung sperrt es Gesprächswerkzeuge).
          await persistChain;
          return get().conversationId;
        },
        onStatus: (status) => {
          if (status === null) set({ toolStatus: null });
          else if (status.kind === "tool")
            set((s) => ({
              toolStatus: status.tool,
              denken: s.denken
                ? denkSchritt(s.denken, { art: "werkzeug", name: status.tool })
                : null,
            }));
          else set({ notice: status.code });
        },
        onDenken: (e) => {
          if (e.art === "start") {
            set({
              denken: { turnId: e.turnId, seit: Date.now(), schritte: [] },
            });
            return;
          }
          if (e.art === "zwischen") {
            set((s) =>
              s.denken ? { denken: { ...s.denken, zwischen: true } } : {},
            );
            return;
          }
          if (e.art === "ende") {
            set((s) => (s.denken?.turnId === e.turnId ? { denken: null } : {}));
            return;
          }
          if (e.art === "gedanke") {
            // Denktext läuft auch nach der ersten Antwortzeile weiter (Werkzeugrunden): dann an den
            // Denkprozess der letzten Antwort.
            set((s) => {
              if (s.denken) return { denken: denkText(s.denken, e.text) };
              const i = s.lines.length - 1;
              const last = s.lines[i];
              if (!last || last.role !== "assistant" || !last.denken) return {};
              const lines = [...s.lines];
              lines[i] = { ...last, denken: denkText(last.denken, e.text) };
              return { lines };
            });
            return;
          }
          const schritt: DenkSchritt =
            e.art === "sofort"
              ? { art: "sofort", modell: e.modell }
              : { art: "text", text: e.text };
          set((s) =>
            s.denken ? { denken: denkSchritt(s.denken, schritt) } : {},
          );
        },
        onContextTiming: (turnId, _retrieveMs, _promptMs, hits) =>
          set((s) =>
            s.denken && s.denken.turnId === turnId && hits > 0
              ? {
                  denken: denkSchritt(s.denken, {
                    art: "kontext",
                    treffer: hits,
                  }),
                }
              : {},
          ),
        onUsage: (u: CallUsageEvent) => {
          pendingUsage = mergeUsage(pendingUsage, u);
          set((s) => ({ usage: addUsage(s.usage, u) }));
        },
      },
      get().config,
    );
    engine.setMuted(get().muted);
    if (import.meta.env.DEV)
      (window as unknown as { __voiceCallEngine?: unknown }).__voiceCallEngine =
        engine;
    return engine;
  };

  const loadConversations = async (): Promise<void> => {
    const run = ++listRun;
    try {
      const list = await invoke<ConversationSummary[]>("conversations_list", {
        includeArchived: get().showArchived,
      });
      if (run !== listRun) return;
      set({ conversations: Array.isArray(list) ? list : [] });
    } catch {
      /* ohne Liste bleibt die alte stehen; das Gespräch selbst läuft weiter */
    }
  };

  /** Reiht das Anhängen eines Beitrags ein; legt das Gespräch beim ersten an. */
  const enqueueUpdate = (lineKey: number, text: string): void => {
    persistChain = persistChain.then(async () => {
      const id = entryIds.get(lineKey);
      // Ohne Kennung ist die Zeile noch nicht gespeichert: der nächste Versuch
      // nimmt ihren geänderten Text ohnehin mit.
      if (!id || !text.trim()) return;
      try {
        await invoke("conversation_update_entry", {
          id,
          text: text.slice(0, MAX_ENTRY_CHARS),
          status: "interrupted",
        });
        await loadConversations();
      } catch (e) {
        set({ saveError: String(e) });
      }
    });
  };

  /**
   * Reiht das Anhängen eines Beitrags ein; legt das Gespräch beim ersten an.
   * Als gespeichert gilt die Zeile erst nach Erfolg; bei einem Fehler bleibt sie
   * offen und der nächste Flush versucht es erneut.
   */
  const enqueueAppend = (entry: NewEntry, lineKey: number): void => {
    const t = target;
    inflight.add(lineKey);
    persistChain = persistChain.then(async () => {
      try {
        if (!t.id) {
          const title = entry.role === "user" ? deriveTitle(entry.text) : null;
          const created = await invoke<ConversationRecord>(
            "conversation_create",
            { title },
          );
          t.id = created.id;
          t.titled = title !== null;
          if (t.main) {
            saveMainConversationId(created.id);
            set({ mainConversationId: created.id });
          }
          if (target === t) set({ conversationId: created.id });
        }
        const stored = await invoke<{ id?: string } | null>(
          "conversation_append_entry",
          { conversationId: t.id, entry },
        );
        if (stored?.id) entryIds.set(lineKey, stored.id);
        set((s) => ({
          saveError: null,
          lines: s.lines.map((l) =>
            l.id === lineKey ? { ...l, persisted: true } : l,
          ),
        }));
        if (!t.titled && entry.role === "user") {
          const title = deriveTitle(entry.text);
          if (title) {
            await invoke("conversation_update", { id: t.id, title });
            t.titled = true;
          }
        }
        await loadConversations();
      } catch (e) {
        set({ saveError: String(e) });
      } finally {
        inflight.delete(lineKey);
      }
    });
  };

  /**
   * Alle fertigen Zeilen, die noch nicht gespeichert sind, in Reihenfolge
   * einreihen. Nutzerzeilen ohne Antwort gelten an den Grenzen (Anruf Ende,
   * Wechsel, Seite verlassen) als fertig; `all` nimmt sie mit.
   */
  const flushLines = (all = false): void => {
    const pending = get().lines.filter(
      (l) => !l.persisted && !inflight.has(l.id),
    );
    const lastFinal = Math.max(
      -1,
      ...pending.filter((l) => l.final).map((l) => l.id),
    );
    const due = pending.filter(
      (l) => l.final || (l.role === "user" && (all || l.id < lastFinal)),
    );
    if (due.length === 0) return;
    const model = get().ready?.llm ?? null;
    for (const line of due) {
      if (line.role === "assistant" && !lineUsage.has(line.id)) {
        lineUsage.set(line.id, pendingUsage);
        pendingUsage = null;
      }
      const usage = lineUsage.get(line.id) ?? null;
      const entry = entryFromLine(line, usage, model);
      if (entry) enqueueAppend(entry, line.id);
    }
  };

  /** Läuft ein Anruf, dann beenden (Wechsel des Gesprächs). */
  const endCall = async (): Promise<void> => {
    if (get().state !== "idle") await engine?.stop();
  };

  /** Zustand für ein anderes (oder leeres) Gespräch zurücksetzen. */
  const switchTo = (
    next: Target,
    id: string | null,
    lines: VoiceLine[],
    usage: CallUsage,
    projectId: string | null = null,
    projektStand: string | null = null,
  ): void => {
    textRun++;
    pendingUsage = null;
    typedNext = false;
    attachmentsNext = undefined;
    target = next;
    engine?.setHistoryLimit(next.main ? MAIN_HISTORY_LIMIT : undefined);
    set({
      conversationId: id,
      projectId,
      projektStand,
      projektHinweis: null,
      textBusy: false,
      lines,
      metrics: [],
      usage,
      error: null,
      toolStatus: null,
      notice: null,
      denken: null,
      ready: get().state === "idle" ? null : get().ready,
    });
  };

  /** Das Hauptgespräch vergessen (gelöscht, archiviert, neu begonnen). */
  const clearMain = (): void => {
    saveMainConversationId(null);
    target.main = false;
    engine?.setHistoryLimit(undefined);
    set({ mainConversationId: null });
  };

  /**
   * Leeres Hauptgespräch. Ist das offene Gespräch noch leer (nichts gesagt, nichts gespeichert),
   * wird es selbst zum Hauptgespräch; sonst beginnt ein neues.
   */
  const startFreshMain = async (): Promise<void> => {
    if (target.id !== null || get().lines.length > 0)
      await get().newConversation();
    target.main = true;
    engine?.setHistoryLimit(MAIN_HISTORY_LIMIT);
  };

  /** Kern von `toggleMainCall`; läuft nie zweimal gleichzeitig. */
  const toggleMain = async (): Promise<void> => {
    const { state, mainCall } = get();
    if (state !== "idle" && mainCall) {
      await get().stopCall();
      return;
    }
    // Ein Anruf in einem anderen Gespräch endet, bevor das Hauptgespräch spricht.
    if (state !== "idle") await get().stopCall();
    // Erst fertig speichern: ein gerade angelegtes Hauptgespräch kennt dann seine Kennung.
    await persistChain;
    const mainId = get().mainConversationId;
    if (mainId && get().conversationId !== mainId) {
      set({ error: null });
      await get().openConversation(mainId);
      if (get().conversationId !== mainId) {
        // Ohne Fehler hat ein anderer Wechsel übernommen: nichts starten.
        if (!get().error) return;
        // Gelöscht oder nicht lesbar: ein neues Hauptgespräch beginnt.
        clearMain();
        set({ error: null });
        await startFreshMain();
      }
    } else if (!mainId) {
      await startFreshMain();
    } else {
      target.main = true;
    }
    if (get().switching) return;
    // Kontext klein halten: nur die jüngsten Wechsel, nach langer Pause gar keiner.
    getEngine().setHistoryLimit(MAIN_HISTORY_LIMIT);
    getEngine().setHistory(compactMainHistory(get().lines, Date.now()));
    set({ mainCall: true });
    await get().startCall();
    if (get().state === "idle") set({ mainCall: false });
  };

  return {
    config: loadCallConfig(),
    active: false,
    state: "idle",
    ready: null,
    error: null,
    toolStatus: null,
    notice: null,
    denken: null,
    metrics: [],
    lines: [],
    usage: EMPTY_USAGE,
    muted: false,
    textBusy: false,
    settingsOpen: false,
    settingsTab: "talk",
    conversationId: null,
    projectId: null,
    projektStand: null,
    projektHinweis: null,
    conversations: [],
    showArchived: false,
    switching: false,
    saveError: null,
    mainConversationId: loadMainConversationId(),
    mainCall: false,

    setActive: (active) => {
      if (active) visit++;
      set({ active });
    },

    setConfig: (update) => {
      const config = update(get().config);
      saveCallConfig(config);
      engine?.updateConfig(config);
      set({ config });
    },
    resetConfig: () => get().setConfig(() => DEFAULT_CALL_CONFIG),

    setMuted: (muted) => {
      engine?.setMuted(muted);
      set({ muted });
    },

    openSettings: (tab) =>
      set((s) => ({ settingsOpen: true, settingsTab: tab ?? s.settingsTab })),
    closeSettings: () => set({ settingsOpen: false }),
    setSettingsTab: (settingsTab) => set({ settingsTab }),
    knowledgeChanged: () => engine?.invalidateKnowledge(),

    startCall: async () => {
      if (get().state !== "idle" || get().switching) return;
      set({ error: null });
      try {
        set({ ready: await getEngine().start() });
      } catch (e) {
        // Gestoppt, bevor der Start fertig war: kein Fehler.
        if (!(e instanceof Error && e.message === START_ABORTED))
          set({ error: String(e) });
      }
    },

    stopCall: async () => {
      await engine?.stop();
      set({ mainCall: false });
      flushLines(true);
    },

    sendText: async (text, display, attachments) => {
      const clean = text.trim();
      if (!clean || get().switching) return;
      const e = getEngine();
      set({ error: null });
      typedNext = true;
      attachmentsNext = attachments?.length ? attachments : undefined;
      if (e.inCall) {
        try {
          await e.sendText(clean, display);
        } finally {
          typedNext = false;
          attachmentsNext = undefined;
        }
        return;
      }
      const run = ++textRun;
      set({ textBusy: true });
      try {
        await e.sendText(clean, display);
      } finally {
        typedNext = false;
        attachmentsNext = undefined;
        // Eine abgebrochene, ältere Anfrage darf die neue nicht „fertig“ melden.
        if (run === textRun) set({ textBusy: false, denken: null });
      }
    },

    editLine: async (lineKey, text) => {
      const { lines, state, textBusy } = get();
      const line = lines.find((l) => l.id === lineKey);
      const next = text.trim().slice(0, MAX_ENTRY_CHARS);
      if (!line || !line.turnId || !lineEditable(line, state, textBusy)) return;
      if (!next || next === line.text) return;
      // Die Rückmeldung geht immer vom Original der Erkennung aus, auch nach einer zweiten Korrektur.
      const heard = line.recognized ?? line.text;
      set((s) => ({
        lines: s.lines.map((l) =>
          l.id === lineKey ? { ...l, text: next } : l,
        ),
      }));
      engine?.correctUserText(line.turnId, next);
      persistChain = persistChain.then(async () => {
        const id = entryIds.get(lineKey);
        // Ohne Kennung ist die Zeile noch nicht gespeichert: das Speichern nimmt den neuen Text mit.
        if (!id) return;
        try {
          await invoke("conversation_update_entry", {
            id,
            text: next,
            status: "complete",
          });
          await loadConversations();
        } catch (e) {
          set({ saveError: String(e) });
        }
      });
      await persistChain;
      await sendFeedback(heard, next);
    },

    denkOffen: (seit, offen) =>
      set((s) => ({
        denken: s.denken?.seit === seit ? { ...s.denken, offen } : s.denken,
        lines: s.lines.some((l) => l.denken?.seit === seit)
          ? s.lines.map((l) =>
              l.denken?.seit === seit
                ? { ...l, denken: { ...l.denken, offen } }
                : l,
            )
          : s.lines,
      })),
    denkModell: (modell) =>
      set((s) =>
        s.denken && s.denken.modell !== modell
          ? { denken: { ...s.denken, modell } }
          : {},
      ),
    stopText: () => {
      engine?.cancelText(true);
      textRun++;
      set({ textBusy: false, denken: null });
    },

    newConversation: async () => {
      openRun++;
      set({ switching: false });
      await endCall();
      flushLines(true);
      engine?.resetHistory();
      switchTo({ id: null, titled: false }, null, [], EMPTY_USAGE);
    },

    newProjectConversation: async (projectId, stand) => {
      const run = ++openRun;
      set({ switching: true });
      await endCall();
      flushLines(true);
      await persistChain;
      try {
        // Die Bindung setzt nur das Anlegen; das Projekt prüft Rust (es muss es geben, nicht archiviert).
        const created = await invoke<ConversationRecord>(
          "conversation_create",
          {
            title: null,
            projectId,
          },
        );
        if (run !== openRun) return;
        engine?.resetHistory();
        switchTo(
          { id: created.id, titled: false },
          created.id,
          [],
          EMPTY_USAGE,
          projectId,
          stand,
        );
      } catch (e) {
        if (run === openRun) set({ error: String(e) });
      } finally {
        if (run === openRun) set({ switching: false });
      }
      await loadConversations();
    },

    setProjektStand: (stand) =>
      set({ projektStand: stand, projektHinweis: null }),
    dismissProjektHinweis: () => set({ projektHinweis: null }),

    loadConversations,

    openConversation: async (id) => {
      if (id === get().conversationId || get().switching) return;
      const run = ++openRun;
      set({ switching: true });
      await endCall();
      flushLines(true);
      await persistChain;
      try {
        const detail = await invoke<ConversationDetail>("conversation_get", {
          id,
        });
        if (run !== openRun) return;
        const lines = detail.entries
          .filter(
            (e) => (e.role === "user" || e.role === "assistant") && e.text,
          )
          .map((e) => {
            const l = lineFromEntry(e, lineId++);
            entryIds.set(l.id, e.id);
            return l;
          });
        // Das Limit des Ziels gilt schon beim Laden (Hauptgespräch: kompakt).
        getEngine().setHistoryLimit(
          id === get().mainConversationId ? MAIN_HISTORY_LIMIT : undefined,
        );
        getEngine().setHistory(
          lines.map((l) => ({
            role: l.role,
            content: l.text,
            turnId: l.turnId,
          })),
        );
        switchTo(
          {
            id,
            titled: detail.conversation.title !== DEFAULT_TITLE,
            main: id === get().mainConversationId,
          },
          id,
          lines,
          usageFromEntries(detail.entries),
          // Die Bindung kommt aus dem gespeicherten Gespräch. Der Stand der Auswahl ist unbekannt, bis
          // die Person ihn sieht: ohne Stand prüft das Tor nur gegen den aktuellen Kontext.
          detail.conversation.scope?.project_id ?? null,
          null,
        );
      } catch (e) {
        if (run === openRun) set({ error: String(e) });
        void loadConversations();
      } finally {
        if (run === openRun) set({ switching: false });
      }
    },

    rename: async (id, title) => {
      try {
        await invoke("conversation_update", { id, title });
      } catch (e) {
        set({ error: String(e) });
      }
      await loadConversations();
    },

    remove: async (id) => {
      if (id === get().mainConversationId) clearMain();
      if (id === target.id) await get().newConversation();
      await persistChain;
      try {
        await invoke("conversation_delete", { id });
      } catch (e) {
        set({ error: String(e) });
      }
      await loadConversations();
    },

    togglePin: async (id) => {
      const current = get().conversations.find((c) => c.id === id);
      try {
        await invoke("conversation_set_pinned", {
          id,
          pinned: !(current?.pinned ?? false),
        });
      } catch (e) {
        set({ error: String(e) });
      }
      await loadConversations();
    },

    archive: async (id, archived = true) => {
      if (archived && id === get().mainConversationId) clearMain();
      if (archived && id === target.id) await get().newConversation();
      await persistChain;
      try {
        await invoke("conversation_update", { id, archived });
      } catch (e) {
        set({ error: String(e) });
      }
      await loadConversations();
    },

    setShowArchived: async (showArchived) => {
      set({ showArchived });
      await loadConversations();
    },

    leave: async () => {
      const left = visit;
      // Ein Anruf im Hauptgespräch läuft weiter, auch ohne die Seite (Tastenkürzel).
      if (get().mainCall && get().state !== "idle") {
        set({ active: false, settingsOpen: false });
        return;
      }
      await engine?.stop();
      flushLines(true);
      if (visit !== left) return;
      await engine?.dispose();
      if (visit !== left) return;
      set({ active: false, settingsOpen: false });
    },

    toggleMainCall: async () => {
      // Ein Druck während eines laufenden Umschaltens: läuft gerade der Start, bricht er
      // ihn ab (der Start kann lange laden); sonst wird er verworfen. Nie zwei Starts.
      if (toggling) {
        if (get().state === "starting") await engine?.stop();
        return;
      }
      if (get().switching) return;
      toggling = true;
      try {
        await toggleMain();
      } finally {
        toggling = false;
      }
    },

    newMainConversation: async () => {
      const wasOpen = target.main;
      if (get().state !== "idle" && get().mainCall) await get().stopCall();
      clearMain();
      // Ist das alte Hauptgespräch offen, beginnt gleich ein leeres als neues.
      if (wasOpen) await startFreshMain();
    },
  };
});
