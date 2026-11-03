import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type Meeting } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { formatMeetingDate } from "@/lib/meetingDate";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { translateMeetingError } from "../meetingErrors";
import { PartOrderList, type OrderItem } from "./PartOrderList";

const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** Wie im Backend (`parts::chronological`): Beginn, dann Dateiname mit Zahlen als Zahlen. */
export const chronologicalOrder = (meetings: Meeting[]): Meeting[] => {
  const collator = new Intl.Collator(undefined, { numeric: true });
  return meetings.slice().sort((a, b) => {
    const ka = a.started_at ?? a.created_at;
    const kb = b.started_at ?? b.created_at;
    if (ka !== kb) return ka - kb;
    const byName = collator.compare(
      baseName(a.source_path ?? ""),
      baseName(b.source_path ?? ""),
    );
    return byName !== 0 ? byName : a.id.localeCompare(b.id);
  });
};

/** Warum diese Auswahl nicht zusammengefuehrt werden kann (i18n-Schluessel), sonst `null`. */
export const mergeBlocker = (meetings: Meeting[]): string | null => {
  if (meetings.length < 2) return "meetings.parts.merge.needTwo";
  if (meetings.some((m) => m.status !== "ready"))
    return "meetings.parts.merge.notReady";
  if (meetings.some((m) => m.source !== "import" && m.source !== "live"))
    return "meetings.parts.merge.noAudio";
  if (new Set(meetings.map((m) => m.source)).size > 1)
    return "meetings.parts.merge.mixed";
  return null;
};

/**
 * Bestaetigung "Zu einer Besprechung zusammenfuehren": Reihenfolge (vorgeschlagen
 * nach Aufnahmebeginn, mit Pfeilen aenderbar) und was mit den Eintraegen
 * geschieht. Die erste Besprechung bleibt und nimmt die anderen auf.
 */
export const MergeDialog: React.FC<{
  meetings: Meeting[] | null;
  onClose: () => void;
  onMerged: (id: string) => void;
}> = ({ meetings, onClose, onMerged }) => {
  const { t, i18n } = useTranslation();
  const [order, setOrder] = useState<OrderItem[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!meetings) return;
    setOrder(
      chronologicalOrder(meetings).map((m) => {
        const when = new Date((m.started_at ?? m.created_at) * 1000);
        const minutes =
          m.duration_ms !== null
            ? t("meetings.header.minutes", {
                count: Math.max(1, Math.round(m.duration_ms / 60000)),
              })
            : null;
        return {
          id: m.id,
          label: m.source_path ? baseName(m.source_path) : m.title,
          detail: [
            m.source_path ? m.title : null,
            formatMeetingDate(when, i18n.language, "compact"),
            minutes,
          ]
            .filter(Boolean)
            .join(" · "),
        };
      }),
    );
  }, [meetings, t, i18n.language]);

  const blocker = meetings ? mergeBlocker(meetings) : null;
  const first = meetings?.find((m) => m.id === order[0]?.id);

  const confirm = async () => {
    if (blocker || order.length < 2) return;
    setBusy(true);
    try {
      const result = await commands.meetingsMerge(
        order.map((o) => o.id),
        true,
      );
      if (result.status === "error") {
        toast.error(translateMeetingError(result.error, t));
        return;
      }
      toast.success(t("meetings.parts.merge.done", { count: order.length }));
      notifyMeetingsChanged();
      onMerged(result.data);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={meetings !== null}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      title={t("meetings.parts.merge.title")}
      closeLabel={t("meetings.consent.cancel")}
      footer={
        <>
          <Button
            onClick={() => void confirm()}
            disabled={busy || blocker !== null}
            data-testid="merge-confirm"
            className="max-sm:min-h-[44px]"
          >
            {busy
              ? t("meetings.parts.merge.running")
              : t("meetings.parts.merge.confirm", { count: order.length })}
          </Button>
          <Button
            variant="secondary"
            onClick={onClose}
            disabled={busy}
            className="max-sm:min-h-[44px]"
          >
            {t("meetings.consent.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="merge-dialog">
        <p className="text-sm text-text/80">{t("meetings.parts.merge.body")}</p>
        <PartOrderList items={order} onChange={setOrder} testId="merge-order" />
        {first && (
          <p className="text-sm" data-testid="merge-target">
            {t("meetings.parts.merge.keeps", { title: first.title })}
          </p>
        )}
        <p className="text-xs text-text/60">{t("meetings.parts.merge.loss")}</p>
        {blocker && (
          <p className="text-sm text-red-400" data-testid="merge-blocker">
            {t(blocker)}
          </p>
        )}
      </div>
    </Dialog>
  );
};
