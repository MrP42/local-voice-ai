import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Mic } from "lucide-react";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { CAPTURE_RATE, floatToPcm16Base64, startCapture } from "./audio";
import type { Capture } from "./audio";
import {
  evaluateSetup,
  getSetupText,
  prepareSetup,
  refreshWords,
  setPairStatus,
  type LearnedPair,
  type LearnedStatus,
  type SetupOutcome,
  type SetupText,
} from "./speechLearning";
import { PairLine } from "./PairLine";

/** Länger wird nicht aufgenommen (das Vorlesen dauert etwa eine Minute). */
const MAX_SECONDS = 90;
/** Kürzer als das ist keine Aufnahme. */
const MIN_SECONDS = 1;

type Phase =
  | { kind: "loading" }
  | { kind: "idle" }
  | { kind: "recording" }
  | { kind: "working" }
  | { kind: "result"; outcome: SetupOutcome }
  | { kind: "error"; message: string };

/**
 * Einrichtung durch Vorlesen: zeigt den Vorlesetext, nimmt das Vorlesen im Arbeitsspeicher auf,
 * lässt die vorhandene Spracherkennung (`voice_call_transcribe`) darüber laufen und legt die
 * Verhörer als Vorschläge ab. Das Audio verlässt den Arbeitsspeicher nie und wird nach der
 * Erkennung verworfen.
 */
