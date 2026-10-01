import React from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";
import { Button } from "../ui/Button";
import { CATALOG, type CatalogEntry } from "./model";
import { KindIcon } from "./KindIcon";

interface IntegrationCatalogProps {
  onBack: () => void;
  /** Richtet eine Art ein (Dialog, Assistent oder Sprung zum Abschnitt). */
  onSetup: (entry: CatalogEntry) => void;
}

/**
 * Katalog: alle Arten von Integrationen. Was sich schon einrichten laesst,
 * hat einen Knopf; was mit einem der naechsten Pakete kommt, ist „bald“
 * markiert und hat keinen Knopf (statt eines Knopfes, der nichts tut).
 */
export const IntegrationCatalog: React.FC<IntegrationCatalogProps> = ({
  onBack,
  onSetup,
}) => {
  const { t } = useTranslation();
  return (
    <div className="space-y-4" data-testid="catalog">
      <div>
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          data-testid="integrations-back"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          {t("integrations.back")}
        </Button>
      </div>
      <div>
        <h2 className="text-lg font-semibold">
          {t("integrations.catalog.title")}
        </h2>
        <p className="text-sm text-text-muted">
          {t("integrations.catalog.description")}
        </p>
      </div>
      <ul className="grid gap-3 sm:grid-cols-2">
        {CATALOG.map((entry) => (
          <li
            key={entry.id}
            className="flex flex-col gap-2 rounded-lg border border-mid-gray/20 p-3"
            data-testid="catalog-item"
            data-catalog-id={entry.id}
            data-status={entry.status}
          >
            <div className="flex items-start gap-3">
              <KindIcon kind={entry.id} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <h3
                    className="text-sm font-semibold"
                    data-testid="catalog-name"
                  >
                    {t(`integrations.catalog.items.${entry.id}.name`)}
                  </h3>
                  {entry.status === "soon" && (
                    <span
                      className="rounded-full bg-mid-gray/20 px-2 py-0.5 text-xs font-medium text-text-muted"
                      data-testid="catalog-soon"
                    >
                      {t("integrations.catalog.soon")}
                    </span>
                  )}
                  {entry.status === "auto" && (
                    <span className="rounded-full bg-green-500/15 px-2 py-0.5 text-xs font-medium text-status-green">
                      {t("integrations.catalog.auto")}
                    </span>
                  )}
                </div>
                <p className="mt-1 text-sm text-text-muted">
                  {t(`integrations.catalog.items.${entry.id}.description`)}
                </p>
              </div>
            </div>
            {entry.status === "available" && (
              <div className="ps-12">
                <Button
                  size="sm"
                  onClick={() => onSetup(entry)}
                  data-testid={`catalog-setup-${entry.id}`}
                >
                  {t(`integrations.catalog.items.${entry.id}.action`)}
                </Button>
              </div>
            )}
            {entry.status === "auto" && (
              <p className="ps-12 text-xs text-text-muted">
                {t(`integrations.catalog.items.${entry.id}.note`)}
              </p>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
};
