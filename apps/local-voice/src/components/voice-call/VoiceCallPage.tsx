import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Mic, PhoneOff, Phone } from "lucide-react";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import {
  DEFAULT_CALL_CONFIG,
  DEFAULT_SYSTEM_PROMPT,
  VoiceCallEngine,
  type CallConfig,
  type CallReady,
  type CallState,
  type TurnMetrics,
} from "./engine";
import { DEFAULT_TURN_CONFIG } from "./turnTaking";

const STORAGE_KEY = "voice-call.config";

interface Line {
  id: number;
  role: "user" | "assistant";
  text: string;
  final?: boolean;
  interrupted?: boolean;
}

function loadConfig(): CallConfig {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
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

function median(values: number[]): number {
  if (values.length === 0) return 0;
  const s = [...values].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

export const VoiceCallPage: React.FC = () => {
  const { t } = useTranslation();
  const [state, setState] = useState<CallState>("idle");
  const [lines, setLines] = useState<Line[]>([]);
  const [ready, setReady] = useState<CallReady | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<TurnMetrics[]>([]);
  const [config, setConfig] = useState<CallConfig>(loadConfig);
  const levelRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<VoiceCallEngine | null>(null);
  const lineId = useRef(0);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
    } catch {
      /* nicht speichern ist verkraftbar */
    }
    engineRef.current?.updateConfig(config);
  }, [config]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  useEffect(() => () => void engineRef.current?.stop(), []);

  const start = useCallback(async () => {
    setError(null);
    setLines([]);
    setMetrics([]);
    const engine = new VoiceCallEngine(
      {
        onState: setState,
        onLevel: (level) => {
          const el = levelRef.current;
          if (el) el.style.transform = `scaleX(${Math.min(1, level * 8)})`;
        },
        onUser: (text) =>
          setLines((l) => [...l, { id: lineId.current++, role: "user", text }]),
        onAssistant: (text, final, interrupted) =>
          setLines((l) => {
            const last = l[l.length - 1];
            if (last && last.role === "assistant" && !last.final) {
              return [...l.slice(0, -1), { ...last, text, final, interrupted }];
            }
            return [
              ...l,
              {
                id: lineId.current++,
                role: "assistant",
                text,
                final,
                interrupted,
              },
            ];
          }),
        onMetrics: (m) => setMetrics((all) => [...all.slice(-19), m]),
        onError: setError,
      },
      config,
    );
    engineRef.current = engine;
    try {
      setReady(await engine.start());
    } catch (e) {
      setError(String(e));
      engineRef.current = null;
    }
  }, [config]);

  const stop = useCallback(async () => {
    await engineRef.current?.stop();
    engineRef.current = null;
  }, []);

  const active = state !== "idle";
  const last = metrics[metrics.length - 1];
  const med = useMemo(
    () => median(metrics.map((m) => m.firstSoundMs)),
    [metrics],
  );

  const setTurn = (patch: Partial<CallConfig["turn"]>) =>
    setConfig((c) => ({ ...c, turn: { ...c.turn, ...patch } }));

  return (
    <PageShell
      title={t("voiceCall.title")}
      description={t("voiceCall.description")}
      fill
      actions={
        active ? (
          <Button variant="danger" size="sm" onClick={() => void stop()}>
            <PhoneOff className="w-4 h-4 mr-1" />
            {t("voiceCall.end")}
          </Button>
        ) : (
          <Button variant="primary" size="sm" onClick={() => void start()}>
            <Phone className="w-4 h-4 mr-1" />
            {t("voiceCall.begin")}
          </Button>
        )
      }
    >
      <div className="flex flex-col gap-3 h-full min-h-0 p-1">
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <span
            className="inline-flex items-center gap-2 rounded-full border border-mid-gray/30 px-3 py-1"
            role="status"
            aria-live="polite"
          >
            <Mic className="w-4 h-4" />
            {t(`voiceCall.state.${state}`)}
          </span>
          <div
            className="h-2 w-40 rounded bg-mid-gray/20 overflow-hidden"
            aria-hidden="true"
          >
            <div
              ref={levelRef}
              className="h-full w-full origin-left bg-logo-primary"
              style={{ transform: "scaleX(0)" }}
            />
          </div>
          {ready?.llm ? (
            <span className="text-text/60">
              {t("voiceCall.model", { model: ready.llm })}
              {ready.llm_local ? ` · ${t("voiceCall.local")}` : ""}
            </span>
          ) : null}
          {last ? (
            <span
              className="text-text/60"
              title={t("voiceCall.latencyDetail", {
                stt: last.sttMs,
                llm: last.llmFirstMs,
                tts: last.ttsFirstMs,
              })}
            >
              {t("voiceCall.latency", { ms: last.firstSoundMs, median: med })}
            </span>
          ) : null}
        </div>

        {ready?.warnings.map((w) => (
          <p key={w} className="text-sm text-amber-600" role="note">
            {w}
          </p>
        ))}
        {error ? (
          <p className="text-sm text-red-600" role="alert">
            {error}
          </p>
        ) : null}

        <div
          className="flex-1 min-h-0 overflow-y-auto rounded-lg border border-mid-gray/20 p-3 flex flex-col gap-2"
          aria-label={t("voiceCall.transcript")}
        >
          {lines.length === 0 ? (
            <p className="text-text/60 text-sm">
              {active ? t("voiceCall.speakNow") : t("voiceCall.hint")}
            </p>
          ) : (
            lines.map((line) => (
              <div
                key={line.id}
                className={`max-w-[85%] rounded-lg px-3 py-2 text-sm ${
                  line.role === "user"
                    ? "self-end bg-logo-primary/15"
                    : "self-start bg-mid-gray/15"
                }`}
              >
                {line.text}
                {line.interrupted ? " …" : ""}
              </div>
            ))
          )}
          <div ref={endRef} />
        </div>

        <details className="rounded-lg border border-mid-gray/20 p-3 text-sm">
          <summary className="cursor-pointer select-none">
            {t("voiceCall.settings.title")}
          </summary>
          <div className="mt-3 grid gap-3 md:grid-cols-2">
            <label className="flex flex-col gap-1 md:col-span-2">
              {t("voiceCall.settings.prompt")}
              <textarea
                className="rounded border border-mid-gray/30 bg-background p-2 min-h-24"
                value={config.systemPrompt}
                onChange={(e) =>
                  setConfig((c) => ({ ...c, systemPrompt: e.target.value }))
                }
              />
            </label>
            <label className="flex flex-col gap-1">
              {t("voiceCall.settings.endMs", { ms: config.turn.endMs })}
              <input
                type="range"
                min={400}
                max={1500}
                step={50}
                value={config.turn.endMs}
                onChange={(e) =>
                  setTurn({
                    endMs: Number(e.target.value),
                    earlyEndMs: Math.min(
                      config.turn.earlyEndMs,
                      Number(e.target.value) - 100,
                    ),
                  })
                }
              />
            </label>
            <label className="flex flex-col gap-1">
              {t("voiceCall.settings.earlyMs", { ms: config.turn.earlyEndMs })}
              <input
                type="range"
                min={200}
                max={Math.max(300, config.turn.endMs - 100)}
                step={20}
                value={config.turn.earlyEndMs}
                onChange={(e) =>
                  setTurn({ earlyEndMs: Number(e.target.value) })
                }
              />
            </label>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={config.turn.bargeIn}
                onChange={(e) => setTurn({ bargeIn: e.target.checked })}
              />
              {t("voiceCall.settings.bargeIn")}
            </label>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setConfig(DEFAULT_CALL_CONFIG)}
            >
              {t("voiceCall.settings.reset")}
            </Button>
            <p className="md:col-span-2 text-text/60">
              {t("voiceCall.settings.note")}
            </p>
          </div>
        </details>
      </div>
    </PageShell>
  );
};
