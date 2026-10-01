import React, { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Upload } from "lucide-react";
import {
  commands,
  type IntegrationView,
  type PendingApproval,
  type WorkflowItem,
  type WorkflowTemplate,
} from "@/bindings";
import { Button } from "../ui/Button";
import { TabList } from "../ui/TabList";
import { PlanView, parsePlan, type Plan } from "./PlanView";
import { RunDetail, RunList } from "./RunViews";
import { ExportDialog, ImportDialog, TemplateDialog } from "./TransferDialogs";
import { WorkflowEditor } from "./WorkflowEditor";
import { WorkflowList } from "./WorkflowList";
import { emptyDef, parseDef, type Def } from "./model";
import { errorOf, useCatalog, useWorkflows } from "./useAutomations";

type Screen =
  | { name: "list" }
  | { name: "editor"; key: number; id: string | null; initial: Def }
  | { name: "run"; runId: string };

type SubTab = "flows" | "runs";
const isSubTab = (v: string): v is SubTab => v === "flows" || v === "runs";

interface AutomationsPanelProps {
  integrations: IntegrationView[];
  pending: PendingApproval[];
  /** Oeffnet den vorhandenen Freigabedialog der Seite. */
  onOpenApprovals: () => void;
  /** Wird hochgezaehlt, wenn dort eine Freigabe entschieden wurde (Daten neu laden). */
  version: number;
}

/**
 * „Automationen“ (B7, Goal Workflow-Automation): Ablaeufe aus Ausloeser, Bedingung und
 * Schritten. Die Seite zeigt und bearbeitet nur, was die Engine hat; Pruefung, Trockenlauf,
 * Rechte und Einwilligung entscheidet das Backend. Entschieden werden Freigaben im Dialog
 * der Seite „Integrationen“, nicht hier.
 */
