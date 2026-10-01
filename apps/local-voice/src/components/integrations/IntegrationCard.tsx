import React from "react";
import { useTranslation } from "react-i18next";
import type { IntegrationView } from "@/bindings";
import { Button } from "../ui/Button";
import { folderPathOf } from "./model";
import { KindIcon } from "./KindIcon";

interface IntegrationCardProps {
  view: IntegrationView;
  onOpen: (id: string) => void;
}

/** Eine eingerichtete Integration in der Liste: Art, Richtung, Zustand. */
export const IntegrationCard: React.FC<IntegrationCardProps> = ({
  view,
  onOpen,
}) => {
  const { t } = useTranslation();
  const { integration } = view;
  const path = folderPathOf(integration.config_json);
  return (
    <li
      className="flex flex-col gap-2 rounded-lg border border-mid-gray/20 p-3"
      data-testid="integration-card"
      data-integration-id={integration.id}
    >
      <div className="flex items-start gap-3">
        <KindIcon kind={integration.kind} />
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold break-words">
            {integration.label}
          </h3>
          <p className="text-xs text-text-muted">
            {t(`integrations.kinds.${integration.kind}`)}
          </p>
          {path && (
            <p className="mt-1 text-xs text-text-muted break-all">{path}</p>
          )}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span
          className="rounded-full bg-mid-gray/20 px-2 py-0.5 font-medium"
          data-testid="integration-direction"
        >
          {t(`integrations.directions.${integration.direction}`)}
        </span>
        {!integration.enabled && (
          <span className="rounded-full bg-amber-500/15 px-2 py-0.5 font-medium text-status-amber">
            {t("integrations.status.off")}
          </span>
        )}
        {view.pending_approvals > 0 && (
          <span
            className="rounded-full bg-logo-primary/30 px-2 py-0.5 font-medium text-text"
            data-testid="integration-pending"
          >
            {t("integrations.status.pending", { count: view.pending_approvals })}
          </span>
        )}
      </div>
      {integration.last_error && (
        <p className="text-xs text-status-red break-words">
          {integration.last_error}
        </p>
      )}
      <div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => onOpen(integration.id)}
          data-testid="integration-open"
        >
          {t("integrations.open")}
        </Button>
      </div>
    </li>
  );
};
