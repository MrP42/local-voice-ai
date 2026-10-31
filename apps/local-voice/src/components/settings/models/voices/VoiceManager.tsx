import React, { useCallback, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LayoutGrid, List, SlidersHorizontal } from "lucide-react";
import { usePersistentState } from "@/hooks/usePersistentState";
import { openSettingsTab } from "@/lib/openSettingsTab";
import { Button } from "../../../ui/Button";
import { Select } from "../../../ui/Select";
import { ActiveBar } from "../ActiveBar";
import { RowChip } from "../ModelRow";
import { useTtsProviders } from "../../tts/providers/useTtsProviders";
import {
  LicenseDialog,
  RemoteDialog,
} from "../../tts/providers/ProviderDialogs";
import { FilterPanel } from "./FilterPanel";
import { ProviderCards } from "./ProviderCards";
import { ProviderErrors } from "./ProviderErrors";
import { ProviderSheet } from "./ProviderSheet";
import {
  VoiceCard,
  VoiceListRow,
  VoicePlay,
  type VoiceActions,
} from "./VoiceCard";
import {
  EMPTY_FILTERS,
  SORT_KEYS,
  activeFilterCount,
  applyFilters,
  sortVoices,
  type Filters,
  type SortKey,
} from "./model";
import { useVoiceLabels } from "./labels";
import { useVoiceCatalog } from "./useVoiceCatalog";

export type VoiceView = "voices" | "providers";
type Layout = "cards" | "list";
const VIEWS: VoiceView[] = ["voices", "providers"];
const LAYOUTS: Layout[] = ["cards", "list"];
export const VIEW_KEY = "models.voices.view";

/**
 * Zur Vorlesen-Verwaltung unter Modelle springen und dort eine Ansicht zeigen
 * (Stimmen oder Anbieter). Die Ansicht wird vor dem Sprung gemerkt.
 */
export const openVoiceManager = (view: VoiceView) => {
  try {
    window.localStorage.setItem(`lva.ui.${VIEW_KEY}`, view);
    window.localStorage.setItem("lva.ui.models.area", "voices");
  } catch {
    /* ohne Speicher landet man in der zuletzt gewaehlten Ansicht */
  }
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "models" } }),
  );
};

const PANEL_ID = "voice-view-panel";
const tabId = (v: VoiceView) => `voice-view-tab-${v}`;

