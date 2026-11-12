import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Scissors } from "lucide-react";
import { toast } from "sonner";
import { commands, type Meeting } from "@/bindings";
import {
  coversAll,
  formatRangeTime,
  fullRange,
  recordingDurationMs,
  type MsRange,
} from "@/lib/meetingRange";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { AudioRangePicker } from "./AudioRangePicker";
import { translateMeetingError } from "./meetingErrors";

interface TrimAudioDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meeting: Meeting;
  /** Nach dem Zuschneiden: der Aufrufer lädt Segmente und Aufnahme neu. */
  onTrimmed: () => void;
}

/**
 * Aufnahme zuschneiden (lva#522): behält nur den gewählten Bereich der Audiodateien.
 * ENDGÜLTIG: das Audio außerhalb wird gelöscht, Transkriptabschnitte außerhalb entfallen
 * (auch in älteren Fassungen), die Zeitstempel beginnen danach bei 0:00. Deshalb ein
 * Warnhinweis, der nie ausgeblendet wird (`contentFades={false}`), und ein Gefahrenknopf;
 * erst dessen Klick ruft `meetings_trim_audio`.
 *
 * Nur sinnvoll mit Audio auf der Platte; das Menü bietet den Eintrag sonst nicht an.
 */
export const TrimAudioDialog: React.FC<TrimAudioDialogProps> = ({
  open,
  onOpenChange,
  meeting,
  onTrimmed,
}) => {
  const { t } = useTranslation();
  const durationMs = recordingDurationMs(meeting.duration_ms);
  const [range, setRange] = useState<MsRange>(fullRange(durationMs ?? 0));
  const [running, setRunning] = useState(false);

  // Beim Öffnen: ganze Aufnahme gewählt (nichts geht verloren, bis man es ändert).
  useEffect(() => {
    if (open) {
      setRange(fullRange(durationMs ?? 0));
      setRunning(false);
    }
  }, [open, durationMs]);

  const nothingToCut = durationMs === null || coversAll(range, durationMs);

  const trim = async () => {
    if (nothingToCut || running) return;
    setRunning(true);
    try {
      const result = await commands.meetingsTrimAudio(
        meeting.id,
        range.start_ms,
        range.end_ms,
      );
      if (result.status === "error") {
        toast.error(translateMeetingError(result.error, t));
        return;
      }
      toast.success(
        t("meetings.trim.done", { length: formatRangeTime(result.data) }),
      );
      onOpenChange(false);
      onTrimmed();
    } finally {
      setRunning(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.trim.title")}
      description={meeting.title}
      closeLabel={t("meetings.trim.close")}
      // Der Verlusthinweis darf nicht unter einer Ausblendkante verschwinden.
      contentFades={false}
      dismissible={!running}
      footer={
        <>
          <Button
            variant="danger"
            className="max-sm:min-h-[44px]"
            onClick={() => void trim()}
            disabled={nothingToCut || running}
            data-testid="trim-start"
          >
            <Scissors width={14} height={14} aria-hidden="true" />
            {t("meetings.trim.submit")}
          </Button>
          <Button
            variant="secondary"
            className="max-sm:min-h-[44px]"
            onClick={() => onOpenChange(false)}
            disabled={running}
            data-testid="trim-cancel"
          >
            {t("meetings.trim.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="trim-dialog">
        <p className="text-sm text-text/80">{t("meetings.trim.intro")}</p>
        {durationMs !== null && (
          <AudioRangePicker
            durationMs={durationMs}
            value={range}
            onChange={setRange}
            testId="trim-range"
            disabled={running}
            lengthKey="meetings.trim.newLength"
          />
        )}
        {nothingToCut && (
          <p className="text-sm text-text/60" data-testid="trim-nothing">
            {t("meetings.trim.nothing")}
          </p>
        )}
        <Alert variant="warning">
          <span data-testid="trim-warning">{t("meetings.trim.warning")}</span>
        </Alert>
      </div>
    </Dialog>
  );
};
