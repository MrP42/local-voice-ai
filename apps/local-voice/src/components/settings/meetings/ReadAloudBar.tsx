import React, { useEffect, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  AudioLines,
  Check,
  ChevronDown,
  Pause,
  Play,
  Settings,
  SkipBack,
  SkipForward,
  Square,
} from "lucide-react";
import { commands } from "@/bindings";
import { ActionMenu, type ActionMenuItem } from "../../ui/ActionMenu";
import { IconAction } from "../../ui/IconAction";
import { VOICE_MANAGE, useVoiceChoice } from "../tts/useVoiceChoice";
import { SamplePlayButton } from "../tts/providers/SamplePlayButton";
import { VoiceTryButton } from "../tts/providers/VoiceTryButton";
import { counterWidthCh, shownSentenceNumber } from "../tts/sentenceCounter";
import {
  discardSavedReading,
  getReadAloudSession,
  getSavedReading,
  pauseReading,
  resumeReading,
  resumeSavedReading,
  seekReading,
  stopReading,
  subscribeReadAloud,
} from "./readAloudSession";

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

/**
 * Was die Steuerung fuer `speakKey` zeigt: `live` = die globale Sitzung gehoert diesem
 * Inhalt (laeuft oder pausiert), `saved` = eine andere Wiedergabe hat die Sitzung
 * ersetzt, die Stelle ist aber gemerkt (Fortsetzen ab Satz n), sonst nichts.
 */
export const useReadAloudView = (speakKey: string): "live" | "saved" | null => {
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  const savedEntry = useSyncExternalStore(subscribeReadAloud, () =>
    getSavedReading(speakKey),
  );
  if (session?.key === speakKey) return "live";
  return savedEntry ? "saved" : null;
};

const Controls: React.FC<{
  speakKey: string;
  testId: string;
  view: "live" | "saved";
}> = ({ speakKey, testId, view }) => {
  const { t } = useTranslation();
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  const savedEntry = useSyncExternalStore(subscribeReadAloud, () =>
    getSavedReading(speakKey),
  );

  const live = view === "live" ? session : null;
  const playing = !!live && !live.paused && live.runs > 0;
  // Zaehler: der Satz, der zu hoeren ist -- oder der, mit dem es weitergeht.
  const total = live ? (live.progress?.total ?? 0) : (savedEntry?.total ?? 0);
  const index = live
    ? (live.sentence?.index ??
      (live.progress && total > 0 ? live.progress.position : null))
    : (savedEntry?.position ?? null);
  const shown = shownSentenceNumber(index, total);
  const resumePosition = (savedEntry?.position ?? 0) + 1;

  return (
    <div
      role="group"
      aria-label={t("meetings.readAloud.bar.label")}
      data-testid={`${testId}-bar`}
      data-view={view}
      className="inline-flex items-center gap-0.5 rounded-lg border border-mid-gray/20 bg-mid-gray/5 p-0.5"
    >
      <IconAction
        size="sm"
        ghost
        icon={SkipBack}
        label={t("tts.previousSentence")}
        description={t("meetings.readAloud.bar.previousHint")}
        testId={`${testId}-bar-prev`}
        disabled={view === "saved"}
        onClick={() => seekReading(-1)}
      />
      {view === "live" ? (
        <IconAction
          size="sm"
          icon={playing ? Pause : Play}
          label={playing ? t("tts.pause") : t("meetings.readAloud.bar.resume")}
          description={
            playing
              ? t("meetings.readAloud.bar.pauseHint")
              : t("meetings.readAloud.bar.resumeHint")
          }
          testId={`${testId}-bar-toggle`}
          onClick={() => (playing ? pauseReading() : resumeReading())}
        />
      ) : (
        <IconAction
          size="sm"
          icon={Play}
          label={t("meetings.readAloud.bar.savedResume", {
            position: resumePosition,
          })}
          description={t("meetings.readAloud.bar.savedResumeHint")}
          testId={`${testId}-bar-toggle`}
          onClick={() => resumeSavedReading(speakKey)}
        />
      )}
      <IconAction
        size="sm"
        ghost
        icon={SkipForward}
        label={t("tts.nextSentence")}
        description={t("meetings.readAloud.bar.nextHint")}
        testId={`${testId}-bar-next`}
        disabled={view === "saved"}
        onClick={() => seekReading(1)}
      />
      <IconAction
        size="sm"
        ghost
        icon={Square}
        label={t("tts.stop")}
        description={
          view === "live"
            ? t("meetings.readAloud.bar.stopHint")
            : t("meetings.readAloud.bar.savedStopHint")
        }
        testId={`${testId}-bar-stop`}
        onClick={() =>
          view === "live" ? stopReading() : discardSavedReading(speakKey)
        }
      />
      {shown !== null && (
        // Feste Breite (tabular-nums + Mindestbreite nach der Stellenzahl der Gesamtzahl):
        // die Schaltflaeche daneben springt nicht, wenn aus "9/12" "10/12" wird.
        <span
          className="inline-block px-1.5 text-end text-xs tabular-nums text-text/70"
          style={{
            minWidth: `calc(${counterWidthCh(total, (position, count) =>
              t("meetings.readAloud.bar.positionShort", {
                position,
                total: count,
              }),
            )}ch + 0.75rem)`,
          }}
          title={live?.sentence?.text}
          data-testid={`${testId}-bar-position`}
        >
          <span aria-hidden="true">
            {t("meetings.readAloud.bar.positionShort", {
              position: shown,
              total,
            })}
          </span>
          <span className="sr-only">
            {t("meetings.readAloud.bar.position", { position: shown, total })}
          </span>
        </span>
      )}
    </div>
  );
};

