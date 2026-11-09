import React, { useState } from "react";
import { useTranslation } from "react-i18next";

/** Vorgaben des Sprachflusses; gleiche Werte wie `SpeechFlow` im Backend. */
export const FLOW_PRESETS = [
  { value: 0, key: "flowBrisk" },
  { value: 0.5, key: "flowAudiobook" },
  { value: 1, key: "flowCalm" },
] as const;

interface FlowControlProps {
  /** Sprachfluss 0 (zügig) bis 1 (ruhig). */
  value: number;
  onChange(value: number): void;
  /** Zusätzliche Klassen für den Container. */
  className?: string;
}

/**
 * Sprachfluss: drei Vorgaben und ein Schieberegler zum Feinjustieren der
 * Pausen an Komma, Satz- und Absatzende.
 */
export const FlowControl: React.FC<FlowControlProps> = ({
  value,
  onChange,
  className = "",
}) => {
  const { t } = useTranslation();
  // Beim Ziehen nur lokal mitführen; gespeichert wird beim Loslassen (sonst
  // ein Einstellungsschreiben je Rasterschritt).
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? value;
  const commit = () => {
    if (draft === null) return;
    setDraft(null);
    if (draft !== value) onChange(draft);
  };
  return (
    <div className={`flex flex-col gap-2 ${className}`} data-flow-control>
      <div
        role="group"
        aria-label={t("tts.settings.flow")}
        className="grid grid-cols-3 gap-1"
      >
        {FLOW_PRESETS.map((preset) => {
          const active = Math.abs(shown - preset.value) < 0.025;
          return (
            <button
              key={preset.value}
              type="button"
              aria-pressed={active}
              onClick={() => onChange(preset.value)}
              className={`h-8 rounded px-2 text-sm cursor-pointer whitespace-nowrap hover:bg-mid-gray/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-text ${
                active
                  ? "bg-logo-primary/15 text-text font-medium"
                  : "text-text/80"
              }`}
              data-testid={`flow-preset-${preset.value}`}
            >
              {t(`tts.settings.${preset.key}`)}
            </button>
          );
        })}
      </div>
      <input
        type="range"
        min={0}
        max={1}
        step={0.05}
        value={shown}
        onChange={(e) => setDraft(parseFloat(e.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        aria-label={t("tts.settings.flow")}
        title={t("tts.settings.flowHint")}
        aria-valuetext={`${Math.round(shown * 100)} %`}
        className="w-full cursor-pointer accent-logo-primary"
        data-testid="flow-slider"
      />
    </div>
  );
};
