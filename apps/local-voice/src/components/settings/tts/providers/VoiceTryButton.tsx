import React from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Play, Square } from "lucide-react";
import {
  toggleTry,
  tryKey,
  useTryingKey,
  type VoiceTryTarget,
} from "./voiceTry";
import { DEFAULT_PREVIEW_KEY } from "./providerModel";

/**
 * Reinhoeren in eine Stimme ohne fertige Hoerprobe: erzeugt beim Klick einen
 * kurzen Probesatz und spielt ihn (Kreisel, bis das Audio kommt). Zweiter Klick
 * stoppt. Waehlt die Stimme NICHT aus -- wie `SamplePlayButton inline`.
 */
export const VoiceTryButton: React.FC<{
  target: VoiceTryTarget;
  name: string;
  testId?: string;
}> = ({ target, name, testId }) => {
  const { t } = useTranslation();
  const trying = useTryingKey() === tryKey(target);
  const label = trying
    ? t("tts.providers.voices.sample.stop", { name })
    : t("tts.providers.voices.sample.play", { name });
  const stop = (e: React.SyntheticEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  return (
    <span
      role="button"
      tabIndex={-1}
      aria-label={label}
      title={label}
      data-testid={testId}
      className={`inline-flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded hover:bg-mid-gray/20 hover:text-text ${
        trying ? "text-logo-primary" : "text-text/60"
      }`}
      onMouseDown={stop}
      onPointerDown={stop}
      onClick={(e) => {
        stop(e);
        void toggleTry(target, t(DEFAULT_PREVIEW_KEY));
      }}
    >
      {trying ? (
        <Square width={14} height={14} aria-hidden="true" />
      ) : (
        <Play width={14} height={14} aria-hidden="true" />
      )}
    </span>
  );
};
