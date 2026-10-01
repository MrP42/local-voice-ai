import React from "react";
import { useTranslation } from "react-i18next";
import type { ProvenanceEntry, SourceRef } from "@/bindings";
import { requestOpenSegment } from "@/lib/meetingsBus";
import {
  percent,
  segmentOf,
  type AgentPreview,
  type ExtractItem,
} from "./agentModel";

const seconds = (ms: number) =>
  `${(ms / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s`;

/**
 * Eine Quelle der Herkunft. Ein Segment des Transkripts (`<Besprechung>:S<n>`) ist anklickbar und
 * fuehrt zur Stelle in der Besprechung; alles andere steht als Text da.
 */
export const SourceItem: React.FC<{ source: SourceRef }> = ({ source }) => {
  const { t } = useTranslation();
  const ref = source.ref ?? "";
  const segment = source.kind === "segment" ? segmentOf(ref) : null;
  if (segment) {
    return (
      <li>
        <button
          type="button"
          className="cursor-pointer text-start text-logo-primary underline decoration-logo-primary/40 underline-offset-2 hover:decoration-logo-primary focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary"
          title={t("automations.agent.openSegment")}
          data-testid="prov-segment-source"
          data-meeting-id={segment.meetingId}
          data-segment={segment.index}
          onClick={() =>
            requestOpenSegment({
              meetingId: segment.meetingId,
              segmentIndex: segment.index,
            })
          }
        >
          {t("automations.agent.segment", { n: segment.index })}
        </button>
      </li>
    );
  }
  return (
    <li data-testid="prov-source">
      {source.title ?? (ref || source.kind)}{" "}
      <span className="text-text-muted">({source.kind})</span>
    </li>
  );
};

/**
 * Herkunft eines Ergebnisses im Laufprotokoll: Modell, Token, Dauer, Konfidenz und die Quellen.
 * Bei einem Agent-Schritt sind die Quellen die Segmente, auf die sich Modell und Code stuetzen.
 */
export const ProvenanceCard: React.FC<{ entry: ProvenanceEntry }> = ({
  entry: e,
}) => {
  const { t } = useTranslation();
  const confidence = percent(e.confidence);
  const hasTokens = e.prompt_tokens != null || e.completion_tokens != null;
  return (
    <li
      className="rounded-lg border border-mid-gray/30 px-3 py-2 text-sm"
      data-testid="run-provenance"
      data-operation={e.operation}
    >
      <p className="font-medium">
        {t(`automations.agent.operation.${e.operation}`, {
          defaultValue: e.operation,
        })}
      </p>
      <p className="text-xs text-text-muted" data-testid="prov-model">
        {e.model_label ?? e.model_id ?? t("automations.runs.noModel")}
        {e.provider ? ` · ${e.provider}` : ""}
        {e.locality
          ? ` · ${t(e.locality === "local" ? "automations.runs.local" : "automations.runs.remote")}`
          : ""}
        {e.duration_ms != null ? ` · ${seconds(e.duration_ms)}` : ""}
        {e.sources.length > 0
          ? ` · ${t("automations.runs.sources", { count: e.sources.length })}`
          : ""}
      </p>
      {(hasTokens || confidence != null) && (
        <p className="text-xs text-text-muted">
          {hasTokens && (
            <span data-testid="prov-tokens">
              {t("automations.agent.tokens", {
                input: e.prompt_tokens ?? 0,
                output: e.completion_tokens ?? 0,
              })}
            </span>
          )}
          {hasTokens && confidence != null ? " · " : ""}
          {confidence != null && (
            <span data-testid="prov-confidence">
              {t("automations.agent.confidence", { value: confidence })}
            </span>
          )}
        </p>
      )}
      {e.sources.length > 0 && (
        <ul className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-xs">
          {e.sources.map((s, i) => (
            <SourceItem key={`${s.kind}-${s.ref ?? i}-${i}`} source={s} />
          ))}
        </ul>
      )}
    </li>
  );
};

const Cited: React.FC<{ item: ExtractItem }> = ({ item }) => (
  <>
    {item.text}
    {item.assignee ? ` (${item.assignee})` : ""}
    {item.due ? ` · ${item.due}` : ""}
    {item.segments.length > 0 ? ` · ${item.segments.join(", ")}` : ""}
  </>
);

