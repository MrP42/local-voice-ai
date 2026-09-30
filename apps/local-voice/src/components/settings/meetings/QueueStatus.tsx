import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUpToLine, X } from "lucide-react";
import { commands } from "@/bindings";
import { QUEUE_ERROR_KEYS, type QueuePlace } from "@/lib/meetingQueue";
import { Alert } from "../../ui/Alert";
import { IconAction } from "../../ui/IconAction";

/** Wo die Datei steht: "Wartet · Platz 2 von 3" (ohne Grund). */
export const useQueuePlaceText = () => {
  const { t } = useTranslation();
  return (place: QueuePlace) =>
    t("meetings.queue.place", {
      position: place.position,
      total: place.total,
    });
};

/** Der Grund, warum die Schlange steht (Text), oder `null`, wenn sie gleich beginnt. */
export const useQueueReasonText = () => {
  const { t } = useTranslation();
  return (place: QueuePlace): string | null =>
    place.reason ? t(`meetings.queue.reason.${place.reason}`) : null;
};

/**
 * Kleine Marke einer wartenden Datei in der Liste: "Wartet · Platz 2 von 3".
 * Der Grund steht im Tooltip-Text (`title`) und in der Bedienspalte.
 */
export const QueueChip: React.FC<{ place: QueuePlace }> = ({ place }) => {
  const placeText = useQueuePlaceText();
  const reasonText = useQueueReasonText();
  const reason = reasonText(place);
  return (
    <span
      data-testid="queue-chip"
      data-position={place.position}
      data-reason={place.reason ?? undefined}
      title={reason ?? undefined}
      className="inline-flex items-center rounded-full bg-mid-gray/20 px-2 text-[11px] font-medium text-text/70"
    >
      {placeText(place)}
    </span>
  );
};

/** Fehlertext eines Warteschlangen-Befehls. */
const useQueueError = () => {
  const { t } = useTranslation();
  return (code: string) => {
    const key = QUEUE_ERROR_KEYS[code.split(":")[0].trim()];
    return key ? t(key) : code;
  };
};

/**
 * Bedienspalte einer wartenden Datei (statt des Fortschrittsbalkens): der
 * Platz, der Grund des Wartens und zwei Symbole, "Nach vorn ziehen" und "Aus der
 * Warteschlange nehmen". Die Datei selbst bleibt unberuehrt; "Fortsetzen" im
 * abgebrochenen Zustand stellt sie wieder hinten an.
 */
export const QueuePanel: React.FC<{
  meetingId: string;
  place: QueuePlace;
}> = ({ meetingId, place }) => {
  const { t } = useTranslation();
  const placeText = useQueuePlaceText();
  const reasonText = useQueueReasonText();
  const describe = useQueueError();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (
    call: () => Promise<{ status: "ok" | "error"; error?: unknown }>,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await call();
      if (result.status === "error") setError(describe(String(result.error)));
    } finally {
      setBusy(false);
    }
  };

  const reason = reasonText(place);
  return (
    <div
      data-testid="queue-panel"
      data-position={place.position}
      data-reason={place.reason ?? undefined}
      className="space-y-1 rounded-md border border-mid-gray/20 px-2.5 py-1.5"
    >
      <div className="flex items-center gap-2">
        <span
          className="min-w-0 flex-1 truncate text-sm font-medium"
          data-testid="queue-place"
        >
          {placeText(place)}
        </span>
        <IconAction
          icon={ArrowUpToLine}
          label={t("meetings.queue.toFront")}
          description={t("meetings.queue.toFrontHint")}
          testId="queue-to-front"
          disabled={busy || place.position === 1}
          onClick={() =>
            void run(() => commands.meetingsQueueToFront(meetingId))
          }
        />
        <IconAction
          icon={X}
          label={t("meetings.queue.remove")}
          description={t("meetings.queue.removeHint")}
          testId="queue-remove"
          iconClassName="text-red-500"
          disabled={busy}
          onClick={() =>
            void run(() => commands.meetingsQueueRemove(meetingId))
          }
        />
      </div>
      <p className="text-xs text-text/60" data-testid="queue-reason">
        {reason ?? t("meetings.queue.startsSoon")}
      </p>
      {error && (
        <div data-testid="queue-error">
          <Alert variant="error">{error}</Alert>
        </div>
      )}
    </div>
  );
};