export const SetupDialog: React.FC<{
  open: boolean;
  onClose: () => void;
  /** Die Liste der gelernten Begriffe hat sich geändert. */
  onChanged: () => void;
}> = ({ open, onClose, onChanged }) => {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [text, setText] = useState<SetupText | null>(null);
  const [seconds, setSeconds] = useState(0);
  const [pairs, setPairs] = useState<LearnedPair[]>([]);
  const frames = useRef<Float32Array[]>([]);
  const capture = useRef<Capture | null>(null);
  const ctx = useRef<AudioContext | null>(null);
  const timer = useRef<number | null>(null);
  const run = useRef(0);

  const releaseAudio = useCallback(() => {
    if (timer.current !== null) window.clearInterval(timer.current);
    timer.current = null;
    capture.current?.stop();
    capture.current = null;
    void ctx.current?.close().catch(() => {});
    ctx.current = null;
  }, []);

  // Öffnen: Text holen, Modell laden. Schließen: alles verwerfen.
  useEffect(() => {
    if (!open) return;
    const mine = ++run.current;
    setPhase({ kind: "loading" });
    setText(null);
    setPairs([]);
    frames.current = [];
    void (async () => {
      try {
        const [setup] = await Promise.all([getSetupText(), prepareSetup()]);
        if (mine !== run.current) return;
        setText(setup);
        setPhase({ kind: "idle" });
      } catch (e) {
        if (mine === run.current)
          setPhase({ kind: "error", message: String(e) });
      }
    })();
    return () => {
      run.current++;
      releaseAudio();
      frames.current = [];
    };
  }, [open, releaseAudio]);

  const evaluate = useCallback(
    async (reference: string) => {
      const mine = run.current;
      setPhase({ kind: "working" });
      const chunks = frames.current;
      frames.current = [];
      const total = chunks.reduce((n, c) => n + c.length, 0);
      if (total < CAPTURE_RATE * MIN_SECONDS) {
        setPhase({ kind: "error", message: t("voiceCall.setup.tooShort") });
        return;
      }
      const samples = new Float32Array(total);
      let at = 0;
      for (const c of chunks) {
        samples.set(c, at);
        at += c.length;
      }
      try {
        const transcript = await invoke<string>("voice_call_transcribe", {
          pcm16B64: floatToPcm16Base64(samples),
        });
        if (mine !== run.current) return;
        const outcome = await evaluateSetup(reference, transcript);
        if (mine !== run.current) return;
        setPairs(outcome.pairs);
        setPhase({ kind: "result", outcome });
        onChanged();
      } catch (e) {
        if (mine === run.current)
          setPhase({ kind: "error", message: String(e) });
      }
    },
    [onChanged, t],
  );

  const stop = useCallback(() => {
    releaseAudio();
    if (text) void evaluate(text.text);
  }, [evaluate, releaseAudio, text]);

  const start = async () => {
    const mine = run.current;
    frames.current = [];
    setSeconds(0);
    try {
      const context = new AudioContext({ latencyHint: "interactive" });
      ctx.current = context;
      await context.resume();
      const cap = await startCapture(context, {
        onFrame: (f) => frames.current.push(f),
      });
      if (mine !== run.current) {
        cap.stop();
        return;
      }
      capture.current = cap;
      setPhase({ kind: "recording" });
      timer.current = window.setInterval(() => setSeconds((s) => s + 1), 1000);
    } catch {
      releaseAudio();
      setPhase({ kind: "error", message: t("voiceCall.setup.micError") });
    }
  };

  // Höchstdauer: dann automatisch fertig.
  useEffect(() => {
    if (phase.kind === "recording" && seconds >= MAX_SECONDS) stop();
  }, [phase.kind, seconds, stop]);

  const decide = async (pair: LearnedPair, status: LearnedStatus) => {
    try {
      const next = await setPairStatus(pair.id, status);
      setPairs((all) => all.map((p) => (p.id === next.id ? next : p)));
      await refreshWords();
      onChanged();
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const busy = phase.kind === "loading" || phase.kind === "working";
  const footer =
    phase.kind === "recording" ? (
      <>
        <Button variant="primary" type="button" onClick={stop}>
          {t("voiceCall.setup.stop")}
        </Button>
        <Button type="button" onClick={onClose}>
          {t("voiceCall.setup.cancel")}
        </Button>
      </>
    ) : (
      <>
        {phase.kind === "idle" && (
          <Button variant="primary" type="button" onClick={() => void start()}>
            {t("voiceCall.setup.start")}
          </Button>
        )}
        {(phase.kind === "result" || phase.kind === "error") && (
          <Button
            variant="primary"
            type="button"
            onClick={() => setPhase({ kind: "idle" })}
            disabled={!text}
          >
            {t("voiceCall.setup.again")}
          </Button>
        )}
        <Button type="button" onClick={onClose}>
          {t("voiceCall.setup.close")}
        </Button>
      </>
    );

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("voiceCall.setup.title")}
      description={t("voiceCall.setup.intro")}
      closeLabel={t("voiceCall.setup.close")}
      className="max-w-2xl"
      footer={footer}
    >
      <div
        className="flex flex-col gap-3 text-sm"
        data-testid="setup-dialog"
        aria-busy={busy}
      >
        {text && (
          <div
            className="rounded-lg border border-mid-gray/25 bg-mid-gray/10 px-3 py-2.5 leading-relaxed flex flex-col gap-1.5"
            aria-label={t("voiceCall.setup.textLabel")}
            role="group"
            data-testid="setup-text"
          >
            {text.sentences.map((s, i) => (
              <p key={i} className="m-0">
                {s}
              </p>
            ))}
          </div>
        )}
        <div role="status" aria-live="polite" className="flex flex-col gap-2">
          {phase.kind === "loading" && <p>{t("voiceCall.setup.loading")}</p>}
          {phase.kind === "recording" && (
            <p className="inline-flex items-center gap-2 font-semibold text-logo-primary">
              <Mic className="w-4 h-4" aria-hidden="true" />
              {t("voiceCall.setup.recording", { seconds })}
            </p>
          )}
          {phase.kind === "working" && <p>{t("voiceCall.setup.working")}</p>}
          {phase.kind === "error" && (
            <p role="alert" className="text-red-600 dark:text-red-400">
              {phase.message}
            </p>
          )}
          {phase.kind === "result" && (
            <>
              <p className="m-0">
                {t(`voiceCall.setup.result.${phase.outcome.reason}`, {
                  percent: phase.outcome.matched_percent,
                })}
              </p>
              {pairs.length > 0 && (
                <ul
                  className="m-0 p-0 list-none flex flex-col gap-1.5"
                  aria-label={t("voiceCall.setup.suggestions")}
                >
                  {pairs.map((p) => (
                    <PairLine
                      key={p.id}
                      pair={p}
                      onSet={(status) => void decide(p, status)}
                    />
                  ))}
                </ul>
              )}
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
};
