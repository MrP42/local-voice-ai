import { useSyncExternalStore } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

/**
 * Eine Hoerprobe zur Zeit, app-weit: ein neuer Klick (Einstellungen, Dropdown,
 * Wiedergabeleiste) stoppt die laufende. Gespielt wird die fertige WAV-Datei
 * ueber das asset-Protokoll -- ohne Synthese, ohne Wartezeit.
 */
let current: { path: string; audio: HTMLAudioElement } | null = null;
const listeners = new Set<() => void>();

const notify = () => listeners.forEach((l) => l());

/** Wird vor jeder neuen Hoerprobe gerufen (z. B. um eine laufende Probestimme zu beenden). */
const startHooks = new Set<() => void>();
export const onSampleStart = (hook: () => void) => {
  startHooks.add(hook);
};

export const stopSample = () => {
  if (!current) return;
  current.audio.pause();
  current = null;
  notify();
};

/** Datei abspielen; laeuft sie schon, stoppt der Aufruf sie (Umschalter). */
export const toggleSample = (path: string) => {
  if (current?.path === path) {
    stopSample();
    return;
  }
  stopSample();
  startHooks.forEach((h) => h());
  const audio = new Audio(convertFileSrc(path, "asset"));
  const entry = { path, audio };
  current = entry;
  const done = () => {
    if (current === entry) {
      current = null;
      notify();
    }
  };
  audio.addEventListener("ended", done);
  audio.addEventListener("error", done);
  notify();
  void audio.play().catch(done);
};

/** Pfad der gerade laufenden Hoerprobe (`null` = keine). */
export const usePlayingSample = (): string | null =>
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current?.path ?? null,
  );
