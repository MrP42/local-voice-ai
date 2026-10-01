import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, RotateCcw, ShieldCheck, XCircle } from "lucide-react";
import {
  commands,
  type PendingApproval,
  type WorkflowRunDetail,
  type WorkflowRunSummary,
} from "@/bindings";
import { Button } from "../ui/Button";
import { StateBadge } from "./StateBadge";
import { errorOf, useRuns } from "./useAutomations";
import { prettyJson } from "./model";

const stamp = (ms: number | null | undefined, language: string): string =>
  ms == null
    ? ""
    : new Date(ms).toLocaleString(language, {
        dateStyle: "short",
        timeStyle: "medium",
      });

const useStamp = () => {
  const { i18n } = useTranslation();
  return (ms: number | null | undefined) => stamp(ms, i18n.language);
};

export const RunStateBadge: React.FC<{ state: string }> = ({ state }) => {
  const { t } = useTranslation();
  return (
    <StateBadge
      state={state}
      label={t(`automations.runState.${state}`, { defaultValue: state })}
      testId="run-state"
    />
  );
};

interface RunListProps {
  workflowId: string | null;
  /** Name des gewaehlten Ablaufs (Filter) oder `null` fuer alle. */
  workflowName: string | null;
  version: number;
  onOpen: (runId: string) => void;
  onClearFilter: () => void;
}

/** Laufprotokoll: neueste Laeufe zuerst, mit Zustand, Herkunft des Starts und Zeit. */
export const RunList: React.FC<RunListProps> = ({
  workflowId,
  workflowName,
  version,
  onOpen,
  onClearFilter,
}) => {
  const { t } = useTranslation();
  const stamp = useStamp();
  const [openOnly, setOpenOnly] = useState(false);
  const { runs, loaded } = useRuns(workflowId, openOnly, version);

  return (
    <div className="space-y-3" data-testid="run-list">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-text-muted">
          {workflowName
            ? t("automations.runs.forWorkflow", { name: workflowName })
            : t("automations.runs.all")}
          {workflowId && (
            <button
              type="button"
              className="ms-2 underline"
              onClick={onClearFilter}
              data-testid="runs-clear-filter"
            >
              {t("automations.runs.showAll")}
            </button>
          )}
        </p>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={openOnly}
            onChange={(e) => setOpenOnly(e.target.checked)}
            data-testid="runs-open-only"
          />
          {t("automations.runs.openOnly")}
        </label>
      </div>
      {loaded && runs.length === 0 && (
        <p
          className="rounded-lg border border-dashed border-mid-gray/40 p-4 text-sm"
          data-testid="runs-empty"
        >
          {t("automations.runs.empty")}
        </p>
      )}
      <ul className="space-y-2">
        {runs.map((r) => (
          <RunRow key={r.id} run={r} stamp={stamp} onOpen={onOpen} />
        ))}
      </ul>
    </div>
  );
};

const RunRow: React.FC<{
  run: WorkflowRunSummary;
  stamp: (ms: number | null | undefined) => string;
  onOpen: (id: string) => void;
}> = ({ run, stamp, onOpen }) => {
  const { t } = useTranslation();
  return (
    <li
      className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-mid-gray/30 px-3 py-2"
      data-testid="run-row"
      data-run-id={run.id}
      data-state={run.state}
    >
      <div className="min-w-0 space-y-0.5">
        <p className="break-words text-sm font-medium">{run.workflow_name}</p>
        <p className="text-xs text-text-muted">
          {stamp(run.created_at)} ·{" "}
          {t(`automations.origin.${run.origin}`, { defaultValue: run.origin })}
          {run.dry_run ? ` · ${t("automations.runs.dry")}` : ""}
        </p>
        {run.error && (
          <p className="break-words text-xs text-status-red">{run.error}</p>
        )}
      </div>
      <div className="flex items-center gap-2">
        <RunStateBadge state={run.state} />
        <Button
          variant="secondary"
          size="sm"
          onClick={() => onOpen(run.id)}
          data-testid="run-open"
        >
          {t("automations.runs.open")}
        </Button>
      </div>
    </li>
  );
};

interface RunDetailProps {
  runId: string;
  version: number;
  pending: PendingApproval[];
  onBack: () => void;
  onOpenApprovals: () => void;
  /** Nach Abbrechen/Wiederholen: Listen neu laden. */
  onChanged: () => void;
}

const DETAIL_POLL_MS = 2000;

