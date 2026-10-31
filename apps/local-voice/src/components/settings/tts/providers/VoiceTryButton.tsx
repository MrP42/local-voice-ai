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

const MEDIA_STYLE = {
  "--_mb-size": "44px",
  "--_mb-ico": "1.1rem",
} as React.CSSProperties;

/**
 * Reinhoeren in eine Stimme ohne fertige Hoerprobe: erzeugt beim Klick einen
 * kurzen Probesatz und spielt ihn (Kreisel, bis das Audio kommt). Zweiter Klick
 * stoppt. Waehlt die Stimme NICHT aus -- wie `SamplePlayButton inline`.
 */
export const VoiceTryButton: React.FC<{
  target: VoiceTryTarget;
  name: string;
  testId?: string;
  /** Runder Wiedergabeknopf der Stimmenverwaltung (44 px, `.mbtn`). */
  media?: boolean;
}> = ({ target, name, testId, media = false }) => {
  const { t } = useTranslation();
  const trying = useTryingKey() === tryKey(target);
  const label = trying
    ? t("tts.providers.voices.sample.stop", { name })
    : t("tts.providers.voices.sample.play", { name });
  const stop = (e: React.SyntheticEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  const icon = trying ? (
    <Square
      width={media ? undefined : 14}
      height={media ? undefined : 14}
      aria-hidden="true"
    />
  ) : (
    <Play
      width={media ? undefined : 14}
      height={media ? undefined : 14}
      aria-hidden="true"
    />
  );
  const run = (e: React.SyntheticEvent) => {
    stop(e);
    void toggleTry(target, t(DEFAULT_PREVIEW_KEY));
  };
  if (media) {
    // Eigener Knopf der Verwaltung: echtes <button> (Tab, Enter, Leertaste gratis).
    return (
      <button
        type="button"
        aria-label={label}
        title={label}
        data-testid={testId}
        style={MEDIA_STYLE}
        className="mbtn mbtn--primary"
        onMouseDown={stop}
        onPointerDown={stop}
        onClick={run}
      >
        {icon}
      </button>
    );
  }
  // Inline in einer Zeile, die selbst ein Knopf ist: kein verschachteltes <button>.
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
      onClick={run}
    >
      {icon}
    </span>
  );
};
