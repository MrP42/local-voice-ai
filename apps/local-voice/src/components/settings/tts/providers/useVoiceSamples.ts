import { useEffect, useSyncExternalStore } from "react";
import { commands, events } from "@/bindings";
import { PROVIDER_VOICES_CHANGED } from "./VisibleVoices";

/** Stimme → Pfad der Hoerprobe (`null` = entsteht noch). */
export type VoiceSamples = Readonly<Record<string, string | null>>;

/**
 * Gemeinsamer Stand der Hoerproben je Anbieter: Einstellungen, Dropdown und
 * Wiedergabeleiste fragen das Backend nur einmal und sehen dieselben Pfade.
 * Fehlende Proben erzeugt das Backend selbst im Hintergrund; jede fertige
 * meldet es mit `tts-provider-voice-sample`, dann erscheint ihr Knopf sofort.
 */
let state: Readonly<Record<string, VoiceSamples>> = {};
const listeners = new Set<() => void>();
const loading = new Set<string>();
const notify = () => listeners.forEach((l) => l());

const load = (providerId: string) => {
  if (loading.has(providerId)) return;
  loading.add(providerId);
  void commands
    .ttsProviderVoiceSamples(providerId)
    .then((result) => {
      if (result.status !== "ok") return;
      const next: Record<string, string | null> = {};
      for (const sample of result.data) {
        // Ein Pfad aus dem Ereignis bleibt, falls die Antwort aelter ist.
        next[sample.voice_id] =
          sample.wav_path ?? state[providerId]?.[sample.voice_id] ?? null;
      }
      state = { ...state, [providerId]: next };
      notify();
    })
    .catch(() => undefined)
    .finally(() => loading.delete(providerId));
};

let wired = false;
const wire = () => {
  if (wired) return;
  wired = true;
  void events.ttsProviderVoiceSample.listen(({ payload }) => {
    state = {
      ...state,
      [payload.provider_id]: {
        ...state[payload.provider_id],
        [payload.voice_id]: payload.wav_path,
      },
    };
    notify();
  });
  const reloadAll = () => Object.keys(state).forEach(load);
  window.addEventListener("lv-voices-changed", reloadAll);
  window.addEventListener(PROVIDER_VOICES_CHANGED, reloadAll);
};

const EMPTY: VoiceSamples = {};

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/**
 * Hoerproben mehrerer Anbieter (Anbieter → Stimme → Pfad). Leer fuer Anbieter
 * ohne Vorratsproben (Cloud, Fish, Piper) und solange die erste Antwort fehlt.
 */
export function useVoiceSamplesOf(
  providerIds: readonly string[],
): Readonly<Record<string, VoiceSamples>> {
  const key = providerIds.join(",");
  useEffect(() => {
    if (!key) return;
    wire();
    key.split(",").forEach(load);
  }, [key]);
  return useSyncExternalStore(subscribe, () => state);
}

/** Hoerproben EINES Anbieters (siehe `useVoiceSamplesOf`). */
export function useVoiceSamples(providerId: string | null): VoiceSamples {
  const all = useVoiceSamplesOf(providerId ? [providerId] : []);
  return (providerId && all[providerId]) || EMPTY;
}
