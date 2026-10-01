import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play, Square } from "lucide-react";
import {
  commands,
  type MeetingSpeaker,
  type NameSuggestion,
  type StoredSegment,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { PersonNameField } from "./PersonNameField";
import { speakerErrorText } from "./SpeakerPopover";
import type { TranscriptPlayer } from "./transcriptPlayer";

interface SpeakerNamesDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  speakers: MeetingSpeaker[];
  segments: StoredSegment[];
  suggestions: NameSuggestion[];
  onDismissSuggestion: (suggestion: NameSuggestion) => void;
  player: TranscriptPlayer;
  /** Nach jedem Speichern: Segmente und Sprecher neu laden. */
  onChanged: () => void;
}

const keyOf = (s: { channel: number; speaker_index: number }) =>
  `${s.channel}-${s.speaker_index}`;

/** Laenger als das spielt die Stimmprobe nie (Millisekunden). */
const MAX_SAMPLE_MS = 15000;

/**
 * U8: "Sprecher benennen …" aus dem Menü: alle Sprecher der Besprechung auf
 * einen Blick, je Zeile Name (mit Vorschlägen aus den bekannten Personen),
 * Anzahl Sätze und ein Sprung zu einer Hörprobe. Der Name gilt überall:
 * Transkript, Notizen, Protokoll, Export und Personen.
 */
