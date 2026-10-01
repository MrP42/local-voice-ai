import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Download,
  ListChecks,
  ListTree,
  Pencil,
  Play,
  PlayCircle,
  Trash2,
} from "lucide-react";
import type { WorkflowCatalog, WorkflowItem, WorkflowStatus } from "@/bindings";
import { Button } from "../ui/Button";
import { RunStateBadge } from "./RunViews";

interface WorkflowListProps {
  items: WorkflowItem[];
  status: WorkflowStatus | null;
  loaded: boolean;
  failed: boolean;
  catalog: WorkflowCatalog;
  busyId: string | null;
  onNew: () => void;
  onEdit: (item: WorkflowItem) => void;
  onPlan: (item: WorkflowItem) => void;
  onRuns: (item: WorkflowItem) => void;
  onStart: (item: WorkflowItem, dryRun: boolean) => void;
  onEnabled: (item: WorkflowItem, enabled: boolean) => void;
  onArmed: (item: WorkflowItem, armed: boolean) => void;
  onExport: (item: WorkflowItem) => void;
  onDelete: (item: WorkflowItem) => void;
}

type Confirm = { id: string; kind: "arm" | "delete" };

const when = (ms: number, language: string) =>
  new Date(ms).toLocaleString(language, {
    dateStyle: "short",
    timeStyle: "short",
  });

