import { useSyncExternalStore } from "react";
import { LocalVoiceAiMark } from "../icons/LocalVoiceAiLogo";

/** Produktname (Marke, nicht übersetzt). */
export const APP_NAME = "Local Voice AI";

/** Unter dieser Breite wird die Navigation zum Overlay-Drawer (Leitlinie §3). */
export const NARROW_QUERY = "(max-width: 767px)";

export function useIsNarrow(): boolean {
  return useSyncExternalStore(
    (cb) => {
      const mq = window.matchMedia(NARROW_QUERY);
      mq.addEventListener("change", cb);
      return () => mq.removeEventListener("change", cb);
    },
    () => window.matchMedia(NARROW_QUERY).matches,
    () => false,
  );
}

/** App-Symbol der Titelleiste (20px, Markenfarben wie bei Office). */
export const AppMark = () => (
  <LocalVoiceAiMark size={20} className="lv-titlebar__mark" />
);