export const SpeakerNamesDialog: React.FC<SpeakerNamesDialogProps> = ({
  open,
  onOpenChange,
  meetingId,
  speakers,
  segments,
  suggestions,
  onDismissSuggestion,
  player,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  /** Der Sprecher, dessen Stimmprobe gerade laeuft. */
  const [playing, setPlaying] = useState<string | null>(null);
  const playingRef = useRef<string | null>(null);
  // Der Player wird bei jedem Rendern der Detailansicht neu gebaut; fuer das
  // Stoppen beim Schliessen zaehlt der aktuelle.
  const playerRef = useRef(player);
  playerRef.current = player;
  const [closing, setClosing] = useState(false);

  /** Haelt die eigene Stimmprobe an (nie eine fremde Wiedergabe). */
  const stopSample = () => {
    if (playingRef.current === null) return;
    playingRef.current = null;
    setPlaying(null);
    playerRef.current.stop();
  };

  // Ein frisch geoeffneter Dialog zeigt den gespeicherten Stand, keine alten
  // Entwuerfe; geschlossen (auch von aussen) laeuft keine Probe weiter.
  useEffect(() => {
    if (open) {
      setDrafts({});
      setErrors({});
      return;
    }
    if (playingRef.current !== null) {
      playingRef.current = null;
      setPlaying(null);
      playerRef.current.stop();
    }
  }, [open]);
  useEffect(
    () => () => {
      if (playingRef.current !== null) playerRef.current.stop();
    },
    [],
  );

  const listen = (speaker: MeetingSpeaker, sample: StoredSegment) => {
    const key = keyOf(speaker);
    if (playingRef.current === key) {
      stopSample();
      return;
    }
    playingRef.current = key;
    setPlaying(key);
    playerRef.current.playRange(
      sample.start_ms,
      Math.min(sample.end_ms, sample.start_ms + MAX_SAMPLE_MS),
      speaker.channel,
      () => {
        if (playingRef.current === key) playingRef.current = null;
        setPlaying((current) => (current === key ? null : current));
      },
    );
  };

  /** Sätze und Hörprobe (der längste Satz) je Sprecher. */
  const stats = useMemo(() => {
    const map = new Map<string, { count: number; sample: StoredSegment }>();
    for (const segment of segments) {
      if (segment.speaker_index === null) continue;
      const key = keyOf({
        channel: segment.channel,
        speaker_index: segment.speaker_index,
      });
      const entry = map.get(key);
      const length = segment.end_ms - segment.start_ms;
      if (!entry) {
        map.set(key, { count: 1, sample: segment });
      } else {
        entry.count += 1;
        if (length > entry.sample.end_ms - entry.sample.start_ms) {
          entry.sample = segment;
        }
      }
    }
    return map;
  }, [segments]);

  /**
   * Speichert einen Namen. `true` bei Erfolg; bei einem Fehler steht die
   * Meldung an der Zeile und der Entwurf bleibt. Neu laden ist Sache des
   * Aufrufers (einmal je Aktion, nicht je Zeile).
   */
  const persist = async (
    speaker: MeetingSpeaker,
    value: string,
  ): Promise<boolean> => {
    const key = keyOf(speaker);
    setBusy(key);
    setErrors((old) => {
      const next = { ...old };
      delete next[key];
      return next;
    });
    const trimmed = value.trim();
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
        return false;
      }
    } catch {
      setErrors((old) => ({
        ...old,
        [key]: speakerErrorText("generic", t),
      }));
      return false;
    } finally {
      setBusy(null);
    }
    setDrafts((old) => {
      const next = { ...old };
      delete next[key];
      return next;
    });
    return true;
  };

  const save = async (speaker: MeetingSpeaker, value: string) => {
    if (await persist(speaker, value)) onChanged();
  };

  /** Eingegebene, noch nicht gespeicherte Namen. */
  const pending = () =>
    speakers.filter((speaker) => {
      const draft = drafts[keyOf(speaker)];
      return (
        draft !== undefined && draft.trim() !== (speaker.display_name ?? "")
      );
    });

  /**
   * Schliessen (Fertig, Kreuz, Escape, Klick daneben): eingegebene Namen werden
   * uebernommen, nicht verworfen. Schlaegt ein Speichern fehl, bleibt der Dialog
   * offen und zeigt den Fehler an der Zeile.
   */
  const requestClose = async () => {
    if (closing) return;
    stopSample();
    const dirty = pending();
    if (dirty.length === 0) {
      onOpenChange(false);
      return;
    }
    setClosing(true);
    let ok = true;
    let saved = false;
    for (const speaker of dirty) {
      const result = await persist(speaker, drafts[keyOf(speaker)] ?? "");
      ok = ok && result;
      saved = saved || result;
    }
    setClosing(false);
    if (saved) onChanged();
    if (ok) onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) onOpenChange(true);
        else void requestClose();
      }}
      title={t("meetings.speakers.dialog.title")}
      description={t("meetings.speakers.dialog.hint")}
      closeLabel={t("meetings.speakers.dialog.close")}
      footer={
        <Button disabled={closing} onClick={() => void requestClose()}>
          {t("meetings.speakers.dialog.done")}
        </Button>
      }
    >
      <div className="space-y-3" data-testid="speakers-dialog">
        {speakers.length === 0 ? (
          <p
            className="text-sm text-text/70"
            data-testid="speakers-dialog-empty"
          >
            {t("meetings.speakers.dialog.empty")}
          </p>
        ) : (
          <ul className="space-y-3">
            {speakers.map((speaker) => {
              const key = keyOf(speaker);
              const info = stats.get(key);
              const suggestion =
                suggestions.find(
                  (s) =>
                    s.channel === speaker.channel &&
                    s.speaker_index === speaker.speaker_index,
                ) ?? null;
              const draft = drafts[key] ?? speaker.display_name ?? "";
              const changed = draft.trim() !== (speaker.display_name ?? "");
              const isPlaying = playing === key;
              return (
                <li
                  key={key}
                  data-testid="speakers-dialog-row"
                  data-speaker={`${speaker.channel}:${speaker.speaker_index}`}
                  className="space-y-1.5 rounded-md border border-mid-gray/30 p-2"
                >
                  <div className="flex items-baseline justify-between gap-2 text-xs text-text/60">
                    <span className="min-w-0 truncate font-semibold text-text">
                      {speaker.label}
                    </span>
                    <span className="shrink-0 tabular-nums">
                      {t("meetings.speakers.dialog.sentences", {
                        count: info?.count ?? 0,
                      })}
                    </span>
                  </div>
                  <form
                    className="flex items-start gap-2"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void save(speaker, draft);
                    }}
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
                      placeholder={t("meetings.speakers.namePlaceholder")}
                      ariaLabel={t("meetings.speakers.dialog.rowName", {
                        label: speaker.label,
                      })}
                      disabled={busy === key || closing}
                      inputTestId="speakers-dialog-input"
                      exclude={
                        speaker.display_name ? [speaker.display_name] : []
                      }
                      className="min-w-0 flex-1"
                    />
                    <Button
                      type="submit"
                      size="sm"
                      disabled={busy === key || closing || !changed}
                      data-testid="speakers-dialog-save"
                      className="mt-0.5"
                    >
                      {t("meetings.speakers.save")}
                    </Button>
                    {player.canSeek && info && (
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() => listen(speaker, info.sample)}
                        title={t("meetings.speakers.dialog.listenHint")}
                        aria-label={
                          isPlaying
                            ? t("meetings.speakers.dialog.stopFor", {
                                label: speaker.label,
                              })
                            : t("meetings.speakers.dialog.listenFor", {
                                label: speaker.label,
                              })
                        }
                        aria-pressed={isPlaying}
                        data-testid="speakers-dialog-listen"
                        className="mt-0.5"
                      >
                        {isPlaying ? (
                          <Square
                            width={12}
                            height={12}
                            fill="currentColor"
                            aria-hidden="true"
                          />
                        ) : (
                          <Play width={12} height={12} aria-hidden="true" />
                        )}
                        {isPlaying
                          ? t("meetings.speakers.dialog.stop")
                          : t("meetings.speakers.dialog.listen")}
                      </Button>
                    )}
                  </form>
                  {suggestion && (
                    <div
                      data-testid="speakers-dialog-suggestion"
                      className="flex flex-wrap items-center gap-2 rounded-md bg-logo-primary/10 px-2 py-1 text-xs"
                    >
                      <span className="min-w-0 flex-1">
                        {t("meetings.speakers.suggestion.popover", {
                          name: suggestion.name,
                          count: suggestion.evidence.length,
                        })}
                      </span>
                      <Button
                        size="sm"
                        disabled={busy === key}
                        onClick={() => void save(speaker, suggestion.name)}
                        data-testid="speakers-dialog-suggestion-accept"
                      >
                        {t("meetings.speakers.suggestion.accept")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => onDismissSuggestion(suggestion)}
                        data-testid="speakers-dialog-suggestion-dismiss"
                      >
                        {t("meetings.speakers.suggestion.dismiss")}
                      </Button>
                    </div>
                  )}
                  {errors[key] && (
                    <p role="alert" className="text-xs text-red-400">
                      {errors[key]}
                    </p>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </Dialog>
  );
};
