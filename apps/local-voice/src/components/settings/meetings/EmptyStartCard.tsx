import React from "react";
import { useTranslation } from "react-i18next";
import { Link, Mic, Upload } from "lucide-react";
import { Button } from "../../ui/Button";

interface EmptyStartCardProps {
  /** Startdialog der Aufnahme (Einwilligung bleibt Pflicht). */
  onRecord: () => void;
  /** Datei waehlen; sie fuellt diesen Eintrag. */
  onImport: () => void;
  /** YouTube-Link einfuegen; er fuellt diesen Eintrag. */
  onLink: () => void;
  /** Eine andere Aufnahme laeuft: es kann nicht noch eine beginnen. */
  recordDisabled?: boolean;
  importDisabled?: boolean;
}

/**
 * Startflaeche eines LEEREN Eintrags (G1, #70): die drei Wege, ihn zu fuellen.
 * Es sind dieselben Wege wie in der Bedienspalte (Aufnahme, Import, Link); hier
 * stehen sie gross in der Mitte, damit ein neuer Eintrag nicht leer wirkt.
 * Die Notizen darunter bleiben von Anfang an nutzbar.
 */
export const EmptyStartCard: React.FC<EmptyStartCardProps> = ({
  onRecord,
  onImport,
  onLink,
  recordDisabled = false,
  importDisabled = false,
}) => {
  const { t } = useTranslation();
  return (
    <section
      role="group"
      aria-label={t("meetings.empty.startGroup")}
      data-testid="empty-start"
      className="space-y-2 rounded-lg border border-dashed border-mid-gray/40 px-3 py-3"
    >
      <div>
        <h4 className="text-sm font-semibold">
          {t("meetings.empty.startTitle")}
        </h4>
        <p className="text-xs text-text/70">{t("meetings.empty.startBody")}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          onClick={onRecord}
          disabled={recordDisabled}
          data-testid="empty-record"
        >
          <Mic width={14} height={14} aria-hidden="true" />
          {t("meetings.record.start")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={onImport}
          disabled={importDisabled}
          data-testid="empty-import"
        >
          <Upload width={14} height={14} aria-hidden="true" />
          {t("meetings.importAction.name")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={onLink}
          data-testid="empty-link"
        >
          <Link width={14} height={14} aria-hidden="true" />
          {t("meetings.importAction.linkName")}
        </Button>
      </div>
    </section>
  );
};
