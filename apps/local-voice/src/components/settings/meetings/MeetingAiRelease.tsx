import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type KiFreigabeStand,
  type Rechtsgrundlage,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Select } from "../../ui/Select";

/**
 * G9a-2 (lva#578, ADR-0031 E2.6–E2.8): Freigabe EINER Besprechung für KI-Clients in den
 * Details. Zeigt die Einstufung laut Termin und die Gründe einer Sperre; bestätigt wird nur
 * diese Besprechung mit dieser Teilnehmerliste (keine Sammelbestätigung). „Als intern
 * freigeben“ gibt es nur, wenn die Liste intern ist. Bestätigen setzt den Nachweis nach
 * ADR-0031 E2 Nr. 7 voraus: Information der Teilnehmenden, gewählte Rechtsgrundlage und bei
 * Einwilligung deren Vorliegen.
 *
 * Nebenläufigkeit (Codex #583): Beim Wechsel der Besprechung wird der alte Stand verworfen;
 * eine verspätete Antwort einer früheren Anfrage überschreibt nie den neuen Stand.
 */
const LEGAL_BASES: Rechtsgrundlage[] = [
  "einwilligung",
  "vertrag",
  "rechtliche_pflicht",
  "oeffentliche_aufgabe",
  "berechtigtes_interesse",
  "kollektivvereinbarung",
];

export const MeetingAiRelease: React.FC<{ meetingId: string }> = ({
  meetingId,
}) => {
  const { t } = useTranslation();
  const [stand, setStand] = useState<KiFreigabeStand | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [informed, setInformed] = useState(false);
  const [basis, setBasis] = useState<Rechtsgrundlage | "">("");
  const [consent, setConsent] = useState(false);
  // Jede Anfrage bekommt eine Nummer; nur die Antwort der jüngsten gilt.
  const generation = useRef(0);

  const load = useCallback(async () => {
    const mine = ++generation.current;
    try {
      const result = await commands.meetingAiReleaseStatus(meetingId);
      if (mine !== generation.current) return;
      if (result.status === "ok") {
        setStand(result.data);
        setLoadFailed(false);
      } else {
        setStand(null);
        setLoadFailed(true);
      }
    } catch {
      if (mine !== generation.current) return;
      setStand(null);
      setLoadFailed(true);
    }
  }, [meetingId]);

  useEffect(() => {
    setStand(null);
    setError(null);
    setLoadFailed(false);
    setInformed(false);
    setBasis("");
    setConsent(false);
    void load();
  }, [load]);

  if (loadFailed) {
    return (
      <p
        className="text-amber-700 dark:text-amber-400"
        role="alert"
        data-testid="ai-release-load-failed"
      >
        {t("meetings.mcp.release.loadFailed")}
      </p>
    );
  }
  if (!stand) return null;

  const { ergebnis, entscheidung, bestaetigung } = stand;
  const allowed = entscheidung.ergebnis === "erlaubt";
  const reasons =
    entscheidung.ergebnis === "gesperrt" ? entscheidung.gruende : [];

  const errorText = (code: string) =>
    code === "ki_freigabe_veraltet" ||
    code === "ki_freigabe_nicht_intern" ||
    code === "ki_freigabe_ohne_information" ||
    code === "ki_freigabe_ohne_rechtsgrundlage" ||
    code === "ki_freigabe_ohne_einwilligung" ||
    code === "ki_freigabe_richtlinie_fehlt"
      ? t(`meetings.mcp.release.errors.${code}`)
      : t("meetings.mcp.release.errors.generic");

  const run = async (
    action: () => Promise<
      | { status: "ok"; data: KiFreigabeStand }
      | { status: "error"; error: string }
    >,
  ) => {
    const mine = ++generation.current;
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      if (mine !== generation.current) return;
      if (result.status === "ok") {
        setStand(result.data);
      } else {
        setError(errorText(result.error));
        await load();
      }
    } catch {
      if (mine === generation.current) {
        setError(t("meetings.mcp.release.errors.generic"));
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
        {
          informiert: informed,
          rechtsgrundlage: basis === "" ? null : basis,
          einwilligungen_liegen_vor: basis === "einwilligung" && consent,
        },
      ),
    );
  const ready =
    informed && basis !== "" && (basis !== "einwilligung" || consent);
  const canConfirm =
    ergebnis.aus_termin &&
    (ergebnis.einstufung === "intern" || ergebnis.einstufung === "extern");

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
      <p aria-live="polite" aria-atomic="true">
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
      {canConfirm && (
        <label className="flex items-start gap-2 text-text/80">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={informed}
            onChange={(e) => setInformed(e.target.checked)}
            data-testid="ai-release-informed"
          />
          <span>{t("meetings.mcp.release.informed")}</span>
        </label>
      )}
      {canConfirm && (
        <div className="space-y-1">
          <div className="flex flex-wrap items-center gap-2 text-text/80">
            <span>{t("meetings.mcp.release.legalBasis")}</span>
            <div className="min-w-64" data-testid="ai-release-basis">
              <Select
                value={basis === "" ? null : basis}
                options={LEGAL_BASES.map((b) => ({
                  value: b,
                  label: t(`meetings.mcp.release.legalBasisOptions.${b}`),
                }))}
                isClearable={false}
                menuPortal
                placeholder={t("meetings.mcp.release.legalBasisChoose")}
                ariaLabel={t("meetings.mcp.release.legalBasis")}
                onChange={(v) => setBasis((v as Rechtsgrundlage | null) ?? "")}
              />
            </div>
          </div>
          <p className="text-xs text-text/60">
            {t("meetings.mcp.release.legalBasisHint")}
          </p>
          {basis === "einwilligung" && (
            <label className="flex items-start gap-2 text-text/80">
              <input
                type="checkbox"
                className="mt-0.5"
                checked={consent}
                onChange={(e) => setConsent(e.target.checked)}
                data-testid="ai-release-consent"
              />
              <span>{t("meetings.mcp.release.consentPresent")}</span>
            </label>
          )}
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        {ergebnis.aus_termin && ergebnis.einstufung === "intern" && (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || !ready}
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
            disabled={busy || !ready}
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
