import React, { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  commands,
  type MeetingSpeaker,
  type NameSuggestion,
  type StoredSegment,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { PersonNameField } from "./PersonNameField";

/** Fehlercodes der Sprecher-Commands in Nutzertext. */
export const speakerErrorText = (code: string, t: TFunction) => {
  switch (code.split(":")[0].trim()) {
    case "speaker_not_found":
      return t("meetings.speakers.errors.notFound");
    case "speaker_invalid":
      return t("meetings.speakers.errors.invalid");
    case "segment_not_found":
      return t("meetings.speakers.errors.segmentNotFound");
    case "stale_epoch":
      return t("meetings.speakers.errors.staleEpoch");
    default:
      return t("meetings.speakers.errors.failed");
  }
};

const PANEL_WIDTH = 288;
const MARGIN = 8;

interface SpeakerPopoverProps {
  meetingId: string;
  /** Das Segment, dessen Label angeklickt wurde (für "nur dieses Segment"). */
  segment: StoredSegment;
  /** Der Sprecher des Segments. */
  speaker: MeetingSpeaker;
  /** Alle Sprecher der Besprechung. */
  speakers: MeetingSpeaker[];
  /** Epoche, auf der die Segmentnummern gelten; `null` = noch nicht geladen. */
  epoch: number | null;
  /** Nach jeder Änderung: Segmente und Sprecher neu laden. */
  onChanged: () => void;
  /** U8: Namensvorschlag aus dem Gesagten für diesen Sprecher. */
  suggestion?: NameSuggestion | null;
  onDismissSuggestion?: (suggestion: NameSuggestion) => void;
  className?: string;
}

/**
 * M3-P3c: Das Sprecherlabel im Transkript. Ein Klick öffnet ein Popover zum
 * Umbenennen (gilt für alle Segmente des Sprechers), Zusammenführen mit einem
 * anderen Sprecher desselben Kanals und Umhängen des einzelnen Segments.
 */
export const SpeakerPopover: React.FC<SpeakerPopoverProps> = ({
  meetingId,
  segment,
  speaker,
  speakers,
  epoch,
  onChanged,
  suggestion = null,
  onDismissSuggestion,
  className = "",
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [name, setName] = useState(speaker.display_name ?? "");
  const [pendingMerge, setPendingMerge] = useState<MeetingSpeaker | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const sameChannel = speakers.filter((s) => s.channel === speaker.channel);
  const others = sameChannel.filter(
    (s) => s.speaker_index !== speaker.speaker_index,
  );
  const nextIndex = Math.max(0, ...sameChannel.map((s) => s.speaker_index)) + 1;

  const place = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const height = panelRef.current?.offsetHeight ?? 0;
    const left = Math.max(
      MARGIN,
      Math.min(rect.left, window.innerWidth - PANEL_WIDTH - MARGIN),
    );
    const below = rect.bottom + 4;
    const top =
      height > 0 && below + height > window.innerHeight - MARGIN
        ? Math.max(MARGIN, rect.top - height - 4)
        : below;
    setPos({ left, top });
  }, []);

  const close = useCallback(() => {
    setOpen(false);
    setPendingMerge(null);
    setError(null);
  }, []);

  // Position: beim Öffnen, bei Größenänderung des Panels (Merge-Rückfrage),
  // Scrollen und Fenstergröße. Scrollen schließt bewusst nicht: im Feld wird
  // getippt, und das Transkript scrollt selbst.
  useEffect(() => {
    if (!open) return;
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open, place, pendingMerge, error]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        panelRef.current?.contains(target) ||
        triggerRef.current?.contains(target)
      )
        return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        close();
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close]);

  const toggle = () => {
    if (open) {
      close();
      return;
    }
    setName(speaker.display_name ?? "");
    setPendingMerge(null);
    setError(null);
    setOpen(true);
  };

  /** Führt einen Command aus; bei Erfolg neu laden und schließen. */
  const run = async (
    action: () => Promise<
      { status: "ok" } | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    setError(null);
    const result = await action();
    setBusy(false);
    if (result.status === "error") {
      setError(speakerErrorText(result.error, t));
      return;
    }
    close();
    onChanged();
  };

  const rename = (value: string | null) =>
    run(() =>
      commands.meetingSpeakerRename(
        meetingId,
        speaker.channel,
        speaker.speaker_index,
        value,
      ),
    );

  const merge = (target: MeetingSpeaker) =>
    run(() =>
      commands.meetingSpeakerMerge(
        meetingId,
        speaker.channel,
        speaker.speaker_index,
        target.speaker_index,
      ),
    );

  const moveSegment = (index: number) =>
    epoch === null
      ? undefined
      : run(() =>
          commands.meetingSegmentSetSpeaker(
            meetingId,
            segment.segment_index,
            epoch,
            index,
          ),
        );

  const trimmed = name.trim();

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={toggle}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={t("meetings.speakers.open")}
        data-testid="speaker-label"
        data-speaker={`${speaker.channel}:${speaker.speaker_index}`}
        className={`truncate text-start cursor-pointer underline decoration-dotted underline-offset-2 hover:text-logo-primary ${className}`}
      >
        {speaker.label}
      </button>
      {open &&
        createPortal(
          <div
            ref={panelRef}
            role="dialog"
            aria-label={t("meetings.speakers.title")}
            data-testid="speaker-popover"
            className="fixed z-50 space-y-3 rounded-md border border-mid-gray/30 bg-background p-3 text-sm shadow-lg"
            style={{
              width: PANEL_WIDTH,
              left: pos?.left ?? -9999,
              top: pos?.top ?? -9999,
            }}
          >
            <div className="flex items-baseline justify-between gap-2">
              <span className="truncate font-semibold">{speaker.label}</span>
              <span className="shrink-0 text-xs text-text/50">
                {t("meetings.speakers.share", {
                  pct: Math.round(speaker.share_pct),
                })}
              </span>
            </div>

            {suggestion && (
              <div
                data-testid="speaker-suggestion"
                className="space-y-1.5 rounded-md bg-logo-primary/10 p-2 text-xs"
              >
                <p>
                  {t("meetings.speakers.suggestion.popover", {
                    name: suggestion.name,
                    count: suggestion.evidence.length,
                  })}
                </p>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => void rename(suggestion.name)}
                    data-testid="speaker-suggestion-accept"
                  >
                    {t("meetings.speakers.suggestion.accept")}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy}
                    onClick={() => {
                      onDismissSuggestion?.(suggestion);
                      close();
                    }}
                    data-testid="speaker-suggestion-dismiss"
                  >
                    {t("meetings.speakers.suggestion.dismiss")}
                  </Button>
                </div>
              </div>
            )}

            <form
              className="space-y-1.5"
              onSubmit={(e) => {
                e.preventDefault();
                void rename(trimmed === "" ? null : trimmed);
              }}
            >
              <label
                htmlFor={`speaker-name-${meetingId}`}
                className="text-xs text-text/60"
              >
                {t("meetings.speakers.nameLabel")}
              </label>
              <div className="flex items-start gap-2">
                <PersonNameField
                  id={`speaker-name-${meetingId}`}
                  value={name}
                  onChange={setName}
                  onPick={(person) => void rename(person.name)}
                  placeholder={t("meetings.speakers.namePlaceholder")}
                  ariaLabel={t("meetings.speakers.nameLabel")}
                  autoFocus
                  disabled={busy}
                  inputTestId="speaker-name-input"
                  exclude={speaker.display_name ? [speaker.display_name] : []}
                  className="min-w-0 flex-1"
                />
                <Button
                  type="submit"
                  size="sm"
                  disabled={busy}
                  data-testid="speaker-save"
                  className="mt-0.5"
                >
                  {t("meetings.speakers.save")}
                </Button>
              </div>
              {speaker.display_name && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void rename(null)}
                  className="cursor-pointer text-xs text-text/60 underline hover:text-text"
                >
                  {t("meetings.speakers.removeName")}
                </button>
              )}
            </form>

            {others.length > 0 && (
              <div className="space-y-1" data-testid="speaker-merge">
                <p className="text-xs text-text/60">
                  {t("meetings.speakers.mergeTitle")}
                </p>
                {pendingMerge ? (
                  <div
                    className="space-y-2"
                    data-testid="speaker-merge-confirm"
                  >
                    <p className="text-xs text-text/80">
                      {t("meetings.speakers.mergeConfirm", {
                        from: speaker.label,
                        into: pendingMerge.label,
                      })}
                    </p>
                    <div className="flex gap-2">
                      <Button
                        size="sm"
                        disabled={busy}
                        onClick={() => void merge(pendingMerge)}
                        data-testid="speaker-merge-do"
                      >
                        {t("meetings.speakers.mergeDo")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => setPendingMerge(null)}
                      >
                        {t("meetings.speakers.cancel")}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <div className="flex flex-col">
                    {others.map((other) => (
                      <button
                        key={other.speaker_index}
                        type="button"
                        onClick={() => setPendingMerge(other)}
                        data-testid={`speaker-merge-${other.speaker_index}`}
                        className="min-h-[32px] cursor-pointer truncate rounded px-2 text-start hover:bg-mid-gray/15"
                      >
                        {t("meetings.speakers.mergeWith", {
                          label: other.label,
                        })}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}

            <div className="space-y-1" data-testid="speaker-move">
              <p className="text-xs text-text/60">
                {t("meetings.speakers.moveTitle")}
              </p>
              <div className="flex flex-col">
                {others.map((other) => (
                  <button
                    key={other.speaker_index}
                    type="button"
                    disabled={busy || epoch === null}
                    onClick={() => void moveSegment(other.speaker_index)}
                    data-testid={`speaker-move-${other.speaker_index}`}
                    className="min-h-[32px] cursor-pointer truncate rounded px-2 text-start hover:bg-mid-gray/15"
                  >
                    {t("meetings.speakers.moveTo", { label: other.label })}
                  </button>
                ))}
                <button
                  type="button"
                  disabled={busy || epoch === null}
                  onClick={() => void moveSegment(nextIndex)}
                  data-testid="speaker-move-new"
                  className="min-h-[32px] cursor-pointer rounded px-2 text-start hover:bg-mid-gray/15"
                >
                  {t("meetings.speakers.moveNew")}
                </button>
              </div>
            </div>

            {error && (
              <p role="alert" className="text-xs text-red-400">
                {error}
              </p>
            )}
          </div>,
          document.body,
        )}
    </>
  );
};