/** Ein Lauf: Zustand, Schritte mit Versuchen, Freigabe, Herkunft der Ausgaben. */
export const RunDetail: React.FC<RunDetailProps> = ({
  runId,
  version,
  pending,
  onBack,
  onOpenApprovals,
  onChanged,
}) => {
  const { t } = useTranslation();
  const stamp = useStamp();
  const [detail, setDetail] = useState<WorkflowRunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [accept, setAccept] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const alive = useRef(true);

  const load = useCallback(async () => {
    try {
      const r = await commands.workflowRunDetail(runId);
      if (!alive.current) return;
      if (r.status === "ok") {
        setDetail(r.data);
        setError(null);
      } else {
        setError(r.error);
      }
    } catch (e) {
      if (alive.current) setError(errorOf(e));
    }
  }, [runId]);

  useEffect(() => {
    alive.current = true;
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void load();
    }, DETAIL_POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(timer);
    };
  }, [load, version]);

  const act = async (fn: () => Promise<{ status: string; error?: string }>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const r = await fn();
      if (r.status === "error")
        setError(r.error ?? t("automations.errors.generic"));
    } catch (e) {
      setError(errorOf(e) || t("automations.errors.generic"));
    }
    await load();
    onChanged();
    setBusy(false);
  };

  if (!detail) {
    return (
      <div className="space-y-3" data-testid="run-detail">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft size={14} aria-hidden="true" />
          {t("automations.back")}
        </Button>
        {error ? (
          <p className="text-sm text-status-red" role="alert">
            {error}
          </p>
        ) : (
          <p className="text-sm text-text-muted">
            {t("automations.runs.loading")}
          </p>
        )}
      </div>
    );
  }

  const { run } = detail;
  const trigger = (() => {
    try {
      const ctx = JSON.parse(detail.context_json) as { trigger?: unknown };
      return JSON.stringify(ctx.trigger ?? {});
    } catch {
      return "{}";
    }
  })();
  const waiting = detail.steps.filter((s) => s.state === "awaiting_approval");
  const attemptsOf = (id: string) =>
    detail.steps.filter((s) => s.step_id === id).length;

  return (
    <div className="space-y-4" data-testid="run-detail" data-run-id={run.id}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          data-testid="run-back"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          {t("automations.back")}
        </Button>
        <RunStateBadge state={run.state} />
      </div>

      <div className="space-y-1">
        <h2 className="break-words text-base font-semibold">
          {run.workflow_name}
        </h2>
        <dl className="grid gap-x-4 gap-y-0.5 text-sm sm:grid-cols-[9rem_1fr]">
          <dt className="text-text-muted">{t("automations.runs.origin")}</dt>
          <dd data-testid="run-origin">
            {t(`automations.origin.${run.origin}`, {
              defaultValue: run.origin,
            })}
            {run.dry_run ? ` · ${t("automations.runs.dry")}` : ""}
          </dd>
          <dt className="text-text-muted">{t("automations.runs.created")}</dt>
          <dd>{stamp(run.created_at)}</dd>
          {run.started_at != null && (
            <>
              <dt className="text-text-muted">
                {t("automations.runs.started")}
              </dt>
              <dd>{stamp(run.started_at)}</dd>
            </>
          )}
          {run.ended_at != null && (
            <>
              <dt className="text-text-muted">{t("automations.runs.ended")}</dt>
              <dd>{stamp(run.ended_at)}</dd>
            </>
          )}
        </dl>
        {run.error && (
          <p
            className="break-words text-sm text-status-red"
            role="alert"
            data-testid="run-error"
          >
            {run.error}
          </p>
        )}
        {run.wait_reason && !run.error && (
          <p className="text-sm text-text-muted">{run.wait_reason}</p>
        )}
        {run.cancel_requested && !detail.run.ended_at && (
          <p className="text-sm text-text-muted">
            {t("automations.runs.cancelRequested")}
          </p>
        )}
      </div>

      {error && (
        <p
          className="text-sm text-status-red"
          role="alert"
          data-testid="run-action-error"
        >
          {error}
        </p>
      )}
      {notice && (
        <p className="text-sm" role="status">
          {notice}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {detail.can_cancel && (
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                const r = await commands.workflowRunCancel(run.id);
                if (r.status === "ok")
                  setNotice(
                    r.data
                      ? t("automations.runs.cancelled")
                      : t("automations.runs.cancelPending"),
                  );
                return r;
              })
            }
            data-testid="run-cancel"
          >
            <XCircle size={14} aria-hidden="true" />
            {t("automations.runs.cancel")}
          </Button>
        )}
        {detail.can_retry && (
          <>
            {detail.retry_needs_confirmation && (
              <label className="flex max-w-md items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={accept}
                  onChange={(e) => setAccept(e.target.checked)}
                  data-testid="run-retry-accept"
                />
                {t("automations.runs.retryAccept")}
              </label>
            )}
            <Button
              size="sm"
              disabled={busy || (detail.retry_needs_confirmation && !accept)}
              onClick={() =>
                void act(() => commands.workflowRunRetry(run.id, accept))
              }
              data-testid="run-retry"
            >
              <RotateCcw size={14} aria-hidden="true" />
              {t("automations.runs.retry")}
            </Button>
          </>
        )}
      </div>

      {waiting.length > 0 && (
        <ApprovalNotice
          approvalIds={waiting
            .map((s) => s.approval_id)
            .filter((x): x is string => !!x)}
          pending={pending}
          onOpen={onOpenApprovals}
        />
      )}

      <section className="space-y-2" aria-label={t("automations.runs.steps")}>
        <h3 className="text-sm font-semibold">{t("automations.runs.steps")}</h3>
        <ol className="space-y-2">
          {detail.steps.map((s) => (
            <li
              key={`${s.step_id}-${s.attempt}`}
              className="space-y-1 rounded-lg border border-mid-gray/30 p-3"
              data-testid="run-step"
              data-step-id={s.step_id}
              data-attempt={s.attempt}
              data-state={s.state}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="min-w-0 break-words text-sm font-medium">
                  {s.ordinal + 1}. {s.action_title ?? s.action}
                  <span className="ms-2 text-xs font-normal text-text-muted">
                    {s.step_id}
                    {attemptsOf(s.step_id) > 1
                      ? ` · ${t("automations.runs.attempt", { n: s.attempt })}`
                      : ""}
                  </span>
                </p>
                <StateBadge
                  state={s.state}
                  label={t(`automations.stepState.${s.state}`, {
                    defaultValue: s.state,
                  })}
                  testId="step-state"
                />
              </div>
              {s.error && (
                <p className="break-words text-xs text-status-red">{s.error}</p>
              )}
              {s.wake_at != null && s.state === "waiting" && (
                <p className="text-xs text-text-muted">
                  {t("automations.runs.wakeAt", { time: stamp(s.wake_at) })}
                </p>
              )}
              {s.started_at != null && (
                <p className="text-xs text-text-muted">
                  {stamp(s.started_at)}
                  {s.ended_at != null ? ` – ${stamp(s.ended_at)}` : ""}
                </p>
              )}
              {(s.input_json || s.output_json) && (
                <details>
                  <summary className="cursor-pointer text-xs text-text-muted">
                    {t("automations.runs.data")}
                  </summary>
                  {s.input_json && (
                    <>
                      <p className="mt-1 text-xs font-medium">
                        {t("automations.runs.input")}
                      </p>
                      <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs">
                        {prettyJson(s.input_json)}
                      </pre>
                    </>
                  )}
                  {s.output_json && (
                    <>
                      <p className="mt-1 text-xs font-medium">
                        {t("automations.runs.output")}
                      </p>
                      <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs">
                        {prettyJson(s.output_json)}
                      </pre>
                    </>
                  )}
                </details>
              )}
            </li>
          ))}
        </ol>
      </section>

      <section
        className="space-y-2"
        aria-label={t("automations.runs.provenance")}
      >
        <h3 className="text-sm font-semibold">
          {t("automations.runs.provenance")}
        </h3>
        {detail.provenance.length === 0 ? (
          <p
            className="text-sm text-text-muted"
            data-testid="run-provenance-empty"
          >
            {run.dry_run
              ? t("automations.runs.provenanceDry")
              : t("automations.runs.provenanceNone")}
          </p>
        ) : (
          <ul className="space-y-2">
            {detail.provenance.map((e) => (
              <li
                key={e.id}
                className="rounded-lg border border-mid-gray/30 px-3 py-2 text-sm"
                data-testid="run-provenance"
              >
                <p className="font-medium">{e.operation}</p>
                <p className="text-xs text-text-muted">
                  {e.model_label ?? e.model_id ?? t("automations.runs.noModel")}
                  {e.provider ? ` · ${e.provider}` : ""}
                  {e.locality
                    ? ` · ${t(e.locality === "local" ? "automations.runs.local" : "automations.runs.remote")}`
                    : ""}
                  {e.duration_ms != null
                    ? ` · ${(e.duration_ms / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s`
                    : ""}
                  {e.sources.length > 0
                    ? ` · ${t("automations.runs.sources", { count: e.sources.length })}`
                    : ""}
                </p>
              </li>
            ))}
          </ul>
        )}
        <details>
          <summary className="cursor-pointer text-xs text-text-muted">
            {t("automations.runs.triggerData")}
          </summary>
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs">
            {prettyJson(trigger)}
          </pre>
        </details>
      </section>
    </div>
  );
};

