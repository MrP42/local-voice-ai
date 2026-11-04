import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Play, SkipForward, Square, TextSearch } from "lucide-react";
import { commands, type MeetingSpeaker, type StoredSegment } from "@/bindings";
import {
  formatShare,
  formatSpeechTime,
  shareRows,
  speakerKey,
  speakerSamples,
  type SpeakerSample,
} from "@/lib/meetingSpeakerShares";
import { IconButton } from "../../ui/IconButton";
import { PersonNameField } from "./PersonNameField";
import { speakerErrorText } from "./SpeakerPopover";
import type { TranscriptPlayer } from "./transcriptPlayer";

interface SpeakerSharesTableProps {
  meetingId: string;
  speakers: MeetingSpeaker[];
  segments: StoredSegment[];
  player: TranscriptPlayer;
  /** Nach dem Speichern eines Namens (Sprecher, Segmente, Dokumente neu laden). */
  onChanged: () => void;
  /** Die Stelle im Transkript zeigen und ab dort abspielen. */
  onJump: (segmentIndex: number) => void;
}

/** Touchziel 44 px: Fluent begrenzt Icon-Buttons sonst auf 40 px (lg). */
const TOUCH: React.CSSProperties = {
  width: 44,
  height: 44,
  minWidth: 44,
  maxWidth: 44,
};

/** Läuft gerade: welcher Sprecher, welche Probe. */
interface Playing {
  key: string;
  sample: number;
}

/**
 * #242: Tabelle "Sprecher & Redeanteile" im Protokoll. Je Zeile Hörproben
 * (typische Ausschnitte über die ganze Besprechung, nacheinander; "Weiter"
 * springt zur nächsten, "Im Transkript zeigen" springt an die Stelle) und der
 * Name direkt in der Zeile. Der Name gilt besprechungsweit: Transkript,
 * Protokoll, KI-Notizen, Fragen und Export lesen ihn aus der zentralen
 * Sprecherzuordnung (Kanal, Nummer -> Name).
 */