const ViewSwitch: React.FC<{
  value: VoiceView;
  counts: Record<VoiceView, number | null>;
  onChange: (v: VoiceView) => void;
}> = ({ value, counts, onChange }) => {
  const { t } = useTranslation();
  const listRef = useRef<HTMLDivElement>(null);
  const onKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>, i: number) => {
    if (!["ArrowRight", "ArrowLeft", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const rtl = e.currentTarget.closest("[dir]")?.getAttribute("dir") === "rtl";
    const step = (e.key === "ArrowRight" ? 1 : -1) * (rtl ? -1 : 1);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? VIEWS.length - 1
          : (i + step + VIEWS.length) % VIEWS.length;
    onChange(VIEWS[next]);
    listRef.current
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [next]?.focus();
  };
  return (
    <div
      ref={listRef}
      role="tablist"
      aria-label={t("settings.models.voiceManager.view.label")}
      className="inline-flex overflow-hidden rounded-lg border border-mid-gray/40"
      data-testid="voice-view-switch"
    >
      {VIEWS.map((v, i) => (
        <button
          key={v}
          id={tabId(v)}
          type="button"
          role="tab"
          aria-selected={value === v}
          aria-controls={PANEL_ID}
          tabIndex={value === v ? 0 : -1}
          data-testid={`voice-view-${v}`}
          onClick={() => onChange(v)}
          onKeyDown={(e) => onKeyDown(e, i)}
          className={`min-h-[44px] px-4 text-sm font-medium focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-logo-primary md:min-h-8 ${
            value === v
              ? "bg-logo-primary text-on-accent"
              : "text-text hover:bg-mid-gray/15"
          }`}
        >
          {t(`settings.models.voiceManager.view.${v}`)}
          {counts[v] !== null && (
            <span className="ms-1 tabular-nums">· {counts[v]}</span>
          )}
        </button>
      ))}
    </div>
  );
};

/**
 * Vorlesen unter Modelle (Variante B): ein Umschalter zwischen Stimmen und
 * Anbietern -- eine Ansicht, kein Einstellungsreiter. Stimmen: Filterspalte mit
 * Zaehlern und Karten oder Liste ueber alle Anbieter, mit Hoerprobe und Auswahl.
 * Anbieter: Karten, deren Seitenblatt die bisherigen Einstellungen enthaelt.
 */
export const VoiceManager: React.FC<{ query: string }> = ({ query }) => {
  const { t, i18n } = useTranslation();
  const label = useVoiceLabels();
  const filterId = useId();
  const providers = useTtsProviders();
  const catalog = useVoiceCatalog(
    providers.entries,
    providers.entries === null && providers.loadError !== null,
  );
  const [view, setView] = usePersistentState<VoiceView>(
    VIEW_KEY,
    "voices",
    (v) => (VIEWS as string[]).includes(v),
  );
  const [layout, setLayout] = usePersistentState<Layout>(
    "models.voices.layout",
    "cards",
    (v) => (LAYOUTS as string[]).includes(v),
  );
  const [sort, setSort] = usePersistentState<SortKey>(
    "models.voices.sort",
    "recommended",
    (v) => (SORT_KEYS as string[]).includes(v),
  );
  const [filters, setFilters] = useState<Filters>(EMPTY_FILTERS);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [sheetId, setSheetId] = useState<string | null>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const openSheet = useCallback((id: string) => {
    openerRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setSheetId(id);
  }, []);
  const closeSheet = useCallback(() => {
    setSheetId(null);
    const opener = openerRef.current;
    openerRef.current = null;
    // Nach dem Schliessen (Fokusfalle weg) zurueck zum Ausloeser.
    if (opener)
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          if (opener.isConnected) opener.focus();
        }),
      );
  }, []);

  const effective = useMemo<Filters>(
    () => ({ ...filters, query }),
    [filters, query],
  );
  const shown = useMemo(
    () =>
      sortVoices(applyFilters(catalog.items, effective), sort, i18n.language),
    [catalog.items, effective, sort, i18n.language],
  );
  const voiceCounts = useMemo(() => {
    if (catalog.loading) return null;
    const map = new Map<string, number>();
    for (const v of catalog.items)
      map.set(v.providerId, (map.get(v.providerId) ?? 0) + 1);
    return map;
  }, [catalog.items, catalog.loading]);

  const actions: VoiceActions = useMemo(
    () => ({ catalog, openProvider: openSheet }),
    [catalog, openSheet],
  );

  const active = catalog.items.find((v) => v.selected) ?? null;
  const dialogEntry =
    providers.dialog && providers.entries
      ? (providers.entries.find(
          (e) => e.descriptor.id === providers.dialog?.id,
        ) ?? null)
      : null;
  const filterCount = activeFilterCount(filters);

  return (
    <div className="space-y-4" data-testid="voice-manager">
      <ActiveBar
        label={t("settings.models.voiceManager.reading")}
        name={active?.name ?? catalog.activeName}
        chips={
          active && (
            <>
              <RowChip tone="accent">{active.providerLabel}</RowChip>
              <RowChip>{label.language(active.language)}</RowChip>
              {active.devices.map((d) => (
                <RowChip key={d}>{label.device(d)}</RowChip>
              ))}
            </>
          )
        }
        aside={
          <span className="flex items-center gap-2">
            {active && <VoicePlay voice={active} />}
            <span>{t("settings.models.voiceManager.readingScope")}</span>
          </span>
        }
      />

      <div className="flex flex-wrap items-center justify-between gap-3">
        <ViewSwitch
          value={view}
          onChange={setView}
          counts={{
            voices: catalog.loading ? null : catalog.items.length,
            providers: providers.entries ? providers.entries.length : null,
          }}
        />
      </div>

      <ProviderErrors providers={providers} />

      <div
        id={PANEL_ID}
        role="tabpanel"
        aria-labelledby={tabId(view)}
        tabIndex={-1}
      >
        {view === "providers" ? (
          <ProviderCards
            providers={providers}
            voiceCounts={voiceCounts}
            query={query}
            onOpen={openSheet}
          />
        ) : (
          <section
            aria-label={t("settings.models.voiceManager.view.voices")}
            className="grid gap-4 min-[1100px]:grid-cols-[15rem_minmax(0,1fr)]"
            data-testid="voice-voices-view"
          >
            <div className="min-w-0 min-[1100px]:self-start">
              <div className="min-[1100px]:hidden">
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  className="min-h-[44px]"
                  aria-expanded={filtersOpen}
                  aria-controls={filterId}
                  data-testid="voice-filter-toggle"
                  onClick={() => setFiltersOpen((o) => !o)}
                >
                  <SlidersHorizontal
                    className="h-3.5 w-3.5"
                    aria-hidden="true"
                  />
                  {filterCount > 0
                    ? t("settings.models.voiceManager.filter.buttonCount", {
                        count: filterCount,
                      })
                    : t("settings.models.voiceManager.filter.button")}
                </Button>
              </div>
              <div
                id={filterId}
                className={`${filtersOpen ? "mt-3 block" : "hidden"} rounded-lg border border-mid-gray/25 p-3 min-[1100px]:mt-0 min-[1100px]:block`}
                data-testid="voice-filter-panel"
              >
                <FilterPanel
                  items={catalog.items}
                  entries={providers.entries ?? []}
                  filters={effective}
                  onChange={(next) => setFilters({ ...next, query: "" })}
                />
              </div>
            </div>

            <div className="min-w-0 space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div
                  role="group"
                  aria-label={t("settings.models.voiceManager.layout.label")}
                  className="inline-flex overflow-hidden rounded-lg border border-mid-gray/40"
                >
                  {LAYOUTS.map((l) => {
                    const Icon = l === "cards" ? LayoutGrid : List;
                    return (
                      <button
                        key={l}
                        type="button"
                        aria-pressed={layout === l}
                        aria-label={t(
                          `settings.models.voiceManager.layout.${l}`,
                        )}
                        title={t(`settings.models.voiceManager.layout.${l}`)}
                        data-testid={`voice-layout-${l}`}
                        onClick={() => setLayout(l)}
                        className={`inline-flex h-[44px] w-[44px] items-center justify-center focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-logo-primary md:h-8 md:w-9 ${
                          layout === l
                            ? "bg-logo-primary text-on-accent"
                            : "text-text hover:bg-mid-gray/15"
                        }`}
                      >
                        <Icon className="h-4 w-4" aria-hidden="true" />
                      </button>
                    );
                  })}
                </div>
                <div className="flex items-center gap-2 text-sm text-text/70">
                  <span id={`${filterId}-sort`}>
                    {t("settings.models.voiceManager.sort.label")}
                  </span>
                  <Select
                    className="voice-sort w-44"
                    value={sort}
                    ariaLabel={t("settings.models.voiceManager.sort.label")}
                    isClearable={false}
                    options={SORT_KEYS.map((k) => ({
                      value: k,
                      label: t(`settings.models.voiceManager.sort.${k}`),
                    }))}
                    onChange={(v) => v && setSort(v as SortKey)}
                  />
                </div>
              </div>
              <p
                role="status"
                aria-live="polite"
                className="text-xs text-text/60"
                data-testid="voice-count"
              >
                {catalog.loading
                  ? t("settings.models.voiceManager.loading")
                  : t("settings.models.voiceManager.count", {
                      shown: shown.length,
                      total: catalog.items.length,
                    })}
              </p>

              {!catalog.loading && catalog.items.length === 0 && (
                <p className="rounded-lg border border-dashed border-mid-gray/30 p-4 text-sm text-text/70">
                  {t("settings.models.voiceManager.empty")}
                </p>
              )}
              {!catalog.loading &&
                catalog.items.length > 0 &&
                shown.length === 0 && (
                  <div className="space-y-2 rounded-lg border border-dashed border-mid-gray/30 p-4 text-sm text-text/70">
                    <p>{t("settings.models.voiceManager.noMatch")}</p>
                    {filterCount > 0 && (
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        className="min-h-[44px] md:min-h-8"
                        onClick={() => setFilters(EMPTY_FILTERS)}
                      >
                        {t("settings.models.voiceManager.filter.reset")}
                      </Button>
                    )}
                  </div>
                )}

              {layout === "cards" ? (
                <ul
                  className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,17rem),1fr))] gap-3"
                  data-testid="voice-gallery"
                >
                  {shown.map((v) => (
                    <VoiceCard key={v.key} voice={v} actions={actions} />
                  ))}
                </ul>
              ) : (
                <ul
                  className="overflow-hidden rounded-lg border border-mid-gray/20"
                  data-testid="voice-list"
                >
                  {shown.map((v) => (
                    <VoiceListRow key={v.key} voice={v} actions={actions} />
                  ))}
                </ul>
              )}

              {catalog.providersWithoutVoices.length > 0 && (
                <p className="flex flex-wrap items-center gap-x-2 text-xs text-text/60">
                  {t("settings.models.voiceManager.noVoicesFrom", {
                    names: catalog.providersWithoutVoices
                      .map((e) => e.descriptor.label)
                      .join(", "),
                  })}
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="min-h-[44px] md:min-h-6"
                    data-testid="voice-open-providers"
                    onClick={() => setView("providers")}
                  >
                    {t("settings.models.voiceManager.view.providers")}
                  </Button>
                </p>
              )}
            </div>
          </section>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-dashed border-mid-gray/30 px-3 py-2 text-xs text-text/60">
        <span>{t("settings.models.voiceManager.generalHint")}</span>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="min-h-[44px] md:min-h-8"
          data-testid="voice-open-output"
          onClick={() => openSettingsTab("output")}
        >
          {t("settings.models.voiceManager.openOutput")}
        </Button>
      </div>

      <ProviderSheet
        providers={providers}
        providerId={sheetId}
        onClose={closeSheet}
      />
      {providers.dialog?.kind === "license" && dialogEntry && (
        <LicenseDialog
          entry={dialogEntry}
          onClose={() => providers.setDialog(null)}
          onAcknowledged={async () => {
            const list = await providers.reload();
            providers.setDialog(null);
            providers.advance(dialogEntry.descriptor.id, list);
          }}
          onStale={() => void providers.reload()}
        />
      )}
      {providers.dialog?.kind === "remote" && dialogEntry && (
        <RemoteDialog
          entry={dialogEntry}
          onClose={() => providers.setDialog(null)}
          onAllowed={async () => {
            await providers.reload();
            providers.setDialog(null);
          }}
        />
      )}
    </div>
  );
};

export default VoiceManager;