/** Der Hinweis auf eine wartende Freigabe: zeigt die Vorschau und fuehrt zum vorhandenen
 *  Freigabedialog der Seite „Integrationen“ (dort wird entschieden, nirgends sonst). */
const ApprovalNotice: React.FC<{
  approvalIds: string[];
  pending: PendingApproval[];
  onOpen: () => void;
}> = ({ approvalIds, pending, onOpen }) => {
  const { t } = useTranslation();
  const mine = pending.filter((p) => approvalIds.includes(p.approval.id));
  return (
    <div
      className="space-y-2 rounded-lg border border-logo-primary bg-logo-primary/15 px-3 py-2"
      role="status"
      data-testid="run-approval"
    >
      <p className="text-sm font-medium">
        {mine.length > 0
          ? t("automations.runs.approvalWaiting")
          : t("automations.runs.approvalGone")}
      </p>
      {mine.map((p) =>
        p.approval.args_preview ? (
          <pre
            key={p.approval.id}
            className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs"
            data-testid="run-approval-preview"
          >
            {p.approval.args_preview}
          </pre>
        ) : null,
      )}
      {mine.length > 0 && (
        <Button size="sm" onClick={onOpen} data-testid="run-approval-open">
          <ShieldCheck size={14} aria-hidden="true" />
          {t("automations.runs.approvalOpen")}
        </Button>
      )}
    </div>
  );
};
