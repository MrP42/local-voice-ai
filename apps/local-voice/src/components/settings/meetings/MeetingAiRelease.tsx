import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type KiFreigabeStand } from "@/bindings";
import { Button } from "../../ui/Button";

/**
 * G9a-2 (lva#578, ADR-0031 E2.6–E2.8): Freigabe EINER Besprechung für KI-Clients in den
 * Details. Zeigt die Einstufung laut Termin und die Gründe einer Sperre; bestätigt wird nur
 * diese Besprechung mit dieser Teilnehmerliste (keine Sammelbestätigung). „Als intern
 * freigeben“ gibt es nur, wenn die Liste intern ist.
 */
export const MeetingAiRelease: React.FC<{ meetingId: string }> = ({
  meetingId,
}) => {
  const { t } = useTranslation();
  const [stand, setStand] = useState<KiFreigabeStand | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const result = await commands.meetingAiReleaseStatus(meetingId);
    if (result.status === "ok") setStand(result.data);
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!stand) return null;

  const { ergebnis, entscheidung, bestaetigung } = stand;
  const allowed = entscheidung.ergebnis === "erlaubt";
  const reasons =
    entscheidung.ergebnis === "gesperrt" ? entscheidung.gruende : [];

  const errorText = (code: string) =>
    code === "ki_freigabe_veraltet" || code === "ki_freigabe_nicht_intern"
      ? t(`meetings.mcp.release.errors.${code}`)
      : t("meetings.mcp.release.errors.generic");

  const run = async (
    action: () => Promise<
      | { status: "ok"; data: KiFreigabeStand }
      | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      if (result.status === "ok") {
        setStand(result.data);
      } else {
        setError(errorText(result.error));
        await load();
      }
    } finally {
      setBusy(false);
    }
  };

  const confirm = (einstufung: "intern" | "extern") =>
    run(() =>
      commands.meetingAiReleaseConfirm(
        meetingId,
        einstufung,
        ergebnis.fingerabdruck,
      ),
    );

  let listText: string;
  if (!ergebnis.aus_termin) {
    listText = t("meetings.mcp.release.list.noEvent");
  } else if (ergebnis.externe.length > 0) {
    listText = t("meetings.mcp.release.list.extern", {
      list: ergebnis.externe.join(", "),
    });
  } else if (ergebnis.ohne_adresse > 0) {
    listText = t("meetings.mcp.release.list.unknown", {
      count: ergebnis.ohne_adresse,
    });
  } else if (ergebnis.einstufung === "intern") {
    listText = t("meetings.mcp.release.list.intern");
  } else {
    listText = t("meetings.mcp.release.list.noDomains");
  }

  const statusText = allowed
    ? entscheidung.einstufung === "extern"
      ? t("meetings.mcp.release.status.allowedExtern")
      : t("meetings.mcp.release.status.allowedIntern")
    : t("meetings.mcp.release.status.blocked");

  return (
    <div className="space-y-1.5" data-testid="ai-release">
      <p>
        <span
          className={
            allowed
              ? "font-medium text-green-700 dark:text-green-400"
              : "font-medium text-amber-700 dark:text-amber-400"
          }
          data-testid="ai-release-status"
        >
          {statusText}
        </span>
        {reasons.length > 0 && (
          <span className="text-text/70" data-testid="ai-release-reasons">
            {": "}
            {reasons
              .map((r) => t(`meetings.mcp.release.reasons.${r}`))
              .join(", ")}
          </span>
        )}
      </p>
      <p className="text-text/70" data-testid="ai-release-list">
        {listText}
      </p>
      <div className="flex flex-wrap gap-2">
        {ergebnis.aus_termin && ergebnis.einstufung === "intern" && (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            onClick={() => void confirm("intern")}
            data-testid="ai-release-confirm-intern"
          >
            {t("meetings.mcp.release.confirmIntern")}
          </Button>
        )}
        {ergebnis.aus_termin && ergebnis.einstufung === "extern" && (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            onClick={() => void confirm("extern")}
            data-testid="ai-release-confirm-extern"
          >
            {t("meetings.mcp.release.confirmExtern")}
          </Button>
        )}
        {bestaetigung && (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            onClick={() =>
              void run(() => commands.meetingAiReleaseRevoke(meetingId))
            }
            data-testid="ai-release-revoke"
          >
            {t("meetings.mcp.release.revoke")}
          </Button>
        )}
      </div>
      {error && (
        <p
          className="text-amber-700 dark:text-amber-400"
          role="alert"
          data-testid="ai-release-error"
        >
          {error}
        </p>
      )}
    </div>
  );
};