const List: React.FC<{
  label: string;
  items: ExtractItem[];
  testId: string;
}> = ({ label, items, testId }) =>
  items.length === 0 ? null : (
    <div data-testid={testId}>
      <p className="text-xs font-medium">
        {label} ({items.length})
      </p>
      <ul className="list-disc ps-5 text-xs">
        {items.map((it, i) => (
          <li key={`${i}-${it.text}`}>
            <Cited item={it} />
          </li>
        ))}
      </ul>
    </div>
  );

/** Das Ergebnis eines Agent-Schritts bzw. der Vorschau: Werkzeug und Argumente, Begruendung oder die gezogenen Listen. */
export const AgentOutcome: React.FC<{
  result: AgentPreview;
  testId: string;
  /** Die Zeile mit Modell, Dauer, Token und Konfidenz (im Lauf zeigt sie die Herkunft). */
  meta?: boolean;
}> = ({ result: r, testId, meta = true }) => {
  const { t } = useTranslation();
  const confidence = percent(r.provenance?.confidence);
  return (
    <div className="space-y-2" data-testid={testId}>
      {r.kind === "route" && r.outcome === "tool" && (
        <div className="space-y-1" data-testid="agent-chosen">
          <p className="text-sm font-medium">
            {t("automations.agent.chosen", { tool: r.tool })}
          </p>
          {r.action && (
            <p className="text-xs text-text-muted">
              {t("automations.agent.runsAction", { action: r.action })}
            </p>
          )}
          {r.arguments.length > 0 && (
            <dl
              className="grid gap-x-3 gap-y-0.5 text-xs sm:grid-cols-[8rem_1fr]"
              data-testid="agent-arguments"
            >
              {r.arguments.map(([k, v]) => (
                <React.Fragment key={k}>
                  <dt className="text-text-muted">{k}</dt>
                  <dd className="min-w-0 whitespace-pre-wrap break-words">
                    {v}
                  </dd>
                </React.Fragment>
              ))}
            </dl>
          )}
          {r.recipients.length > 0 && (
            <p className="text-xs" data-testid="agent-recipients">
              {t("automations.agent.recipients", {
                list: r.recipients.join(", "),
              })}
            </p>
          )}
          <p className="text-xs text-text-muted" data-testid="agent-reason">
            {t("automations.agent.passed")}
          </p>
        </div>
      )}
      {r.kind === "route" && r.outcome !== "tool" && (
        <div className="space-y-1" data-testid="agent-no-action">
          <p className="text-sm font-medium">
            {t("automations.agent.noAction")}
          </p>
          <p className="text-xs" data-testid="agent-reason">
            {r.reasonText || t("automations.agent.noReason")}
          </p>
          {r.signals.length > 0 && (
            <p className="text-xs text-text-muted">
              {t("automations.agent.signals", { list: r.signals.join(", ") })}
            </p>
          )}
        </div>
      )}
      {r.kind === "extract" && (
        <div className="space-y-1" data-testid="agent-extracted">
          <p className="text-sm font-medium">
            {r.summary ||
              (r.todos.length + r.deadlines.length + r.decisions.length > 0
                ? t("automations.agent.counts", {
                    todos: r.todos.length,
                    deadlines: r.deadlines.length,
                    decisions: r.decisions.length,
                  })
                : t("automations.agent.noItems"))}
          </p>
          <List
            label={t("automations.agent.kinds.todos")}
            items={r.todos}
            testId="agent-todos"
          />
          <List
            label={t("automations.agent.kinds.deadlines")}
            items={r.deadlines}
            testId="agent-deadlines"
          />
          <List
            label={t("automations.agent.kinds.decisions")}
            items={r.decisions}
            testId="agent-decisions"
          />
        </div>
      )}
      {r.notes.length > 0 && (
        <ul className="list-disc ps-5 text-xs text-text-muted">
          {r.notes.map((n, i) => (
            <li key={`${i}-${n}`}>{n}</li>
          ))}
        </ul>
      )}
      {meta && r.provenance && (
        <p className="text-xs text-text-muted" data-testid="agent-meta">
          {r.provenance.model || t("automations.runs.noModel")}
          {r.provenance.durationMs != null
            ? ` · ${seconds(r.provenance.durationMs)}`
            : ""}
          {r.provenance.promptTokens != null ||
          r.provenance.completionTokens != null
            ? ` · ${t("automations.agent.tokens", {
                input: r.provenance.promptTokens ?? 0,
                output: r.provenance.completionTokens ?? 0,
              })}`
            : ""}
          {confidence != null
            ? ` · ${t("automations.agent.confidence", { value: confidence })}`
            : ""}
        </p>
      )}
    </div>
  );
};
