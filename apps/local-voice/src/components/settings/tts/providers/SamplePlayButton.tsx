import React from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Play, Square } from "lucide-react";
import { toggleSample, usePlayingSample } from "@/lib/voices/samplePlayer";

const MEDIA_STYLE = {
  "--_mb-size": "44px",
  "--_mb-ico": "1.1rem",
} as React.CSSProperties;

/**
 * Reinhoeren in eine Stimme: spielt ihre fertige Hoerprobe (oder stoppt sie).
 * `path === null`: die Probe entsteht noch im Hintergrund (Kreisel, gesperrt).
 *
 * `inline`: fuer Listen, deren Eintrag selbst schon ein Knopf ist (Dropdown,
 * Menue der Wiedergabeleiste) -- dann ein `span` mit Knopf-Rolle, und der Klick
 * waehlt die Stimme NICHT aus, er spielt nur.
 */
export const SamplePlayButton: React.FC<{
  path: string | null;
  name: string;
  inline?: boolean;
  testId?: string;
  /** Runder Wiedergabeknopf der Stimmenverwaltung (44 px, `.mbtn`). */
  media?: boolean;
}> = ({ path, name, inline = false, testId, media = false }) => {
  const { t } = useTranslation();
  const playing = usePlayingSample();
  const active = path !== null && playing === path;
  const label =
    path === null
      ? t("tts.providers.voices.sample.pending", { name })
      : active
        ? t("tts.providers.voices.sample.stop", { name })
        : t("tts.providers.voices.sample.play", { name });
  const Icon = path === null ? Loader2 : active ? Square : Play;
  const icon = (
    <Icon
      width={media ? undefined : 14}
      height={media ? undefined : 14}
      aria-hidden="true"
      className={path === null ? "animate-spin opacity-60" : undefined}
    />
  );
  const className = media
    ? `mbtn mbtn--primary ${path === null ? "opacity-70" : ""}`
    : `inline-flex h-6 w-6 shrink-0 items-center justify-center rounded ${
        active ? "text-logo-primary" : "text-text/60"
      } ${path === null ? "cursor-default" : "cursor-pointer hover:bg-mid-gray/20 hover:text-text"}`;
  const style = media ? MEDIA_STYLE : undefined;
  const stop = (e: React.SyntheticEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  const onClick = (e: React.SyntheticEvent) => {
    stop(e);
    if (path) toggleSample(path);
  };

  if (inline) {
    return (
      <span
        role="button"
        tabIndex={-1}
        aria-label={label}
        aria-disabled={path === null}
        title={label}
        data-testid={testId}
        className={className}
        style={style}
        // Die Auswahl der Liste reagiert schon auf das Druecken der Maustaste.
        onMouseDown={stop}
        onPointerDown={stop}
        onClick={onClick}
      >
        {icon}
      </span>
    );
  }
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={path === null}
      data-testid={testId}
      className={className}
      style={style}
      onClick={onClick}
    >
      {icon}
    </button>
  );
};