/**
 * Die Stimmenwahl der Steuerung, dezent: nur ein Pfeil-Knopf mit Menue, daneben klein der
 * Name der gewaehlten Stimme bzw. des Anbieters. Sie steht als LETZTES der Zeile und waechst
 * auf den Rest der Zeile (`flex-1`, Basis = der Knopf): der Name erscheint nur, wenn dort
 * Platz fuer ihn bleibt (Containerabfrage auf die Gruppe selbst, deren Breite von der Zeile
 * kommt, nicht vom Inhalt); sonst steht er im Tooltip des Knopfes. So bricht die Zeile nie
 * um, nur weil ein Name lang ist.
 */
const Voice: React.FC<{ testId: string }> = ({ testId }) => {
  const { t } = useTranslation();
  const phase = useServerPhase();
  const {
    options,
    voiceValue,
    applyVoiceValue,
    openVoiceManagement,
    selectedVoiceNotSetUp,
    sampleOf,
    tryOf,
  } = useVoiceChoice(phase);

  const voiceLabel = options.find((o) => o.value === voiceValue)?.label ?? "";
  const items: ActionMenuItem[] = options.map((option) =>
    option.value === VOICE_MANAGE
      ? {
          id: option.value,
          label: option.label,
          icon: Settings,
          testId: `${testId}-bar-voice-manage`,
          onSelect: openVoiceManagement,
        }
      : {
          id: option.value,
          label: option.label,
          icon: AudioLines,
          disabled: option.isDisabled,
          trailing: (() => {
            const sample = sampleOf(option.value);
            const check =
              option.value === voiceValue ? (
                <Check width={14} height={14} aria-hidden="true" />
              ) : undefined;
            const target = sample ? undefined : tryOf(option.value);
            if (!sample && !target) return check;
            // Reinhoeren waehlt die Stimme nicht: der Knopf spielt nur die Hoerprobe.
            return (
              <span className="flex items-center gap-1">
                {sample ? (
                  <SamplePlayButton
                    inline
                    path={sample.path}
                    name={sample.name}
                    testId={`${testId}-bar-voice-sample-${option.value}`}
                  />
                ) : (
                  target && (
                    <VoiceTryButton
                      target={target}
                      name={option.label}
                      testId={`${testId}-bar-voice-sample-${option.value}`}
                    />
                  )
                )}
                <span className="inline-flex w-[14px]">{check}</span>
              </span>
            );
          })(),
          // Ein Wechsel gilt sofort: der laufende Satz wird zu Ende gelesen, ab dem naechsten
          // liest die neue Stimme -- dieselbe Wiedergabe, kein Neustart, keine Pause.
          onSelect: () => applyVoiceValue(option.value),
        },
  );

  return (
    <div
      className="@container flex min-w-[28px] flex-[1_1_28px] items-center gap-1"
      data-testid={`${testId}-bar-voice`}
    >
      <ActionMenu
        trigger={{
          icon: ChevronDown,
          label: t("meetings.readAloud.bar.voiceMenu"),
          description: t("meetings.readAloud.bar.voiceMenuHint", {
            name: voiceLabel || "\u2013",
          }),
          testId: `${testId}-bar-voice-menu`,
          size: "sm",
        }}
        items={items}
        menuLabel={t("tts.voices.title")}
        widthClass="w-72"
      />
      <span
        className={`hidden min-w-0 flex-1 truncate text-xs @[5.5rem]:block ${
          selectedVoiceNotSetUp
            ? "text-amber-700 dark:text-amber-400"
            : "text-text/60"
        }`}
        title={
          selectedVoiceNotSetUp ? t("tts.voices.notSetUpHint") : voiceLabel
        }
        data-testid={`${testId}-bar-voice-name`}
        data-not-set-up={selectedVoiceNotSetUp ? "true" : undefined}
      >
        {voiceLabel}
        {selectedVoiceNotSetUp && (
          <span className="sr-only">{t("tts.voices.notSetUpHint")}</span>
        )}
      </span>
    </div>
  );
};

/**
 * Wiedergabe-Steuerung der Aufnahmen, gedacht als Teil der Werkzeugzeile (gleiche Zeile
 * wie Neu erzeugen/Kopieren/Herunterladen, kein eigener Streifen darunter): Satz zurueck,
 * Pause/Weiter, Satz vor, Stopp, Zaehler. Dieselben Bausteine wie die Vorlesen-Seite --
 * Satzsprung, Pause/Weiter; keine eigene Sprachausgabe. Erscheint, solange fuer `speakKey`
 * eine Sitzung besteht (auch pausiert) oder eine Stelle gemerkt ist (`useReadAloudView`).
 */
export const ReadAloudControls: React.FC<{
  speakKey: string;
  testId: string;
}> = ({ speakKey, testId }) => {
  const view = useReadAloudView(speakKey);
  if (!view) return null;
  return <Controls speakKey={speakKey} testId={testId} view={view} />;
};

/** Die Stimmenwahl dazu (`useVoiceChoice`); gehoert ans Ende der Zeile, hinter "Audio speichern". */
export const ReadAloudVoice: React.FC<{
  speakKey: string;
  testId: string;
}> = ({ speakKey, testId }) => {
  const view = useReadAloudView(speakKey);
  if (!view) return null;
  return <Voice testId={testId} />;
};
