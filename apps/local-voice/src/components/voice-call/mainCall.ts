import { emitTo, listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { subscribeCallLevel, useVoiceCallStore } from "@/stores/voiceCallStore";
import type { CallState } from "./engine";

// Hauptgespräch per Tastenkürzel (auch außerhalb der App): Das Backend meldet nur
// den Tastendruck (`voice-call-hotkey`). Das Gespräch selbst läuft hier im
// Hauptfenster, auch wenn es verborgen ist; das Overlay zeigt wie beim Diktat,
// ob zugehört, nachgedacht oder gesprochen wird. Es wird nichts in ein Feld
// eingefügt: die Antwort kommt über die Stimme, der Text steht im Gesprächsverlauf.

/** Ereignis des Backends: das Tastenkürzel „Hauptgespräch“ wurde gedrückt. */
export const HOTKEY_EVENT = "voice-call-hotkey";
/** Ereignis des Overlays: Knopf „Beenden“ am Gesprächsstatus. */
export const END_EVENT = "voice-call-end";

export type CallOverlayState =
  "call-listening" | "call-hearing" | "call-thinking" | "call-speaking";

/** Rein: Overlay-Zustand zum Zustand des Anrufs; `null` = ausblenden. */
export function overlayStateFor(
  state: CallState,
  mainCall: boolean,
): CallOverlayState | null {
  if (!mainCall) return null;
  switch (state) {
    case "listening":
      return "call-listening";
    case "hearing":
      return "call-hearing";
    case "starting":
    case "thinking":
      return "call-thinking";
    case "speaking":
      return "call-speaking";
    default:
      return null;
  }
}

/** Die Wellenanzeige des Overlays erwartet 16 Bänder (wie beim Diktat). */
const BAND_SHAPE = [
  0.55, 0.8, 1, 0.9, 0.7, 1, 0.85, 0.6, 0.75, 0.95, 0.65, 0.8, 0.5, 0.7, 0.6,
  0.5,
];
/** Höchstens ~30 Bilder je Sekunde ans Overlay (wie das Backend beim Diktat). */
const LEVEL_INTERVAL_MS = 33;

/** Pegel des Gesprächs als Bänder fürs Overlay (0…1). */
export function levelBands(level: number): number[] {
  const v = Math.min(1, Math.max(0, level * 8));
  return BAND_SHAPE.map((k) => v * k);
}

/**
 * Hört auf das Tastenkürzel und hält das Overlay im Takt des Anrufs. Einmal im
 * Hauptfenster aufrufen; gibt die Abmeldung zurück.
 */
export function installMainCallBridge(): () => void {
  const store = useVoiceCallStore;
  let shown: CallOverlayState | null = null;
  let lastLevel = 0;

  const sync = () => {
    const { state, mainCall } = store.getState();
    const next = overlayStateFor(state, mainCall);
    if (next === shown) return;
    shown = next;
    void commands.voiceCallOverlay(next).catch(() => {
      /* ohne Overlay läuft das Gespräch trotzdem */
    });
  };

  const unsubscribeStore = store.subscribe(sync);
  const unsubscribeLevel = subscribeCallLevel((level) => {
    if (shown !== "call-listening" && shown !== "call-hearing") return;
    const now = performance.now();
    if (now - lastLevel < LEVEL_INTERVAL_MS) return;
    lastLevel = now;
    void emitTo("recording_overlay", "mic-level", levelBands(level)).catch(
      () => {},
    );
  });
  const unlistenHotkey = listen(HOTKEY_EVENT, () => {
    void store.getState().toggleMainCall();
  });
  const unlistenEnd = listen(END_EVENT, () => {
    const s = store.getState();
    if (s.mainCall && s.state !== "idle") void s.stopCall();
  });

  return () => {
    unsubscribeStore();
    unsubscribeLevel();
    void unlistenHotkey.then((fn) => fn());
    void unlistenEnd.then((fn) => fn());
    if (shown) void commands.voiceCallOverlay(null).catch(() => {});
  };
}
