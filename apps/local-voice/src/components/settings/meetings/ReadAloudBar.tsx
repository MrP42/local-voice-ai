import React, { useEffect, useId, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { Glyph } from "../../ui/AudioPlayer";
import { Select } from "../../ui/Select";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { VOICE_MANAGE, useVoiceChoice } from "../tts/useVoiceChoice";
import {
  getReadAloudSession,
  pauseReading,
  resumeReading,
  seekReading,
  stopReading,
  subscribeReadAloud,
} from "./readAloudSession";

interface TransportProps {
  glyph: "prev" | "next" | "play" | "pause" | "stop";
  label: string;
  description: string;
  testId: string;
  primary?: boolean;
  disabled?: boolean;
  onClick: () => void;
}

/** Ein Schalter der Transportzeile: runder Glyph-Knopf der Mediabar, Name im Tooltip. */
const Transport: React.FC<TransportProps> = ({
  glyph,
  label,
  description,
  testId,
  primary,
  disabled,
  onClick,
}) => {
  const tipId = `${useId()}-tip`;
  return (
    <TooltipTrigger
      tooltipId={tipId}
      content={
        <>
          <div className="text-sm font-semibold">
            <strong>{label}</strong>
          </div>
          <div className="text-xs text-text/70">{description}</div>
        </>
      }
    >
      <button
        type="button"
        className={`mbtn mbtn--sm${primary ? " mbtn--primary" : ""}`}
        aria-label={label}
        aria-describedby={tipId}
        data-testid={testId}
        disabled={disabled}
        onClick={onClick}
      >
        <Glyph name={glyph} />
      </button>
    </TooltipTrigger>
  );
};

/** Phase des Sprachservers (Fish) -- die Stimmenliste braucht sie fuer "eingerichtet". */
const useServerPhase = () => {
  const [phase, setPhase] = useState("stopped");
  useEffect(() => {
    let alive = true;
    void commands.ttsServerStatus().then((r) => {
      if (alive && r.status === "ok") setPhase(r.data.phase);
    });
    const un = listen<{ phase: string }>("tts-state-changed", (e) =>
      setPhase(e.payload.phase),
    );
    return () => {
      alive = false;
      void un.then((off) => off());
    };
  }, []);
  return phase;
};

const Bar: React.FC<{ testId: string }> = ({ testId }) => {
  const { t } = useTranslation();
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  const phase = useServerPhase();
  const {
    options,
    voiceValue,
    applyVoiceValue,
    openVoiceManagement,
    selectedVoiceNotSetUp,
  } = useVoiceChoice(phase);

  if (!session) return null;
  const playing = session.runs > 0 && !session.paused;
  const total = session.progress?.total ?? 0;
  const index = session.sentence?.index ?? null;
  const position =
    index !== null && total > 0
      ? t("meetings.readAloud.bar.position", {
          position: Math.min(index + 1, total),
          total,
        })
      : null;

  return (
    <div
      className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-mid-gray/30 px-2 py-1"
      data-testid={testId}
      role="group"
      aria-label={t("meetings.readAloud.bar.label")}
    >
      <div className="mediabar mediabar--start" style={{ gap: "0.25rem" }}>
        <Transport
          glyph="prev"
          label={t("tts.previousSentence")}
          description={t("meetings.readAloud.bar.previousHint")}
          testId={`${testId}-prev`}
          onClick={() => seekReading(-1)}
        />
        <Transport
          glyph={playing ? "pause" : "play"}
          label={playing ? t("tts.pause") : t("meetings.readAloud.bar.resume")}
          description={
            playing
              ? t("meetings.readAloud.bar.pauseHint")
              : t("meetings.readAloud.bar.resumeHint")
          }
          testId={`${testId}-toggle`}
          primary
          onClick={() => (playing ? pauseReading() : resumeReading())}
        />
        <Transport
          glyph="next"
          label={t("tts.nextSentence")}
          description={t("meetings.readAloud.bar.nextHint")}
          testId={`${testId}-next`}
          onClick={() => seekReading(1)}
        />
        <Transport
          glyph="stop"
          label={t("tts.stop")}
          description={t("meetings.readAloud.bar.stopHint")}
          testId={`${testId}-stop`}
          onClick={stopReading}
        />
        {position && (
          <span
            className="ms-2 text-xs tabular-nums text-text/70"
            data-testid={`${testId}-position`}
          >
            {position}
          </span>
        )}
      </div>
      <div className="w-56 max-w-full" data-testid={`${testId}-voice`}>
        {/* Dieselbe Auswahl wie auf der Vorlesen-Seite: gleiche Eintraege, gleiche Werte,
            ein Wechsel gilt sofort -- eine laufende Wiedergabe stellt am aktuellen Satz um. */}
        <Select
          menuPortal
          value={voiceValue}
          options={options}
          ariaLabel={t("tts.voices.title")}
          isClearable={false}
          onChange={(value) => {
            if (!value) return;
            if (value === VOICE_MANAGE) {
              openVoiceManagement();
              return;
            }
            applyVoiceValue(value);
          }}
        />
      </div>
      {selectedVoiceNotSetUp && (
        <span
          role="status"
          className="text-xs text-amber-700 dark:text-amber-400"
          data-testid={`${testId}-voice-hint`}
        >
          {t("tts.voices.notSetUpHint")}
        </span>
      )}
      {session.sentence && (
        <p
          className="w-full min-w-0 truncate text-xs text-text/60"
          title={session.sentence.text}
          data-testid={`${testId}-sentence`}
        >
          {session.sentence.text}
        </p>
      )}
    </div>
  );
};

/**
 * Wiedergabeleiste der Aufnahmen: erscheint, solange der Knopf `speakKey` vorliest (oder
 * pausiert hat). Dieselben Bausteine wie die Vorlesen-Seite -- Mediabar-Glyphen,
 * Satzsprung, Pause/Weiter und die Stimmenauswahl (`useVoiceChoice`); keine eigene
 * Sprachausgabe.
 */
export const ReadAloudBar: React.FC<{ speakKey: string; testId: string }> = ({
  speakKey,
  testId,
}) => {
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  if (session?.key !== speakKey) return null;
  return <Bar testId={testId} />;
};
