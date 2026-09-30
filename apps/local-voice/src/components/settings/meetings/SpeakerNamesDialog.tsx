import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
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

  const save = async (speaker: MeetingSpeaker, value: string) => {
    const key = keyOf(speaker);
    setBusy(key);
    setErrors((old) => {
      const next = { ...old };
      delete next[key];
      return next;
    });
    const trimmed = value.trim();
    const result = await commands.meetingSpeakerRename(
      meetingId,
      speaker.channel,
      speaker.speaker_index,
      trimmed === "" ? null : trimmed,
    );
    setBusy(null);
    if (result.status === "error") {
      setErrors((old) => ({
        ...old,
        [key]: speakerErrorText(result.error, t),
      }));
      return;
    }
    setDrafts((old) => {
      const next = { ...old };
      delete next[key];
      return next;
    });
    onChanged();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.speakers.dialog.title")}
      description={t("meetings.speakers.dialog.hint")}
      closeLabel={t("meetings.speakers.dialog.close")}
      footer={
        <Button onClick={() => onOpenChange(false)}>
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
                      disabled={busy === key}
                      inputTestId="speakers-dialog-input"
                      exclude={
                        speaker.display_name ? [speaker.display_name] : []
                      }
                      className="min-w-0 flex-1"
                    />
                    <Button
                      type="submit"
                      size="sm"
                      disabled={busy === key || !changed}
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
                        onClick={() =>
                          player.seek(info.sample.start_ms, speaker.channel)
                        }
                        title={t("meetings.speakers.dialog.listenHint")}
                        aria-label={t("meetings.speakers.dialog.listenFor", {
                          label: speaker.label,
                        })}
                        data-testid="speakers-dialog-listen"
                        className="mt-0.5"
                      >
                        <Play width={12} height={12} aria-hidden="true" />
                        {t("meetings.speakers.dialog.listen")}
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
