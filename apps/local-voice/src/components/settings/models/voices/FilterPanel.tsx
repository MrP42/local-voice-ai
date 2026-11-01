import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../ui/Button";
import {
  EMPTY_FILTERS,
  activeFilterCount,
  facetCounts,
  installedCount,
  providerStatus,
  type DeviceGroup,
  type FacetKey,
  type Filters,
  type LicenseKind,
  type VoiceItem,
} from "./model";
import { useVoiceLabels } from "./labels";
import type { TtsProviderEntry } from "@/bindings";

interface Option {
  value: string;
  label: string;
  dot?: "ready" | "warn" | "off";
}

const DOT: Record<NonNullable<Option["dot"]>, string> = {
  ready: "bg-green-500",
  warn: "bg-amber-500",
  off: "bg-mid-gray/50",
};

const Facet: React.FC<{
  title: string;
  facet: FacetKey;
  options: Option[];
  counts: Map<string, number>;
  selected: ReadonlySet<string>;
  onToggle: (value: string) => void;
}> = ({ title, facet, options, counts, selected, onToggle }) => {
  if (options.length === 0) return null;
  return (
    <fieldset className="min-w-0 space-y-0.5" data-facet={facet}>
      <legend className="mb-1 text-xs font-semibold text-text/60">
        {title}
      </legend>
      {options.map((o) => {
        const n = counts.get(o.value) ?? 0;
        return (
          <label
            key={o.value}
            className={`flex min-h-[44px] cursor-pointer items-center gap-2 rounded px-1 text-sm focus-within:ring-1 focus-within:ring-logo-primary md:min-h-8 ${
              n === 0 && !selected.has(o.value) ? "text-text/40" : "text-text"
            }`}
          >
            <input
              type="checkbox"
              className="h-4 w-4 shrink-0 accent-logo-primary"
              checked={selected.has(o.value)}
              onChange={() => onToggle(o.value)}
              data-facet-value={o.value}
            />
            {o.dot && (
              <span
                aria-hidden="true"
                className={`h-2 w-2 shrink-0 rounded-full ${DOT[o.dot]}`}
              />
            )}
            <span className="min-w-0 flex-1 truncate">{o.label}</span>
            <span className="tabular-nums text-xs text-text/50">{n}</span>
          </label>
        );
      })}
    </fieldset>
  );
};

const toggled = <T,>(set: ReadonlySet<T>, v: T): Set<T> => {
  const next = new Set(set);
  if (next.has(v)) next.delete(v);
  else next.add(v);
  return next;
};

const DEVICE_ORDER: DeviceGroup[] = ["cpu", "gpu", "server"];
const LICENSE_ORDER: LicenseKind[] = ["free", "nonCommercial", "unverified"];

/** Filterspalte mit Zaehlern: Sprache, Geraet, Anbieter, Lizenz, nur installierte. */
export const FilterPanel: React.FC<{
  items: readonly VoiceItem[];
  entries: readonly TtsProviderEntry[];
  filters: Filters;
  onChange: (next: Filters) => void;
}> = ({ items, entries, filters, onChange }) => {
  const { t } = useTranslation();
  const label = useVoiceLabels();
  const id = useId();
  const count = activeFilterCount(filters);

  const languages = [...new Set(items.map((v) => v.language))]
    .map((code) => ({ value: code, label: label.language(code) }))
    .sort((a, b) => a.label.localeCompare(b.label));
  const devices = DEVICE_ORDER.filter((d) =>
    items.some((v) => v.devices.includes(d)),
  ).map((d) => ({ value: d, label: label.device(d) }));
  const providerIds = new Set(items.map((v) => v.providerId));
  const providers: Option[] = entries
    .filter((e) => providerIds.has(e.descriptor.id))
    .map((e) => {
      const s = providerStatus(e);
      return {
        value: e.descriptor.id,
        label: e.descriptor.label,
        dot: s === "ready" ? "ready" : s === "unavailable" ? "off" : "warn",
      } as Option;
    });
  const licenses = LICENSE_ORDER.filter((k) =>
    items.some((v) => v.license === k),
  ).map((k) => ({ value: k, label: label.license(k) }));

  const counts = (facet: FacetKey) => facetCounts(items, filters, facet);

  return (
    <div className="space-y-4" role="group" aria-labelledby={`${id}-title`}>
      <div className="flex items-center justify-between gap-2">
        <h2 id={`${id}-title`} className="text-sm font-semibold text-text">
          {t("settings.models.voiceManager.filter.title")}
        </h2>
        {count > 0 && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="min-h-[44px] md:min-h-6"
            data-testid="voice-filter-reset"
            onClick={() => onChange({ ...EMPTY_FILTERS, query: filters.query })}
          >
            {t("settings.models.voiceManager.filter.reset")}
          </Button>
        )}
      </div>
      <fieldset className="min-w-0">
        <legend className="mb-1 text-xs font-semibold text-text/60">
          {t("settings.models.voiceManager.filter.status")}
        </legend>
        <label className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded px-1 text-sm focus-within:ring-1 focus-within:ring-logo-primary md:min-h-8">
          <input
            type="checkbox"
            className="h-4 w-4 shrink-0 accent-logo-primary"
            checked={filters.onlyInstalled}
            onChange={() =>
              onChange({ ...filters, onlyInstalled: !filters.onlyInstalled })
            }
            data-testid="voice-filter-installed"
          />
          <span className="min-w-0 flex-1">
            {t("settings.models.voiceManager.filter.onlyInstalled")}
          </span>
          <span className="tabular-nums text-xs text-text/50">
            {installedCount(items, filters)}
          </span>
        </label>
      </fieldset>
      <Facet
        title={t("settings.models.voiceManager.filter.language")}
        facet="languages"
        options={languages}
        counts={counts("languages")}
        selected={filters.languages}
        onToggle={(v) =>
          onChange({ ...filters, languages: toggled(filters.languages, v) })
        }
      />
      <Facet
        title={t("settings.models.voiceManager.filter.device")}
        facet="devices"
        options={devices}
        counts={counts("devices")}
        selected={filters.devices}
        onToggle={(v) =>
          onChange({
            ...filters,
            devices: toggled(filters.devices, v as DeviceGroup),
          })
        }
      />
      <Facet
        title={t("settings.models.voiceManager.filter.provider")}
        facet="providers"
        options={providers}
        counts={counts("providers")}
        selected={filters.providers}
        onToggle={(v) =>
          onChange({ ...filters, providers: toggled(filters.providers, v) })
        }
      />
      <Facet
        title={t("settings.models.voiceManager.filter.license")}
        facet="licenses"
        options={licenses}
        counts={counts("licenses")}
        selected={filters.licenses}
        onToggle={(v) =>
          onChange({
            ...filters,
            licenses: toggled(filters.licenses, v as LicenseKind),
          })
        }
      />
    </div>
  );
};
