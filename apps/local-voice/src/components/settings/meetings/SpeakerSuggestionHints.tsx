import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { MeetingSpeaker, NameSuggestion } from "@/bindings";
import { Button } from "../../ui/Button";
import { acceptNameSuggestion } from "./useNameSuggestions";
import { speakerErrorText } from "./SpeakerPopover";

const formatMmSs = (ms: number) => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(totalSeconds / 60)}:${(totalSeconds % 60)
    .toString()
    .padStart(2, "0")}`;
};

interface SpeakerSuggestionHintsProps {
  meetingId: string;
  suggestions: NameSuggestion[];
  speakers: MeetingSpeaker[];
  /** Verwerfen (dauerhaft). */
  onDismiss: (suggestion: NameSuggestion) => void;
  /** Nach dem Übernehmen: Segmente und Sprecher neu laden. */
  onAccepted: () => void;
  /** Sprung zur Belegstelle (nur mit Audio). */
  onSeek?: (ms: number, channel: number) => void;
}

/**
 * U8: Hinweise "Person 2 ist vermutlich André (3 Belege)" mit Übernehmen und
 * Verwerfen. Die Belege (Sätze, in denen die Person angesprochen wurde) stehen
 * hinter einem Knopf, damit der Hinweis eine Zeile bleibt. Nichts wird ohne
 * Klick übernommen.
 */
export const SpeakerSuggestionHints: React.FC<SpeakerSuggestionHintsProps> = ({
  meetingId,
  suggestions,
  speakers,
  onDismiss,
  onAccepted,
  onSeek,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  if (suggestions.length === 0) return null;

  const labelOf = (s: NameSuggestion) =>
    speakers.find(
      (p) => p.channel === s.channel && p.speaker_index === s.speaker_index,
    )?.label ?? t("meetings.speakers.title");

  const accept = async (s: NameSuggestion, key: string) => {
    setBusy(key);
    const error = await acceptNameSuggestion(meetingId, s);
    setBusy(null);
    if (error) {
      toast.error(speakerErrorText(error, t));
      return;
    }
    onAccepted();
  };

  return (
    <ul
      aria-label={t("meetings.speakers.suggestion.label")}
      data-testid="name-suggestions"
      className="space-y-1.5"
    >
      {suggestions.map((s) => {
        const key = `${s.channel}:${s.speaker_index}`;
        const expanded = open === key;
        return (
          <li
            key={key}
            data-testid="name-suggestion"
            data-speaker={key}
            className="rounded-md border border-logo-primary/40 bg-logo-primary/10 px-2 py-1.5 text-sm"
          >
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span
                className="w-full min-w-0"
                data-testid="name-suggestion-text"
              >
                {t("meetings.speakers.suggestion.hint", {
                  label: labelOf(s),
                  name: s.name,
                  count: s.evidence.length,
                })}
              </span>
              <Button
                size="sm"
                disabled={busy === key}
                onClick={() => void accept(s, key)}
                data-testid="name-suggestion-accept"
              >
                {t("meetings.speakers.suggestion.accept")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={busy === key}
                onClick={() => onDismiss(s)}
                data-testid="name-suggestion-dismiss"
              >
                {t("meetings.speakers.suggestion.dismiss")}
              </Button>
              <button
                type="button"
                aria-expanded={expanded}
                onClick={() => setOpen(expanded ? null : key)}
                data-testid="name-suggestion-evidence-toggle"
                className="cursor-pointer text-xs text-text/60 underline hover:text-text"
              >
                {expanded
                  ? t("meetings.speakers.suggestion.hideEvidence")
                  : t("meetings.speakers.suggestion.showEvidence")}
              </button>
            </div>
            {expanded && (
              <ul
                className="mt-1.5 space-y-1 border-s-2 border-mid-gray/30 ps-2 text-xs text-text/80"
                data-testid="name-suggestion-evidence"
              >
                {s.evidence.map((e) => (
                  <li key={e.segment_index} className="flex gap-2">
                    {onSeek ? (
                      <button
                        type="button"
                        onClick={() => onSeek(e.start_ms, s.channel)}
                        title={t("meetings.detail.playFrom")}
                        className="w-10 shrink-0 cursor-pointer text-start tabular-nums hover:text-logo-primary hover:underline"
                      >
                        {formatMmSs(e.start_ms)}
                      </button>
                    ) : (
                      <span className="w-10 shrink-0 tabular-nums">
                        {formatMmSs(e.start_ms)}
                      </span>
                    )}
                    <span className="min-w-0 break-words">
                      {t("meetings.speakers.suggestion.quote", {
                        text: e.quote,
                      })}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
};