export const SpeakerSharesTable: React.FC<SpeakerSharesTableProps> = ({
  meetingId,
  speakers,
  segments,
  player,
  onChanged,
  onJump,
}) => {
  const { t, i18n } = useTranslation();
  const rows = useMemo(() => shareRows(speakers), [speakers]);
  const samples = useMemo(() => {
    const map = new Map<string, SpeakerSample[]>();
    for (const speaker of rows) {
      map.set(speakerKey(speaker), speakerSamples(segments, speaker));
    }
    return map;
  }, [rows, segments]);

  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [savedKey, setSavedKey] = useState<string | null>(null);
  /** Nächste Probe je Sprecher (nach "Weiter" und am Ende einer Probe). */
  const [cursor, setCursor] = useState<Record<string, number>>({});
  const [playing, setPlaying] = useState<Playing | null>(null);
  const playingRef = useRef<Playing | null>(null);
  const playerRef = useRef(player);
  playerRef.current = player;
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Ansicht verlassen: keine Probe läuft weiter.
  useEffect(
    () => () => {
      if (playingRef.current) {
        playingRef.current = null;
        playerRef.current.stop();
      }
      if (savedTimer.current) clearTimeout(savedTimer.current);
    },
    [],
  );

  const setRunning = (next: Playing | null) => {
    playingRef.current = next;
    setPlaying(next);
  };

  /** Spielt Probe `index` und danach die folgenden, bis die letzte endet. */
  const playFrom = (key: string, index: number) => {
    const list = samples.get(key) ?? [];
    const sample = list[index];
    if (!sample) {
      setRunning(null);
      return;
    }
    const run = { key, sample: index };
    setRunning(run);
    setCursor((old) => ({ ...old, [key]: index }));
    playerRef.current.playRange(
      sample.start_ms,
      sample.end_ms,
      sample.channel,
      (completed) => {
        const current = playingRef.current;
        // Abgelöst (andere Probe, Stopp, Weiter): nichts tun.
        if (!current || current.key !== key || current.sample !== index) return;
        // Von außen angehalten (Pause, Sprung im Player): Kette beenden.
        if (!completed) {
          setRunning(null);
          return;
        }
        if (index + 1 < list.length) {
          playFrom(key, index + 1);
        } else {
          setRunning(null);
          setCursor((old) => ({ ...old, [key]: 0 }));
        }
      },
    );
  };

  const togglePlay = (key: string) => {
    if (playingRef.current?.key === key) {
      setRunning(null);
      playerRef.current.stop();
      return;
    }
    playFrom(key, cursor[key] ?? 0);
  };

  const next = (key: string) => {
    const total = samples.get(key)?.length ?? 0;
    if (total === 0) return;
    const current =
      playingRef.current?.key === key
        ? playingRef.current.sample
        : (cursor[key] ?? 0);
    playFrom(key, (current + 1) % total);
  };

  const jump = (key: string) => {
    const list = samples.get(key) ?? [];
    const index =
      playingRef.current?.key === key
        ? playingRef.current.sample
        : (cursor[key] ?? 0);
    const sample = list[index] ?? list[0];
    if (!sample) return;
    if (playingRef.current) {
      setRunning(null);
      playerRef.current.stop();
    }
    onJump(sample.segment_index);
  };

  const save = async (speaker: MeetingSpeaker, value: string) => {
    const key = speakerKey(speaker);
    const trimmed = value.trim();
    if (trimmed === (speaker.display_name ?? "")) {
      setDrafts((old) => {
        const copy = { ...old };
        delete copy[key];
        return copy;
      });
      return;
    }
    setBusy(key);
    setErrors((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });
    try {
      const result = await commands.meetingSpeakerRename(
        meetingId,
        speaker.channel,
        speaker.speaker_index,
        trimmed === "" ? null : trimmed,
      );
      if (result.status === "error") {
        setErrors((old) => ({
          ...old,
          [key]: speakerErrorText(result.error, t),
        }));
        return;
      }
    } catch {
      setErrors((old) => ({ ...old, [key]: speakerErrorText("generic", t) }));
      return;
    } finally {
      setBusy(null);
    }
    setDrafts((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });
    setSavedKey(key);
    if (savedTimer.current) clearTimeout(savedTimer.current);
    savedTimer.current = setTimeout(() => setSavedKey(null), 2500);
    onChanged();
  };

  const cancel = (key: string) =>
    setDrafts((old) => {
      const copy = { ...old };
      delete copy[key];
      return copy;
    });

  if (rows.length === 0) return null;

  return (
    <div className="@container mt-2" data-speaker-shares="">
      <p className="mb-2 text-xs text-text/60">
        {player.canSeek
          ? t("meetings.speakers.table.hint")
          : t("meetings.speakers.table.hintNoAudio")}
      </p>
      <div role="table" aria-label={t("meetings.speakers.table.label")}>
        <div
          role="row"
          className="hidden border-b border-mid-gray/30 pb-1 text-sm font-semibold @xl:grid @xl:grid-cols-[minmax(0,1fr)_11.5rem_5rem_5rem] @xl:gap-3"
        >
          <span role="columnheader">
            {t("meetings.speakers.table.speaker")}
          </span>
          <span role="columnheader">{t("meetings.speakers.table.listen")}</span>
          <span role="columnheader">{t("meetings.speakers.table.time")}</span>
          <span role="columnheader">{t("meetings.speakers.table.share")}</span>
        </div>
        {rows.map((speaker) => {
          const key = speakerKey(speaker);
          const list = samples.get(key) ?? [];
          const isPlaying = playing?.key === key;
          const shownSample = isPlaying ? playing.sample : (cursor[key] ?? 0);
          const draft = drafts[key] ?? speaker.display_name ?? "";
          const time = formatSpeechTime(speaker.speech_ms);
          const share = formatShare(speaker.share_pct, i18n.language);
          const label = speaker.label;
          return (
            <div
              key={key}
              role="row"
              data-testid="speaker-share-row"
              data-speaker={key}
              data-copy-name={label}
              data-copy-time={time}
              data-copy-share={share}
              className="grid grid-cols-[11.5rem_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-b border-mid-gray/20 py-2 @xl:grid-cols-[minmax(0,1fr)_11.5rem_5rem_5rem]"
            >
              <div role="cell" className="col-span-3 min-w-0 @xl:col-span-1">
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    void save(speaker, draft);
                  }}
                  onBlur={(e) => {
                    // Fokus bleibt in der Zeile (Treffer der Personenliste):
                    // noch nicht speichern.
                    if (
                      e.currentTarget.contains(e.relatedTarget as Node | null)
                    )
                      return;
                    if (drafts[key] !== undefined) void save(speaker, draft);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Escape" && drafts[key] !== undefined) {
                      e.stopPropagation();
                      cancel(key);
                    }
                  }}
                  className="flex min-w-0 items-center gap-2"
                >
                  <PersonNameField
                    value={draft}
                    onChange={(value) =>
                      setDrafts((old) => ({ ...old, [key]: value }))
                    }
                    onPick={(person) => {
                      setDrafts((old) => ({ ...old, [key]: person.name }));
                      void save(speaker, person.name);
                    }}
                    placeholder={speaker.default_label}
                    ariaLabel={t("meetings.speakers.table.nameFor", {
                      label: speaker.default_label,
                    })}
                    disabled={busy === key}
                    inputTestId="speaker-share-name"
                    inputClassName="[&>input]:min-h-[44px]"
                    exclude={speaker.display_name ? [speaker.display_name] : []}
                    className="min-w-0 flex-1"
                  />
                  {savedKey === key && (
                    <Check
                      width={16}
                      height={16}
                      aria-label={t("meetings.speakers.table.saved")}
                      className="shrink-0 text-logo-primary"
                      data-testid="speaker-share-saved"
                    />
                  )}
                </form>
                {errors[key] && (
                  <p role="alert" className="mt-1 text-xs text-red-400">
                    {errors[key]}
                  </p>
                )}
              </div>
              <div role="cell" className="flex items-center gap-1">
                <IconButton
                  size="lg"
                  variant={isPlaying ? "secondary" : "ghost"}
                  style={TOUCH}
                  disabled={!player.canSeek || list.length === 0}
                  aria-pressed={isPlaying}
                  icon={
                    isPlaying ? (
                      <Square
                        width={16}
                        height={16}
                        fill="currentColor"
                        aria-hidden="true"
                      />
                    ) : (
                      <Play width={16} height={16} aria-hidden="true" />
                    )
                  }
                  label={
                    isPlaying
                      ? t("meetings.speakers.table.stopFor", { label })
                      : t("meetings.speakers.table.playFor", { label })
                  }
                  data-testid="speaker-share-play"
                  onClick={() => togglePlay(key)}
                />
                <IconButton
                  size="lg"
                  style={TOUCH}
                  disabled={!player.canSeek || list.length < 2}
                  icon={
                    <SkipForward width={16} height={16} aria-hidden="true" />
                  }
                  label={t("meetings.speakers.table.nextFor", { label })}
                  data-testid="speaker-share-next"
                  onClick={() => next(key)}
                />
                <IconButton
                  size="lg"
                  style={TOUCH}
                  disabled={list.length === 0}
                  icon={
                    <TextSearch width={16} height={16} aria-hidden="true" />
                  }
                  label={t("meetings.speakers.table.jumpFor", { label })}
                  data-testid="speaker-share-jump"
                  onClick={() => jump(key)}
                />
                <span
                  className="w-9 text-center text-xs tabular-nums text-text/60"
                  aria-live="polite"
                  data-testid="speaker-share-counter"
                >
                  {list.length > 0
                    ? t("meetings.speakers.table.sampleOf", {
                        n: Math.min(shownSample + 1, list.length),
                        total: list.length,
                      })
                    : ""}
                </span>
              </div>
              <div
                role="cell"
                className="text-end text-sm whitespace-nowrap tabular-nums @xl:text-start"
                data-testid="speaker-share-time"
              >
                <span className="sr-only @xl:hidden">
                  {t("meetings.speakers.table.time")}:{" "}
                </span>
                {time}
              </div>
              <div
                role="cell"
                className="text-end text-sm tabular-nums @xl:text-start"
                data-testid="speaker-share-pct"
              >
                {share}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

/**
 * Für die Zwischenablage: die Tabelle als einfache HTML-Tabelle wie im
 * gespeicherten Protokoll (ohne Felder und Knöpfe), mit den aktuellen Namen.
 */
export function staticSharesTable(
  live: Element,
  headers: { speaker: string; time: string; share: string },
): HTMLTableElement {
  const doc = live.ownerDocument;
  const table = doc.createElement("table");
  const head = table.createTHead().insertRow();
  for (const text of [headers.speaker, headers.time, headers.share]) {
    const th = doc.createElement("th");
    th.textContent = text;
    head.appendChild(th);
  }
  const body = table.createTBody();
  live.querySelectorAll("[data-copy-name]").forEach((row) => {
    const tr = body.insertRow();
    for (const attr of [
      "data-copy-name",
      "data-copy-time",
      "data-copy-share",
    ]) {
      tr.insertCell().textContent = row.getAttribute(attr) ?? "";
    }
  });
  return table;
}
