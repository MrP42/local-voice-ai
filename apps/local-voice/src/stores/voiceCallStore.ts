import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import {
  DEFAULT_CALL_CONFIG,
  DEFAULT_SYSTEM_PROMPT,
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
import {
  DEFAULT_ORCHESTRATION,
  SOUVERAENITAET_STUFEN,
  type CallStatus,
} from "@/components/voice-call/orchestration";
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

export type VoiceSettingsTab = "talk" | "voice" | "recognition" | "models";

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
  /** Gesprochen (Anruf) oder getippt; so steht es im gespeicherten Beitrag. */
  mode?: "typed" | "spoken";
  /** Schon im Verlauf gespeichert (oder zum Speichern eingereiht). */
  persisted?: boolean;
  /** Dateien, die der Nutzer mitgeschickt hat (nur Name, Typ, Größe; kein Inhalt). */
  attachments?: HistoryAttachment[];
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
  };
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
  conversations: ConversationSummary[];
  /** Archivierte Gespräche in der Liste zeigen. */
  showArchived: boolean;
  /** Ein anderes Gespräch wird gerade geladen: Eingabe gesperrt. */
  switching: boolean;
  /** Letzter Fehler beim Speichern im Verlauf (getrennt von Gesprächsfehlern). */
  saveError: string | null;

  setActive: (active: boolean) => void;
  setConfig: (update: (c: CallConfig) => CallConfig) => void;
  resetConfig: () => void;
  setMuted: (muted: boolean) => void;
  openSettings: (tab?: VoiceSettingsTab) => void;
  closeSettings: () => void;
  setSettingsTab: (tab: VoiceSettingsTab) => void;
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
  /** Seite verlassen: Anruf beenden, Verlauf bleibt. */
  leave: () => Promise<void>;
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
        onState: (state) => set({ state }),
        // Der Pegel läuft am Store vorbei: 50 Bilder je Sekunde gehören nicht in den Zustand.
        onLevel: (level) => levelListeners.forEach((fn) => fn(level)),
        onUser: (text) => {
          const mode = typedNext ? "typed" : "spoken";
          const attachments = attachmentsNext;
          typedNext = false;
          attachmentsNext = undefined;
          set({ notice: null, toolStatus: null });
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
          set((s) => {
            const l = s.lines;
            const last = l[l.length - 1];
            if (last && last.role === "assistant" && !last.final) {
              return {
                lines: [
                  ...l.slice(0, -1),
                  { ...last, text, final, interrupted },
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
        onError: (error) => set({ error }),
        onStatus: (status) => {
          if (status === null) set({ toolStatus: null });
          else if (status.kind === "tool") set({ toolStatus: status.tool });
          else set({ notice: status.code });
        },
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
  ): void => {
    textRun++;
    pendingUsage = null;
    typedNext = false;
    attachmentsNext = undefined;
    target = next;
    set({
      conversationId: id,
      textBusy: false,
      lines,
      metrics: [],
      usage,
      error: null,
      toolStatus: null,
      notice: null,
      ready: get().state === "idle" ? null : get().ready,
    });
  };

  return {
    config: loadCallConfig(),
    active: false,
    state: "idle",
    ready: null,
    error: null,
    toolStatus: null,
    notice: null,
    metrics: [],
    lines: [],
    usage: EMPTY_USAGE,
    muted: false,
    textBusy: false,
    settingsOpen: false,
    settingsTab: "talk",
    conversationId: null,
    conversations: [],
    showArchived: false,
    switching: false,
    saveError: null,

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

    startCall: async () => {
      if (get().state !== "idle" || get().switching) return;
      set({ error: null });
      try {
        set({ ready: await getEngine().start() });
      } catch (e) {
        set({ error: String(e) });
      }
    },

    stopCall: async () => {
      await engine?.stop();
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
        if (run === textRun) set({ textBusy: false });
      }
    },

    editLine: async (lineKey, text) => {
      const line = get().lines.find((l) => l.id === lineKey);
      const next = text.trim().slice(0, MAX_ENTRY_CHARS);
      if (!line || line.role !== "user" || line.mode !== "spoken") return;
      if (!next || next === line.text) return;
      const heard = line.text;
      set((s) => ({
        lines: s.lines.map((l) =>
          l.id === lineKey ? { ...l, text: next } : l,
        ),
      }));
      engine?.correctUserText(heard, next);
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

    stopText: () => {
      engine?.cancelText(true);
      textRun++;
      set({ textBusy: false });
    },

    newConversation: async () => {
      openRun++;
      set({ switching: false });
      await endCall();
      flushLines(true);
      engine?.resetHistory();
      switchTo({ id: null, titled: false }, null, [], EMPTY_USAGE);
    },

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
        getEngine().setHistory(
          lines.map((l) => ({ role: l.role, content: l.text })),
        );
        switchTo(
          { id, titled: detail.conversation.title !== DEFAULT_TITLE },
          id,
          lines,
          usageFromEntries(detail.entries),
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
        await invoke("conversation_update", {
          id,
          pinned: !(current?.pinned ?? false),
        });
      } catch (e) {
        set({ error: String(e) });
      }
      await loadConversations();
    },

    archive: async (id, archived = true) => {
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
      await engine?.stop();
      flushLines(true);
      if (visit !== left) return;
      await engine?.dispose();
      if (visit !== left) return;
      set({ active: false, settingsOpen: false });
    },
  };
});
