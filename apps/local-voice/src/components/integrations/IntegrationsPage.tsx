import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { Button } from "../ui/Button";
import { PageShell } from "../ui/PageShell";
import { SettingsGroup } from "../ui/SettingsGroup";
import { TabList } from "../ui/TabList";
import { CalendarConnectDialog } from "../settings/meetings/CalendarConnectDialog";
import { MeetingMcpSettings } from "../settings/meetings/MeetingMcpSettings";
import { usePersistentState } from "../../hooks/usePersistentState";
import { commands } from "@/bindings";
import { AutomationsPanel } from "../automations/AutomationsPanel";
import { ApprovalDialog } from "./ApprovalDialog";
import { AuditView, EMPTY_AUDIT_FILTER, type AuditFilter } from "./AuditView";
import { CalendarSourceCards, useCalendarSources } from "./CalendarSourceCards";
import { FolderDialog } from "./FolderDialog";
import { IntegrationCard } from "./IntegrationCard";
import { IntegrationCatalog } from "./IntegrationCatalog";
import { IntegrationDetail } from "./IntegrationDetail";
import { TargetDialog } from "./TargetDialog";
import { M365Dialog } from "./M365Dialog";
import { isTargetKind, type CatalogEntry, type TargetKind } from "./model";
import { useIntegrations, usePendingApprovals } from "./useIntegrations";

type Screen =
  { name: "list" } | { name: "catalog" } | { name: "detail"; id: string };

type PageTab = "connections" | "automations" | "audit";
const isPageTab = (value: string): value is PageTab =>
  value === "connections" || value === "automations" || value === "audit";

/**
 * „Integrationen“ (A4, Goal Integrationen): die Verbindungen der App zu
 * Kalendern, Ordnern, Agenten und kommenden Konten, jede mit Richtung und einem
 * Recht je Faehigkeit (aus / fragen / erlaubt). Sie steht zwischen „Modelle“ und
 * „Einstellungen“ und ist die bewusste Ausnahme von der Regel „keine Einstellung
 * in die Seitenleiste“ (Patrick, 30.09.): hier liegen auch die Kalenderquellen
 * und der MCP-Schalter, die vorher unter Einstellungen > Besprechungen standen.
 */