export const AutomationsPanel: React.FC<AutomationsPanelProps> = ({
  integrations,
  pending,
  onOpenApprovals,
  version,
}) => {
  const { t } = useTranslation();
  const { catalog, templates, failed: catalogFailed } = useCatalog();
  const [localVersion, setLocalVersion] = useState(0);
  const bump = useCallback(() => setLocalVersion((n) => n + 1), []);
  const { items, status, loaded, failed, upsert, reload } = useWorkflows(
    version + localVersion,
  );
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [sub, setSub] = useState<SubTab>("flows");
  const [runFilter, setRunFilter] = useState<{
    id: string;
    name: string;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [templateOpen, setTemplateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [exportItem, setExportItem] = useState<WorkflowItem | null>(null);
  const [planOpen, setPlanOpen] = useState(false);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [planFailed, setPlanFailed] = useState(false);
  const [planName, setPlanName] = useState("");
  const editorKey = useRef(0);

  const flash = (message: string) => {
    setError(null);
    setNotice(message);
  };
  const fail = (message: string) => {
    setNotice(null);
    setError(message);
  };

  const openEditor = (id: string | null, initial: Def) => {
    editorKey.current += 1;
    setNotice(null);
    setError(null);
    setScreen({ name: "editor", key: editorKey.current, id, initial });
  };

  const pickTemplate = (tpl: WorkflowTemplate | null) => {
    setTemplateOpen(false);
    if (!tpl) return openEditor(null, emptyDef());
    const def = parseDef(tpl.definition_json);
    if (!def) return fail(t("automations.errors.templateBroken"));
    openEditor(null, def);
  };

  const showPlan = async (
    definitionJson: string,
    workflowId: string | null,
    name: string,
  ) => {
    setPlan(null);
    setPlanFailed(false);
    setPlanName(name);
    setPlanOpen(true);
    try {
      const r = await commands.workflowPlan(definitionJson, workflowId);
      const parsed = r.status === "ok" ? parsePlan(r.data) : null;
      if (parsed) setPlan(parsed);
      else setPlanFailed(true);
    } catch {
      setPlanFailed(true);
    }
  };

  /** Ein Handgriff auf einem Ablauf: Antwort einsetzen, sonst Fehlertext zeigen. */
  const withItem = async (
    item: WorkflowItem,
    fn: () => Promise<
      { status: "ok"; data: WorkflowItem } | { status: "error"; error: string }
    >,
    done?: string,
  ) => {
    setBusyId(item.id);
    try {
      const r = await fn();
      if (r.status === "ok") {
        upsert(r.data);
        if (done) flash(done);
      } else {
        fail(r.error);
      }
    } catch (e) {
      fail(errorOf(e) || t("automations.errors.generic"));
    }
    setBusyId(null);
  };

  const start = async (item: WorkflowItem, dryRun: boolean) => {
    setBusyId(item.id);
    try {
      const r = await commands.workflowRunStart(item.id, dryRun, null);
      if (r.status === "ok") {
        bump();
        setScreen({ name: "run", runId: r.data.run_id });
      } else {
        fail(r.error);
      }
    } catch (e) {
      fail(errorOf(e) || t("automations.errors.generic"));
    }
    setBusyId(null);
  };

  const remove = async (item: WorkflowItem) => {
    setBusyId(item.id);
    try {
      const r = await commands.workflowDelete(item.id);
      if (r.status === "ok") {
        await reload();
        flash(t("automations.deleted", { name: item.name }));
      } else {
        fail(r.error);
      }
    } catch (e) {
      fail(errorOf(e) || t("automations.errors.generic"));
    }
    setBusyId(null);
  };

  const edit = (item: WorkflowItem) => {
    const def = parseDef(item.definition_json);
    if (!def) return fail(t("automations.errors.unreadable"));
    openEditor(item.id, def);
  };

  const toList = () => {
    setScreen({ name: "list" });
    bump();
  };

  if (!catalog) {
    return (
      <p
        className={`text-sm ${catalogFailed ? "text-status-red" : "text-text-muted"}`}
        role={catalogFailed ? "alert" : "status"}
        data-testid="automations-loading"
      >
        {catalogFailed ? t("automations.loadError") : t("automations.loading")}
      </p>
    );
  }

  return (
    <div className="space-y-4" data-testid="automations-panel">
      {screen.name === "list" && (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <TabList
              tabs={[
                { id: "flows", label: t("automations.tabs.flows") },
                { id: "runs", label: t("automations.tabs.runs") },
              ]}
              value={sub}
              onChange={(v) => isSubTab(v) && setSub(v)}
              ariaLabel={t("automations.title")}
              compact
            />
            {sub === "flows" && (
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  onClick={() => setTemplateOpen(true)}
                  data-testid="automations-new"
                >
                  <Plus size={14} aria-hidden="true" />
                  {t("automations.new")}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => setImportOpen(true)}
                  data-testid="automations-import"
                >
                  <Upload size={14} aria-hidden="true" />
                  {t("automations.import.open")}
                </Button>
              </div>
            )}
          </div>
          {notice && (
            <p
              className="text-sm"
              role="status"
              data-testid="automations-notice"
            >
              {notice}
            </p>
          )}
          {error && (
            <p
              className="text-sm text-status-red"
              role="alert"
              data-testid="automations-error"
            >
              {error}
            </p>
          )}
          {sub === "flows" ? (
            <WorkflowList
              items={items}
              status={status}
              loaded={loaded}
              failed={failed}
              catalog={catalog}
              busyId={busyId}
              onNew={() => setTemplateOpen(true)}
              onEdit={edit}
              onPlan={(item) =>
                void showPlan(item.definition_json, item.id, item.name)
              }
              onRuns={(item) => {
                setRunFilter({ id: item.id, name: item.name });
                setSub("runs");
              }}
              onStart={(item, dry) => void start(item, dry)}
              onEnabled={(item, enabled) =>
                void withItem(item, () =>
                  commands.workflowSetEnabled(item.id, enabled),
                )
              }
              onArmed={(item, armed) =>
                void withItem(
                  item,
                  () => commands.workflowSetArmed(item.id, armed),
                  armed
                    ? t("automations.mode.armedNow", { name: item.name })
                    : t("automations.mode.disarmedNow", { name: item.name }),
                )
              }
              onExport={setExportItem}
              onDelete={(item) => void remove(item)}
            />
          ) : (
            <RunList
              workflowId={runFilter?.id ?? null}
              workflowName={runFilter?.name ?? null}
              version={version + localVersion}
              onOpen={(runId) => setScreen({ name: "run", runId })}
              onClearFilter={() => setRunFilter(null)}
            />
          )}
        </>
      )}

      {screen.name === "editor" && (
        <WorkflowEditor
          key={screen.key}
          catalog={catalog}
          integrations={integrations}
          workflowId={screen.id}
          initial={screen.initial}
          onSaved={(item) => {
            upsert(item);
            setScreen((s) => (s.name === "editor" ? { ...s, id: item.id } : s));
          }}
          onBack={toList}
          onPlan={(json, id, name) => void showPlan(json, id, name)}
        />
      )}

      {screen.name === "run" && (
        <RunDetail
          runId={screen.runId}
          version={version + localVersion}
          pending={pending}
          onBack={toList}
          onOpenApprovals={onOpenApprovals}
          onChanged={bump}
        />
      )}

      <TemplateDialog
        open={templateOpen}
        onOpenChange={setTemplateOpen}
        templates={templates}
        onPick={pickTemplate}
      />
      <ImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={(item) => {
          upsert(item);
          flash(t("automations.import.done", { name: item.name }));
        }}
      />
      <ExportDialog item={exportItem} onClose={() => setExportItem(null)} />
      <PlanView
        open={planOpen}
        onOpenChange={setPlanOpen}
        plan={plan}
        failed={planFailed}
        name={planName}
      />
    </div>
  );
};
