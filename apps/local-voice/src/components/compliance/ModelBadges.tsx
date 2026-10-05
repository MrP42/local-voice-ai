import React from "react";
import { useTranslation } from "react-i18next";
import { Ban, Cloud, HardDrive } from "lucide-react";
import type { Assessment } from "@/bindings";
import { Flag } from "./Flag";

/** Gründe einer Bewertung als lesbarer Satz. */
export const reasonsText = (
  t: (key: string, opts?: Record<string, unknown>) => string,
  a: Assessment,
) => a.reasons.map((r) => t(`compliance.reasons.${r}`, { defaultValue: r })).join(" ");

/**
 * Hinter dem Modellnamen: Wolke für Cloud-Modelle, die Flagge des
 * Serverstandorts, und ein Sperrzeichen, wenn das Regelwerk das Modell nicht
 * erlaubt. Lokale Modelle zeigen ein Laufwerk -- "bleibt auf dem Rechner".
 */
export const ModelBadges: React.FC<{ assessment?: Assessment | null }> = ({
  assessment,
}) => {
  const { t } = useTranslation();
  if (!assessment) return null;
  const where = assessment.countries
    .map((c) => t(`compliance.countries.${c}`, { defaultValue: c }))
    .join(", ");
  const title = assessment.cloud
    ? t("compliance.badge.cloud", { where: where || t("compliance.countries.unknown") })
    : t("compliance.badge.local");
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 align-middle"
      title={`${title}\n${reasonsText(t, assessment)}`}
      data-model-badges
      data-cloud={assessment.cloud || undefined}
      data-verdict={assessment.verdict}
    >
      {assessment.cloud ? (
        <Cloud className="h-3.5 w-3.5 text-sky-500" aria-label={title} />
      ) : (
        <HardDrive className="h-3.5 w-3.5 text-text/40" aria-label={title} />
      )}
      {assessment.countries.map((c) => (
        <Flag key={c} code={c} title={t(`compliance.countries.${c}`, { defaultValue: c })} />
      ))}
      {assessment.verdict === "blocked" && (
        <Ban className="h-3.5 w-3.5 text-red-500" aria-label={t("compliance.badge.blocked")} />
      )}
    </span>
  );
};

export default ModelBadges;
