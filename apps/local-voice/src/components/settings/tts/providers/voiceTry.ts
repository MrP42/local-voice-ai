import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { commands, events } from "@/bindings";
import { stopSample, toggleSample } from "@/lib/voices/samplePlayer";
import { asTtsError } from "./providerModel";

/**
 * Reinhoeren in Stimmen ohne fertige Hoerprobe (Fish, Piper, Cloud-Anbieter):
 * ein Probesatz von wenigen Sekunden, erzeugt beim Klick. Fish legt die Probe
 * als Datei ab (danach wie jede andere Hoerprobe); Piper und die Anbieter
 * spricht die Probestimme des Backends direkt. Eine Probe zur Zeit, app-weit.
 */
export type VoiceTryTarget =
  | { kind: "fish"; voiceId: string }
  | { kind: "provider"; providerId: string; voice: string | null };

type TryState = { key: string; requestId: string | null } | null;

let state: TryState = null;
const listeners = new Set<() => void>();
const set = (next: TryState) => {
  state = next;
  listeners.forEach((l) => l());
};

/** Kennung des Eintrags, dessen Probe gerade erzeugt wird oder laeuft. */
export const useTryingKey = (): string | null =>
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => state?.key ?? null,
  );

export const tryKey = (target: VoiceTryTarget): string =>
  target.kind === "fish"
    ? `fish:${target.voiceId}`
    : `${target.providerId}:${target.voice ?? ""}`;

const cancelRunning = () => {
  const requestId = state?.requestId;
  if (requestId) void commands.ttsProviderCancel(requestId).catch(() => null);
  set(null);
};

let wired = false;
const wire = () => {
  if (wired) return;
  wired = true;
  void events.ttsProviderPreview.listen(({ payload }) => {
    if (!state || state.requestId !== payload.request_id) return;
    if (
      payload.phase === "done" ||
      payload.phase === "cancelled" ||
      payload.phase === "error"
    ) {
      if (payload.phase === "error" && payload.message) {
        toast.error(payload.message);
      }
      set(null);
    }
  });
};

/** Probe starten; laeuft sie fuer denselben Eintrag schon, stoppt der Aufruf sie. */
export async function toggleTry(target: VoiceTryTarget, text: string) {
  const key = tryKey(target);
  const wasRunning = state?.key === key;
  cancelRunning();
  stopSample();
  if (wasRunning) return;
  wire();
  set({ key, requestId: null });
  try {
    if (target.kind === "fish") {
      const cached = await commands.ttsVoiceDemoCached(target.voiceId);
      const sample =
        cached ??
        (await (async () => {
          const result = await commands.ttsVoiceDemo(target.voiceId);
          if (result.status === "error") throw new Error(result.error);
          return result.data;
        })());
      if (state?.key !== key) return;
      set(null);
      toggleSample(sample.wav_path);
      return;
    }
    const result = await commands.ttsProviderPreview(
      target.providerId,
      text,
      target.voice,
    );
    if (result.status === "error") throw asTtsError(result.error);
    if (state?.key !== key) {
      void commands.ttsProviderCancel(result.data).catch(() => null);
      return;
    }
    set({ key, requestId: result.data });
  } catch (error) {
    if (state?.key === key) set(null);
    toast.error(
      error instanceof Error ? error.message : asTtsError(error).message,
    );
  }
}
