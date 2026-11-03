import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Layers } from "lucide-react";
import { toast } from "sonner";
import { commands, type PartsInfo } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { translateMeetingError } from "../meetingErrors";
import { formatPartTime } from "./useMeetingParts";

const CHIP =
  "inline-flex h-6 shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-mid-gray/30 px-2 text-xs text-text/80 hover:bg-mid-gray/15 cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60";

const ROW_BTN =
  "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-start text-sm hover:bg-mid-gray/15 max-sm:min-h-[44px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60";

/** Text des Status eines wartenden Teils. */
const pendingStatusKey = (status: string): string => {
  switch (status) {
    case "queued":
      return "meetings.parts.pending.queued";
    case "processing":
      return "meetings.parts.pending.processing";
    case "ready":
      return "meetings.parts.pending.ready";
    case "failed":
      return "meetings.parts.pending.failed";
    case "cancelled":
      return "meetings.parts.pending.cancelled";
    default:
      return "meetings.parts.pending.processing";
  }
};

/** Gibt es etwas zu zeigen (mehrere Teile oder noch laufende)? */
export const hasParts = (info: PartsInfo): boolean =>
  info.parts.length > 1 || info.pending.length > 0;

const partCount = (info: PartsInfo) =>
  Math.max(info.parts.length, 1) + info.pending.length;

/**
 * Ausloeser der Teilliste. `chip`: im Kopf neben der Dauer (ab 640 px);
 * `row`: eigene Zeile mit Touchziel >= 44 px unter 640 px.
 */
export const PartsTrigger: React.FC<{
  info: PartsInfo;
  variant: "chip" | "row";
  onOpen: () => void;
}> = ({ info, variant, onOpen }) => {
  const { t } = useTranslation();
  if (!hasParts(info)) return null;
  const label =
    info.pending.length > 0
      ? t("meetings.parts.chipPending", {
          count: partCount(info),
          pending: info.pending.length,
        })
      : t("meetings.parts.chip", { count: partCount(info) });
  return (
    <button
      type="button"
      className={
        variant === "chip"
          ? `${CHIP} max-sm:hidden`
          : "flex min-h-[44px] w-full items-center gap-2 rounded-md border border-mid-gray/30 px-3 text-start text-sm text-text/80 hover:bg-mid-gray/15 sm:hidden focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
      }
      data-testid={variant === "chip" ? "parts-chip" : "parts-row-trigger"}
      onClick={onOpen}
      title={t("meetings.parts.chipHint")}
    >
      <Layers
        width={variant === "chip" ? 12 : 16}
        height={variant === "chip" ? 12 : 16}
        aria-hidden="true"
      />
      {label}
    </button>
  );
};

/**
 * Teilliste einer Besprechung aus mehreren Dateien: jeder Teil mit Beginn auf
 * der Zeitachse (Klick spielt ab dort), noch laufende Teile mit Zustand; ein
 * gescheiterter Teil kann geloest werden, damit die anderen nicht warten.
 */
export const PartsDialog: React.FC<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  info: PartsInfo;
  onSeek?: (ms: number) => void;
  onChanged: () => void;
}> = ({ open, onOpenChange, meetingId, info, onSeek, onChanged }) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const setOpen = onOpenChange;

  const allReady =
    info.pending.length > 0 && info.pending.every((p) => p.status === "ready");

  const detach = async (id: string) => {
    setBusy(true);
    try {
      const result = await commands.meetingsAppendDetach(id);
      if (result.status === "error")
        toast.error(translateMeetingError(result.error, t));
      notifyMeetingsChanged();
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  const mergeNow = async () => {
    setBusy(true);
    try {
      const result = await commands.meetingsMerge(
        [meetingId, ...info.pending.map((p) => p.meeting_id)],
        true,
      );
      if (result.status === "error")
        toast.error(translateMeetingError(result.error, t));
      notifyMeetingsChanged();
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={setOpen}
      title={t("meetings.parts.title")}
      closeLabel={t("meetings.parts.close")}
      footer={
        <Button
          variant="secondary"
          onClick={() => setOpen(false)}
          className="max-sm:min-h-[44px]"
        >
          {t("meetings.parts.close")}
        </Button>
      }
    >
      <div className="space-y-3" data-testid="parts-dialog">
        {info.parts.length > 0 && (
          <ol className="space-y-0.5" aria-label={t("meetings.parts.title")}>
            {info.parts.map((part) => (
              <li key={part.part_index}>
                <button
                  type="button"
                  className={ROW_BTN}
                  data-testid="parts-row"
                  disabled={!onSeek}
                  onClick={() => {
                    onSeek?.(part.offset_ms);
                    setOpen(false);
                  }}
                  title={t("meetings.parts.playFrom")}
                >
                  <span className="w-14 shrink-0 text-xs tabular-nums text-text/60">
                    {formatPartTime(part.offset_ms)}
                  </span>
                  <span className="min-w-0 flex-1 truncate">
                    {t("meetings.parts.partLabel", {
                      n: part.part_index + 1,
                      title: part.title,
                    })}
                  </span>
                  <span className="shrink-0 text-xs tabular-nums text-text/60">
                    {formatPartTime(part.duration_ms)}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        )}
        {info.pending.length > 0 && (
          <div className="space-y-1" data-testid="parts-pending">
            <p className="text-xs font-semibold text-text/70">
              {t("meetings.parts.pendingTitle")}
            </p>
            <ul className="space-y-0.5">
              {info.pending.map((p) => (
                <li
                  key={p.meeting_id}
                  className="flex items-center gap-2 px-2 py-1 text-sm"
                  data-testid="parts-pending-row"
                >
                  <span className="min-w-0 flex-1 truncate">{p.title}</span>
                  <span className="shrink-0 text-xs text-text/60">
                    {t(pendingStatusKey(p.status))}
                  </span>
                  {(p.status === "failed" || p.status === "cancelled") && (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => void detach(p.meeting_id)}
                      title={t("meetings.parts.detachHint")}
                      className="max-sm:min-h-[44px]"
                    >
                      {t("meetings.parts.detach")}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
            <p className="text-xs text-text/60">
              {t("meetings.parts.pendingHint")}
            </p>
            {allReady && (
              <Button
                size="sm"
                disabled={busy}
                onClick={() => void mergeNow()}
                data-testid="parts-merge-now"
                className="max-sm:min-h-[44px]"
              >
                {t("meetings.parts.mergeNow")}
              </Button>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
};

/**
 * Hinweis in einer Besprechung, die selbst ein wartender Teil ist: sie wird
 * nach ihrer Verarbeitung an eine andere angehaengt.
 */
export const AppendNotice: React.FC<{
  meetingId: string;
  target: [string, string];
  onChanged: () => void;
}> = ({ meetingId, target, onChanged }) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  return (
    <div
      className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-logo-primary/10 px-3 py-1.5 text-sm"
      data-testid="append-notice"
      role="status"
    >
      <span>{t("meetings.parts.appendNotice", { title: target[1] })}</span>
      <Button
        size="sm"
        variant="ghost"
        disabled={busy}
        className="max-sm:min-h-[44px]"
        onClick={async () => {
          setBusy(true);
          try {
            const result = await commands.meetingsAppendDetach(meetingId);
            if (result.status === "error")
              toast.error(translateMeetingError(result.error, t));
            notifyMeetingsChanged();
            onChanged();
          } finally {
            setBusy(false);
          }
        }}
      >
        {t("meetings.parts.keepSeparate")}
      </Button>
    </div>
  );
};