/** Die Liste der Ablaeufe: an/aus, scharf oder Trockenlauf, letzter Lauf, alle Handgriffe. */
export const WorkflowList: React.FC<WorkflowListProps> = ({
  items,
  status,
  loaded,
  failed,
  catalog,
  busyId,
  onNew,
  onEdit,
  onPlan,
  onRuns,
  onStart,
  onEnabled,
  onArmed,
  onExport,
  onDelete,
}) => {
  const { t, i18n } = useTranslation();
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const triggerTitle = (kind: string) =>
    t(`automations.triggers.${kind}`, {
      defaultValue: catalog.triggers.find((x) => x.id === kind)?.title ?? kind,
    });

  const cloud = status?.cloud_only ?? [];
  const outages = (status?.channels ?? []).filter((c) => c.outage);

  return (
    <div className="space-y-3" data-testid="workflow-list">
      {failed && (
        <p className="text-sm text-status-red" role="alert">
          {t("automations.loadError")}
        </p>
      )}
      {cloud.length > 0 && (
        <div
          className="space-y-1 rounded-lg border border-mid-gray/40 bg-amber-500/10 px-3 py-2"
          role="status"
          data-testid="status-cloud"
        >
          <p className="text-sm font-medium">
            {t("automations.status.cloudTitle", { count: cloud.length })}
          </p>
          <ul className="space-y-0.5 text-xs">
            {cloud.map((c) => (
              <li key={`${c.workflow_id}:${c.name}`} className="break-words">
                {t("automations.status.cloudFile", {
                  file: c.name,
                  workflow: c.workflow_name,
                })}
              </li>
            ))}
          </ul>
          <p className="text-xs text-text-muted">
            {t("automations.status.cloudHint")}
          </p>
        </div>
      )}
      {outages.length > 0 && (
        <div
          className="space-y-1 rounded-lg border border-mid-gray/40 bg-red-500/10 px-3 py-2"
          role="status"
          data-testid="status-channels"
        >
          <p className="text-sm font-medium">
            {t("automations.status.channelTitle", { count: outages.length })}
          </p>
          <ul className="space-y-0.5 text-xs">
            {outages.map((c) => (
              <li
                key={`${c.workflow_id}:${c.channel_id}`}
                className="break-words"
              >
                {t("automations.status.channelLine", {
                  channel: c.channel_id,
                  workflow: c.workflow_name,
                  failures: c.failures,
                })}
                {c.last_error ? ` (${c.last_error})` : ""}
                {c.last_ok_ms != null
                  ? ` · ${t("automations.status.lastOk", { time: when(c.last_ok_ms, i18n.language) })}`
                  : ""}
              </li>
            ))}
          </ul>
          <p className="text-xs text-text-muted">
            {t("automations.status.channelHint")}
          </p>
        </div>
      )}

      {loaded && items.length === 0 && !failed && (
        <div
          className="space-y-2 rounded-lg border border-dashed border-mid-gray/40 p-4 text-sm"
          data-testid="workflows-empty"
        >
          <p>{t("automations.empty")}</p>
          <p className="text-text-muted">{t("automations.emptyHint")}</p>
          <Button size="sm" onClick={onNew}>
            {t("automations.new")}
          </Button>
        </div>
      )}

      <ul className="space-y-3">
        {items.map((item) => {
          const confirming = confirm?.id === item.id ? confirm.kind : null;
          const busy = busyId === item.id;
          return (
            <li
              key={item.id}
              className="space-y-3 rounded-lg border border-mid-gray/30 p-3"
              data-testid="workflow-card"
              data-workflow-id={item.id}
              data-enabled={item.enabled}
              data-armed={!item.dry_run}
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0">
                  <h3
                    className="break-words text-sm font-semibold"
                    data-testid="workflow-name"
                  >
                    {item.name}
                  </h3>
                  <p className="text-xs text-text-muted">
                    {triggerTitle(item.trigger_kind)} ·{" "}
                    {t("automations.steps", { count: item.step_count })}
                  </p>
                </div>
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    role="switch"
                    checked={item.enabled}
                    disabled={busy}
                    onChange={(e) => onEnabled(item, e.target.checked)}
                    aria-label={t("automations.enabledLabel", {
                      name: item.name,
                    })}
                    data-testid="workflow-enabled"
                  />
                  {item.enabled
                    ? t("automations.enabled")
                    : t("automations.disabled")}
                </label>
              </div>

              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
                <span
                  className={`rounded-full px-2 py-0.5 text-xs font-medium text-text ${item.dry_run ? "bg-mid-gray/20" : "bg-green-500/20"}`}
                  data-testid="workflow-mode"
                >
                  {item.dry_run
                    ? t("automations.mode.dry")
                    : t("automations.mode.armed")}
                </span>
                <span
                  className="flex items-center gap-2 text-xs text-text-muted"
                  data-testid="workflow-lastrun"
                >
                  {item.last_run ? (
                    <>
                      {t("automations.lastRun")}
                      <RunStateBadge state={item.last_run.state} />
                      {when(item.last_run.created_at, i18n.language)}
                    </>
                  ) : (
                    t("automations.noRun")
                  )}
                </span>
                {item.open_runs > 0 && (
                  <span className="text-xs text-text-muted">
                    {t("automations.openRuns", { count: item.open_runs })}
                  </span>
                )}
              </div>

              {item.dry_run && (
                <p className="text-xs text-text-muted">
                  {t("automations.mode.dryHint")}
                </p>
              )}

              {confirming === "arm" && (
                <div
                  className="space-y-2 rounded-lg border border-logo-primary bg-logo-primary/15 px-3 py-2"
                  role="alert"
                  data-testid="arm-confirm"
                >
                  <p className="text-sm">
                    {t("automations.mode.armAsk", { name: item.name })}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      size="sm"
                      onClick={() => {
                        setConfirm(null);
                        onArmed(item, true);
                      }}
                      data-testid="arm-confirm-yes"
                    >
                      {t("automations.mode.arm")}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setConfirm(null)}
                    >
                      {t("automations.cancel")}
                    </Button>
                  </div>
                </div>
              )}
              {confirming === "delete" && (
                <div
                  className="flex flex-wrap items-center gap-2 rounded-lg border border-red-500/50 bg-red-500/10 px-3 py-2"
                  role="alert"
                  data-testid="delete-confirm"
                >
                  <span className="text-sm">
                    {t("automations.deleteAsk", { name: item.name })}
                  </span>
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => {
                      setConfirm(null);
                      onDelete(item);
                    }}
                    data-testid="delete-confirm-yes"
                  >
                    {t("automations.delete")}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => setConfirm(null)}
                  >
                    {t("automations.cancel")}
                  </Button>
                </div>
              )}

              <div className="flex flex-wrap gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => onEdit(item)}
                  data-testid="workflow-edit"
                >
                  <Pencil size={14} aria-hidden="true" />
                  {t("automations.edit")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => onPlan(item)}
                  data-testid="workflow-plan"
                >
                  <ListChecks size={14} aria-hidden="true" />
                  {t("automations.planShow")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onClick={() => onStart(item, true)}
                  data-testid="workflow-probe"
                >
                  <PlayCircle size={14} aria-hidden="true" />
                  {t("automations.probe")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy || item.dry_run || !item.enabled}
                  title={
                    item.dry_run || !item.enabled
                      ? t("automations.startNowDisabled")
                      : undefined
                  }
                  onClick={() => onStart(item, false)}
                  data-testid="workflow-start"
                >
                  <Play size={14} aria-hidden="true" />
                  {t("automations.startNow")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => onRuns(item)}
                  data-testid="workflow-runs"
                >
                  <ListTree size={14} aria-hidden="true" />
                  {t("automations.runsOf")}
                </Button>
                {item.dry_run ? (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirm({ id: item.id, kind: "arm" })}
                    data-testid="workflow-arm"
                  >
                    {t("automations.mode.arm")}
                  </Button>
                ) : (
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={busy}
                    onClick={() => onArmed(item, false)}
                    data-testid="workflow-disarm"
                  >
                    {t("automations.mode.disarm")}
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onExport(item)}
                  data-testid="workflow-export"
                >
                  <Download size={14} aria-hidden="true" />
                  {t("automations.exportJson")}
                </Button>
                <Button
                  variant="danger-ghost"
                  size="sm"
                  onClick={() => setConfirm({ id: item.id, kind: "delete" })}
                  data-testid="workflow-delete"
                >
                  <Trash2 size={14} aria-hidden="true" />
                  {t("automations.delete")}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
};
