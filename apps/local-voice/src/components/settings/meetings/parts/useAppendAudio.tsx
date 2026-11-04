import React, { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands, type Meeting } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { translateMeetingError } from "../meetingErrors";
import { PartOrderList, type OrderItem } from "./PartOrderList";
import "./parts.css";

/** Audio und Video (Untertitel haben keine Tonspur zum Anhaengen). */
export const APPEND_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "mp4",
  "mkv",
  "mov",
  "flac",
  "ogg",
];

const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** Kann an diese Besprechung Audio angehaengt werden? */
export const canAppendAudio = (meeting: Meeting | null): boolean =>
  !!meeting &&
  ((meeting.source === "import" && meeting.status !== "recording") ||
    (meeting.source === "live" && meeting.status === "ready"));

/**
 * "Audiodatei anhängen …": Dateiauswahl, Einwilligung und Reihenfolge, dann
 * kommt jede Datei als Teil in die Warteschlange. Ist der letzte Teil fertig,
 * haengt das Backend alle hinten an die Besprechung an (fortlaufende Zeitachse).
 * Auch fuer eine Ablage von Dateien auf eine gewaehlte Besprechung nutzbar
 * (`ask(paths)`).
 */
export function useAppendAudio(meeting: Meeting | null) {
  const { t } = useTranslation();
  const [items, setItems] = useState<OrderItem[] | null>(null);
  const [busy, setBusy] = useState(false);

  const ask = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    setItems(paths.map((p) => ({ id: p, label: baseName(p) })));
  }, []);

  const pick = useCallback(async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Audio", extensions: APPEND_EXTENSIONS }],
    });
    const paths = Array.isArray(picked)
      ? picked
      : typeof picked === "string"
        ? [picked]
        : [];
    ask(paths);
  }, [ask]);

  const confirm = async () => {
    if (!meeting || !items) return;
    setBusy(true);
    try {
      const result = await commands.meetingsAppendFiles(
        meeting.id,
        items.map((i) => i.id),
        true,
      );
      if (result.status === "error") {
        toast.error(translateMeetingError(result.error, t));
        return;
      }
      toast.success(
        t("meetings.parts.append.queued", {
          count: items.length,
          title: meeting.title,
        }),
      );
      setItems(null);
      notifyMeetingsChanged();
    } finally {
      setBusy(false);
    }
  };

  const dialog = (
    <Dialog
      className="parts-dialog-touch"
      contentFades={false}
      open={items !== null}
      onOpenChange={(isOpen) => {
        if (!isOpen && !busy) setItems(null);
      }}
      title={t("meetings.parts.append.title")}
      closeLabel={t("meetings.consent.cancel")}
      footer={
        <>
          <Button
            onClick={() => void confirm()}
            disabled={busy}
            data-testid="append-confirm"
            className="max-sm:min-h-[44px]"
          >
            {t("meetings.parts.append.confirm")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => setItems(null)}
            disabled={busy}
            className="max-sm:min-h-[44px]"
          >
            {t("meetings.consent.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="append-dialog">
        <p className="text-sm text-text/80">
          {t("meetings.parts.append.body", { title: meeting?.title ?? "" })}
        </p>
        {items && (
          <PartOrderList
            items={items}
            onChange={setItems}
            testId="append-order"
          />
        )}
        <p className="whitespace-pre-wrap text-xs text-text/60">
          {t("meetings.consent.importBody")}
        </p>
      </div>
    </Dialog>
  );

  return { pick, ask, dialog };
}
