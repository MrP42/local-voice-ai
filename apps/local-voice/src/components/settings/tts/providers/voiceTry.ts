import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { commands, events } from "@/bindings";
import {
  onSampleStart,
  stopSample,
  toggleSample,
} from "@/lib/voices/samplePlayer";
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
/** Zaehler der Laeufe: jede asynchrone Fortsetzung prueft, ob ihr Lauf noch der aktuelle ist. */
let seq = 0;
/** Endereignisse, die vor der Antwort des Starts eintrafen (request_id -> Fehlertext/null). */
const finished = new Map<string, string | null>();
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

// Randfall: `voice === null` (Standardstimme) und `voice === ""` ergeben denselben
// Schluessel `<anbieter>:`; eine Fish-Probe mit leerer Stimmen-Kennung (Phase ""
// ohne Sprecher) bleibt davon getrennt, weil ihr Schluessel mit `fish:` beginnt.
export const tryKey = (target: VoiceTryTarget): string =>
  target.kind === "fish"
    ? `fish:${target.voiceId}`
    : `${target.providerId}:${target.voice ?? ""}`;

/** Wird gerufen, wenn eine Karten-Hoerprobe startet (z. B. um die Anbieter-Probestimme zu beenden). */
const tryStartHooks = new Set<() => void>();
export const onTryStart = (hook: () => void): (() => void) => {
  tryStartHooks.add(hook);
  return () => tryStartHooks.delete(hook);
};

const cancelRunning = () => {
  seq += 1;
  const requestId = state?.requestId;
  if (requestId) void commands.ttsProviderCancel(requestId).catch(() => null);
  set(null);
};

/** Laufende Karten-Hoerprobe beenden (Anbieter-Probestimme startet). */
export const stopTry = () => {
  if (state) cancelRunning();
};

let wired = false;
const wire = () => {
  if (wired) return;
  wired = true;
  onSampleStart(() => {
    if (state) cancelRunning();
  });
  void events.ttsProviderPreview.listen(({ payload }) => {
    if (
      payload.phase !== "done" &&
      payload.phase !== "cancelled" &&
      payload.phase !== "error"
    ) {
      return;
    }
    const failure = payload.phase === "error" ? (payload.message ?? "") : null;
    if (state?.requestId === payload.request_id) {
      if (failure) toast.error(failure);
      set(null);
      return;
    }
    // Der Start ist noch nicht beantwortet: fuer die Antwort merken.
    finished.set(payload.request_id, failure);
  });
};

/** Probe starten; laeuft sie fuer denselben Eintrag schon, stoppt der Aufruf sie. */
export async function toggleTry(target: VoiceTryTarget, text: string) {
  const key = tryKey(target);
  const wasRunning = state?.key === key;
  cancelRunning();
  stopSample();
  if (wasRunning) return;
  tryStartHooks.forEach((h) => h());
  wire();
  const mine = seq;
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
      if (mine !== seq) return;
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
    const requestId = result.data;
    if (mine !== seq) {
      void commands.ttsProviderCancel(requestId).catch(() => null);
      return;
    }
    if (finished.has(requestId)) {
      const failure = finished.get(requestId);
      finished.clear();
      if (failure) toast.error(failure);
      set(null);
      return;
    }
    set({ key, requestId });
  } catch (error) {
    if (mine === seq) set(null);
    toast.error(
      error instanceof Error ? error.message : asTtsError(error).message,
    );
  }
}
