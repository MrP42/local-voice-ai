import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { commands, type BriefInfo } from "@/bindings";
import { Button } from "../../../ui/Button";

interface BriefButtonProps {
  /** Schluessel des Termins (`CalEvent.key`). */
  eventKey: string;
  /** Klick bei vorhandenem Brief-Zuschnitt. */
  onOpen: (info: BriefInfo) => void;
  testId?: string;
  size?: "sm" | "md";
}

/**
 * Knopf "Vorbereiten" zu einem Kalendertermin (M5-P5e). Er fragt das Backend,
 * welche fruehere Besprechungen mit gemeinsamen Teilnehmenden es gibt; ohne
 * eine ist er deaktiviert und sagt im Tooltip warum. Mit gespeichertem Brief
 * (zweiter Klick) heisst der Tooltip "Gespeicherten Brief oeffnen".
 */
export const BriefButton: React.FC<BriefButtonProps> = ({
  eventKey,
  onOpen,
  testId = "brief-button",
  size = "sm",
}) => {
  const { t } = useTranslation();
  const [info, setInfo] = useState<BriefInfo | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setInfo(null);
    setFailed(false);
    void commands.peopleBriefInfo(eventKey).then((result) => {
      if (cancelled) return;
      if (result.status === "ok") setInfo(result.data);
      else setFailed(true);
    });
    return () => {
      cancelled = true;
    };
  }, [eventKey]);

  const shared = info?.shared_meetings ?? 0;
  const enabled = info !== null && shared > 0;
  const title = failed
    ? t("meetings.people.brief.error")
    : info === null
      ? ""
      : shared === 0
        ? t("meetings.people.brief.none")
        : info.thread_id
          ? t("meetings.people.brief.saved")
          : t("meetings.people.brief.title", { count: shared });

  return (
    <span title={title} className="inline-flex">
      <Button
        size={size}
        variant="secondary"
        disabled={!enabled}
        onClick={() => info && onOpen(info)}
        data-testid={testId}
        data-shared={info ? shared : undefined}
        data-saved={info?.thread_id ? "true" : undefined}
      >
        <Sparkles width={12} height={12} aria-hidden="true" />
        {t("meetings.people.brief.button")}
      </Button>
    </span>
  );
};