export const IntegrationsPage: React.FC = () => {
  const { t } = useTranslation();
  const register = useIntegrations();
  const calendar = useCalendarSources();
  const [tab, setTab] = usePersistentState<PageTab>(
    "integrations.tab",
    "connections",
    isPageTab,
  );
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [auditFilter, setAuditFilter] =
    useState<AuditFilter>(EMPTY_AUDIT_FILTER);
  const [calendarDialog, setCalendarDialog] = useState(false);
  const [folderDialog, setFolderDialog] = useState(false);
  const [targetKind, setTargetKind] = useState<TargetKind | null>(null);
  const [m365Dialog, setM365Dialog] = useState(false);
  const [approvalsOpen, setApprovalsOpen] = useState(false);
  // B7: wird hochgezaehlt, wenn eine Freigabe entschieden wurde, damit die Automationen
  // ihre Laeufe neu laden.
  const [approvalVersion, setApprovalVersion] = useState(0);
  const approvals = usePendingApprovals(true);

  const { views, loaded, loadFailed, reload, upsert, remove } = register;
  const calendarIds = new Set(
    views.filter((v) => v.calendar_managed).map((v) => v.integration.id),
  );
  const own = views.filter((v) => !v.calendar_managed);

  // Die offene Detailansicht verschwindet, wenn die Integration weg ist
  // (entfernt, oder die Kalenderquelle wurde abgemeldet).
  useEffect(() => {
    if (
      screen.name === "detail" &&
      loaded &&
      !views.some((v) => v.integration.id === screen.id)
    ) {
      setScreen({ name: "list" });
    }
  }, [screen, views, loaded]);

  const toList = useCallback(() => setScreen({ name: "list" }), []);

  const setup = (entry: CatalogEntry) => {
    if (entry.id === "calendar_ics" || entry.id === "calendar_graph") {
      setCalendarDialog(true);
    } else if (entry.id === "folder") {
      setFolderDialog(true);
    } else if (
      entry.kind &&
      (entry.id === "smtp" ||
        entry.id === "obsidian" ||
        entry.id === "wissen" ||
        entry.id === "webhook") &&
      isTargetKind(entry.kind)
    ) {
      setTargetKind(entry.kind);
    } else if (entry.id === "m365") {
      setM365Dialog(true);
    } else if (entry.id === "mcp") {
      setTab("connections");
      toList();
      window.requestAnimationFrame(() =>
        document
          .getElementById("integrations-mcp")
          ?.scrollIntoView({ block: "start" }),
      );
    }
  };

  const showAudit = (id: string) => {
    setAuditFilter({ ...EMPTY_AUDIT_FILTER, integration: id });
    setTab("audit");
    toList();
  };

  const detail =
    screen.name === "detail"
      ? views.find((v) => v.integration.id === screen.id)
      : undefined;
  const inList = screen.name === "list";

  return (
    <PageShell
      title={t("integrations.title")}
      description={t("integrations.description")}
      actions={
        inList && tab === "connections" ? (
          <Button
            size="sm"
            onClick={() => setScreen({ name: "catalog" })}
            data-testid="integrations-add"
          >
            <Plus size={14} aria-hidden="true" />
            {t("integrations.add")}
          </Button>
        ) : undefined
      }
    >
      {approvals.pending.length > 0 && (
        <div
          className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-logo-primary bg-logo-primary/15 px-3 py-2"
          role="status"
          data-testid="approvals-banner"
        >
          <span className="text-sm font-medium">
            {t("integrations.approvals.banner", {
              count: approvals.pending.length,
            })}
          </span>
          <Button
            size="sm"
            onClick={() => setApprovalsOpen(true)}
            data-testid="approvals-open"
          >
            {t("integrations.approvals.review")}
          </Button>
        </div>
      )}

      {inList && (
        <TabList
          tabs={[
            { id: "connections", label: t("integrations.tabs.connections") },
            { id: "automations", label: t("integrations.tabs.automations") },
            { id: "audit", label: t("integrations.tabs.audit") },
          ]}
          value={tab}
          onChange={setTab}
          ariaLabel={t("integrations.title")}
          className="border-b border-mid-gray/20"
        />
      )}

      {screen.name === "catalog" && (
        <IntegrationCatalog onBack={toList} onSetup={setup} />
      )}

      {screen.name === "detail" && detail && (
        <IntegrationDetail
          key={detail.integration.id}
          view={detail}
          onBack={toList}
          onChanged={upsert}
          onRemoved={(id) => {
            remove(id);
            toList();
          }}
          onShowAudit={showAudit}
        />
      )}

      {inList && tab === "automations" && (
        <AutomationsPanel
          integrations={views}
          pending={approvals.pending}
          onOpenApprovals={() => setApprovalsOpen(true)}
          version={approvalVersion}
        />
      )}

      {inList && tab === "audit" && (
        <AuditView
          views={views}
          filter={auditFilter}
          onFilterChange={setAuditFilter}
        />
      )}

      {inList && tab === "connections" && (
        <div className="space-y-6">
          {loadFailed && (
            <p className="text-sm text-status-red" role="alert">
              {t("integrations.loadError")}
            </p>
          )}

          <section className="space-y-3" aria-labelledby="int-sec-conn">
            <h2 id="int-sec-conn" className="text-sm font-semibold">
              {t("integrations.sections.connections")}
            </h2>
            {loaded && own.length === 0 && !loadFailed ? (
              <div
                className="rounded-lg border border-dashed border-mid-gray/40 p-4 text-sm"
                data-testid="integrations-empty"
              >
                <p>{t("integrations.empty")}</p>
                <p className="mt-1 text-text-muted">
                  {t("integrations.emptyHint")}
                </p>
              </div>
            ) : (
              <ul className="grid gap-3 sm:grid-cols-2">
                {own.map((view) => (
                  <IntegrationCard
                    key={view.integration.id}
                    view={view}
                    onOpen={(id) => setScreen({ name: "detail", id })}
                  />
                ))}
              </ul>
            )}
          </section>

          <section className="space-y-3" aria-labelledby="int-sec-cal">
            <div>
              <h2 id="int-sec-cal" className="text-sm font-semibold">
                {t("integrations.sections.calendars")}
              </h2>
              <p className="text-xs text-text-muted">
                {t("integrations.sections.calendarsHint")}
              </p>
            </div>
            <CalendarSourceCards
              calendar={calendar}
              onConnect={() => setCalendarDialog(true)}
              hasRights={(id) => calendarIds.has(id)}
              onOpenRights={(id) => setScreen({ name: "detail", id })}
              onChanged={() => void reload()}
            />
          </section>

          <section
            className="scroll-mt-4 space-y-2"
            id="integrations-mcp"
            aria-labelledby="int-sec-mcp"
          >
            <div>
              <h2 id="int-sec-mcp" className="text-sm font-semibold">
                {t("integrations.sections.mcp")}
              </h2>
              <p className="text-xs text-text-muted">
                {t("integrations.sections.mcpHint")}
              </p>
            </div>
            <SettingsGroup>
              <MeetingMcpSettings />
            </SettingsGroup>
          </section>
        </div>
      )}

      <CalendarConnectDialog
        open={calendarDialog}
        onOpenChange={setCalendarDialog}
        onAdded={(source) => {
          calendar.add(source);
          void reload();
          setScreen({ name: "list" });
          setTab("connections");
        }}
      />
      <FolderDialog
        open={folderDialog}
        onOpenChange={setFolderDialog}
        onCreated={(view) => {
          upsert(view);
          setTab("connections");
          setScreen({ name: "list" });
        }}
      />
      <TargetDialog
        open={targetKind !== null}
        onOpenChange={(next) => {
          if (!next) setTargetKind(null);
        }}
        kind={targetKind ?? "smtp"}
        onSaved={(view) => {
          upsert(view);
          setTab("connections");
          setScreen({ name: "detail", id: view.integration.id });
        }}
      />
      <M365Dialog
        open={m365Dialog}
        onOpenChange={setM365Dialog}
        onCreated={(view) => {
          upsert(view);
          setTab("connections");
          // Weiter zum Anmelden: das Detail des neuen Kontos.
          setScreen({ name: "detail", id: view.integration.id });
        }}
      />
      <ApprovalDialog
        open={approvalsOpen}
        onOpenChange={setApprovalsOpen}
        pending={approvals.pending}
        onDecided={async () => {
          await approvals.reload();
          await reload();
          // B7: ein Lauf, der auf diese Freigabe wartet, geht ohne Wartezeit weiter.
          try {
            await commands.workflowApprovalsChanged();
          } catch {
            /* der Takt der Engine findet die Entscheidung auch so */
          }
          setApprovalVersion((n) => n + 1);
        }}
      />
    </PageShell>
  );
};
