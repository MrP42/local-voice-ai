import { create } from "zustand";
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
import { DEFAULT_TURN_CONFIG } from "@/components/voice-call/turnTaking";

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
}

/** Verbrauch des Gesprächs seit „Neues Gespräch“. */
export interface CallUsage {
  promptTokens: number;
  completionTokens: number;
  usageEventIds: number[];
  providerId: string | null;
  local: boolean;
}

const EMPTY_USAGE: CallUsage = {
  promptTokens: 0,
  completionTokens: 0,
  usageEventIds: [],
  providerId: null,
  local: false,
};

export function loadCallConfig(): CallConfig {
  try {
    const raw = window.localStorage.getItem(VOICE_CALL_CONFIG_KEY);
    if (!raw) return DEFAULT_CALL_CONFIG;
    const parsed = JSON.parse(raw) as Partial<CallConfig>;
    return {
      systemPrompt: parsed.systemPrompt?.trim() || DEFAULT_SYSTEM_PROMPT,
      turn: { ...DEFAULT_TURN_CONFIG, ...parsed.turn },
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
  metrics: TurnMetrics[];
  lines: VoiceLine[];
  usage: CallUsage;
  muted: boolean;
  /** Text-Antwort ohne Anruf wird gerade geholt. */
  textBusy: boolean;
  settingsOpen: boolean;
  settingsTab: VoiceSettingsTab;

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
  sendText: (text: string, display?: string) => Promise<void>;
  newConversation: () => void;
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

export const useVoiceCallStore = create<VoiceCallStore>()((set, get) => {
  const getEngine = (): VoiceCallEngine => {
    if (engine) return engine;
    engine = new VoiceCallEngine(
      {
        onState: (state) => set({ state }),
        // Der Pegel läuft am Store vorbei: 50 Bilder je Sekunde gehören nicht in den Zustand.
        onLevel: (level) => levelListeners.forEach((fn) => fn(level)),
        onUser: (text) =>
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
                { id: lineId++, role: "user", text, at: Date.now() },
              ],
            };
          }),
        onAssistant: (text, final, interrupted) =>
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
                },
              ],
            };
          }),
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
        onUsage: (u: CallUsageEvent) =>
          set((s) => ({
            usage: {
              promptTokens: s.usage.promptTokens + (u.prompt_tokens ?? 0),
              completionTokens:
                s.usage.completionTokens + (u.completion_tokens ?? 0),
              usageEventIds: [
                ...s.usage.usageEventIds,
                ...(u.usage_event_ids ??
                  (u.usage_event_id != null ? [u.usage_event_id] : [])),
              ],
              providerId: u.provider_id ?? s.usage.providerId,
              local: u.local ?? s.usage.local,
            },
          })),
      },
      get().config,
    );
    engine.setMuted(get().muted);
    return engine;
  };

  return {
    config: loadCallConfig(),
    active: false,
    state: "idle",
    ready: null,
    error: null,
    metrics: [],
    lines: [],
    usage: EMPTY_USAGE,
    muted: false,
    textBusy: false,
    settingsOpen: false,
    settingsTab: "talk",

    setActive: (active) => set({ active }),

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
      if (get().state !== "idle") return;
      set({ error: null });
      try {
        set({ ready: await getEngine().start() });
      } catch (e) {
        set({ error: String(e) });
      }
    },

    stopCall: async () => {
      await engine?.stop();
    },

    sendText: async (text, display) => {
      const clean = text.trim();
      if (!clean) return;
      const e = getEngine();
      set({ error: null });
      if (e.inCall) {
        await e.sendText(clean, display);
        return;
      }
      set({ textBusy: true });
      try {
        await e.sendText(clean, display);
      } finally {
        set({ textBusy: false });
      }
    },

    newConversation: () => {
      engine?.resetHistory();
      set({
        lines: [],
        metrics: [],
        usage: EMPTY_USAGE,
        error: null,
        ready: get().state === "idle" ? null : get().ready,
      });
    },

    leave: async () => {
      await engine?.stop();
      engine?.dispose();
      set({ active: false, settingsOpen: false });
    },
  };
});
