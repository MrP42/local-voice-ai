import React from "react";
import { useTranslation } from "react-i18next";
import type { Coverage } from "@/bindings";
import { formatCoverage } from "@/lib/meetingChat";

interface CoverageNoteProps {
  coverage: Coverage | null | undefined;
  /** Chat ueber viele Besprechungen (sonst eine bzw. die laufende). */
  global: boolean;
}

/** Graue Zeile unter jeder Antwort: was durchsucht und gelesen wurde. */
export const CoverageNote: React.FC<CoverageNoteProps> = ({
  coverage,
  global,
}) => {
  const { t } = useTranslation();
  if (!coverage) return null;
  const text = formatCoverage(coverage, (key, options) => t(key, options), {
    global,
  });
  if (text === "") return null;
  return (
    <p data-testid="coverage-note" className="text-xs text-text/50">
      {text}
    </p>
  );
};
