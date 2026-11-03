import React, { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";

interface Pending {
  version: number;
  resolve: (ok: boolean) => void;
}

/**
 * Rückfrage vor dem Neu-Erzeugen eines von Hand bearbeiteten Protokolls (oder
 * Dokumenttyps). Neu erzeugen überschreibt nichts: es entsteht eine neue
 * Version, die bearbeitete bleibt als ältere Version erhalten. Gefragt wird
 * trotzdem, damit niemand seine Korrekturen still aus dem Blick verliert.
 *
 * `confirm(meetingId, kind)` liefert `true`, wenn erzeugt werden darf: sofort,
 * wenn die neueste Version unbearbeitet ist (oder sich nicht laden lässt),
 * sonst nach der Antwort im Dialog. `dialog` gehört einmal in die Ansicht.
 */
export function useRegenerateGuard() {
  const { t } = useTranslation();
  const [pending, setPending] = useState<Pending | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  pendingRef.current = pending;

  const confirm = useCallback(
    async (meetingId: string, kind: string): Promise<boolean> => {
      const result = await commands.meetingsGetDocuments(meetingId);
      if (result.status !== "ok") return true;
      const latest = result.data
        .filter((d) => d.kind === kind)
        .sort((a, b) => b.version - a.version)[0];
      if (!latest?.edited) return true;
      // Eine offene Rückfrage wird verneint, bevor die neue kommt.
      pendingRef.current?.resolve(false);
      return new Promise<boolean>((resolve) =>
        setPending({ version: latest.version, resolve }),
      );
    },
    [],
  );

  const answer = (ok: boolean) => {
    pending?.resolve(ok);
    setPending(null);
  };

  const dialog = (
    <Dialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open) answer(false);
      }}
      title={t("meetings.minutes.regenerateEdited.title")}
      closeLabel={t("meetings.actions.close")}
      contentFades={false}
      footer={
        <>
          <Button
            variant="primary"
            data-testid="regenerate-edited-go"
            onClick={() => answer(true)}
          >
            {t("meetings.minutes.regenerateEdited.go")}
          </Button>
          <Button
            variant="secondary"
            data-testid="regenerate-edited-cancel"
            onClick={() => answer(false)}
          >
            {t("meetings.detail.cancel")}
          </Button>
        </>
      }
    >
      <p className="text-sm" data-testid="regenerate-edited-dialog">
        {t("meetings.minutes.regenerateEdited.body", {
          version: pending?.version ?? 0,
        })}
      </p>
    </Dialog>
  );

  return { confirm, dialog };
}
