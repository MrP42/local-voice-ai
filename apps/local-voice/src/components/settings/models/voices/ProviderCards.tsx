import React from "react";
import { useTranslation } from "react-i18next";
import { Download, RefreshCw, Settings2 } from "lucide-react";
import { Switch } from "@fluentui/react-components";
import type { TtsProviderEntry } from "@/bindings";
import { Button } from "../../../ui/Button";
import { RowChip } from "../ModelRow";
import { hasEnableSwitch } from "../../tts/providers/providerModel";
import type { TtsProvidersState } from "../../tts/providers/useTtsProviders";
import { deviceGroups, providerStatus, type ProviderStatus } from "./model";
import { useVoiceLabels } from "./labels";

const STATUS_TONE: Record<ProviderStatus, "success" | "warning" | "neutral"> = {
  ready: "success",
  notReady: "warning",
  blocked: "warning",
  notEnabled: "neutral",
  unavailable: "neutral",
};

/** Was fehlt, damit der Anbieter benutzt werden darf (Text wie in der Anbieterzeile). */
function missingText(
  entry: TtsProviderEntry,
  t: (key: string) => string,
): string | null {
  if (entry.blocked === null) return null;
  if (entry.descriptor.opt_in && !entry.enabled)
    return t("tts.providers.blocked.notEnabled");
  if (entry.license_needs_ack && !entry.license_acked)
    return t("tts.providers.blocked.needsLicense");
  if (entry.remote && !entry.allow_remote)
    return t("tts.providers.blocked.needsRemote");
  return entry.blocked.message;
}

const ProviderCard: React.FC<{
  entry: TtsProviderEntry;
  voiceCount: number | null;
  providers: TtsProvidersState;
  onOpen: (id: string) => void;
}> = ({ entry, voiceCount, providers, onOpen }) => {
  const { t } = useTranslation();
  const label = useVoiceLabels();
  const { descriptor, health } = entry;
  const id = descriptor.id;
  const status = providerStatus(entry);
  const busy = providers.busy === id;
  const problem =
    missingText(entry, t) ??
    (!health.available
      ? [health.reason, health.hint].filter(Boolean).join(" ")
      : null);
  return (
    <li
      className="flex flex-col gap-2 rounded-lg border border-mid-gray/25 p-3"
      data-provider-card={id}
      data-status={status}
    >
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-semibold text-text">{descriptor.label}</h3>
        <RowChip tone={STATUS_TONE[status]}>
          {t(`settings.models.voiceManager.providers.status.${status}`)}
        </RowChip>
        <RowChip>{t(`tts.providers.tier.${descriptor.tier}`)}</RowChip>
        {entry.selected && (
          <RowChip tone="accent">{t("tts.providers.selected")}</RowChip>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {deviceGroups(
          health.device ? [health.device] : descriptor.caps.devices,
        ).map((d) => (
          <RowChip key={d}>{label.device(d)}</RowChip>
        ))}
        {voiceCount !== null && (
          <RowChip>
            {t("settings.models.voiceManager.providers.voiceCount", {
              count: voiceCount,
            })}
          </RowChip>
        )}
        {descriptor.license.non_commercial && (
          <RowChip tone="warning">{label.license("nonCommercial")}</RowChip>
        )}
        <RowChip tone={descriptor.license.verified ? "success" : "warning"}>
          {label.license(descriptor.license.verified ? "free" : "unverified")}
        </RowChip>
      </div>
      {problem && (
        <p className="text-xs text-amber-700 dark:text-amber-400">{problem}</p>
      )}
      <div className="mt-auto flex flex-wrap items-center gap-2 pt-1">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="min-h-[44px] md:min-h-8"
          data-testid={`tts-provider-open-${id}`}
          aria-label={t("settings.models.voiceManager.providers.openNamed", {
            name: descriptor.label,
          })}
          onClick={() => onOpen(id)}
        >
          <Settings2 className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.models.voiceManager.providers.details")}
        </Button>
        {(id === "piper" || id === "phonon") && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="min-h-[44px] md:min-h-8"
            data-testid={`tts-provider-downloads-${id}`}
            onClick={() => onOpen(id)}
          >
            <Download className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.models.voiceManager.providers.manageDownloads")}
          </Button>
        )}
        {hasEnableSwitch(entry) && (
          <Switch
            checked={entry.enabled}
            disabled={busy}
            label={t("tts.providers.enable")}
            labelPosition="before"
            data-testid={`tts-provider-card-enable-${id}`}
            onChange={(_e, data) => void providers.enable(id, data.checked)}
          />
        )}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="ms-auto min-h-[44px] md:min-h-8"
          disabled={busy}
          data-testid={`tts-provider-card-recheck-${id}`}
          aria-label={t("settings.models.voiceManager.providers.recheckNamed", {
            name: descriptor.label,
          })}
          onClick={() => void providers.recheck(id)}
        >
          <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          {t("tts.providers.recheck")}
        </Button>
      </div>
    </li>
  );
};

/** Anbieter als Karten; Einstellungen und Freigaben stehen im Seitenblatt. */
export const ProviderCards: React.FC<{
  providers: TtsProvidersState;
  voiceCounts: Map<string, number> | null;
  query: string;
  onOpen: (id: string) => void;
}> = ({ providers, voiceCounts, query, onOpen }) => {
  const { t } = useTranslation();
  const { entries, checking, loadError } = providers;
  const q = query.trim().toLowerCase();
  const shown = (entries ?? []).filter(
    (e) =>
      !q ||
      `${e.descriptor.label} ${e.descriptor.id}`.toLowerCase().includes(q),
  );
  return (
    <section
      aria-labelledby="voice-providers-title"
      className="space-y-3"
      data-testid="voice-providers-view"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2
            id="voice-providers-title"
            className="text-sm font-semibold text-text"
          >
            {t("settings.models.voiceManager.providers.title")}
          </h2>
          <p className="text-xs text-text/60">
            {t("tts.providers.description")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {checking && (
            <span
              role="status"
              className="text-xs text-text/70"
              data-testid="tts-providers-checking"
            >
              {t("tts.providers.checking")}
            </span>
          )}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="min-h-[44px] md:min-h-8"
            disabled={checking}
            data-testid="tts-providers-recheck-all"
            onClick={() => void providers.reload()}
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
            {t("tts.providers.recheckAll")}
          </Button>
        </div>
      </div>
      {entries === null && !loadError && (
        <p role="status" className="text-sm text-text/60">
          {t("tts.providers.checking")}
        </p>
      )}
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,19rem),1fr))] gap-3">
        {shown.map((entry) => (
          <ProviderCard
            key={entry.descriptor.id}
            entry={entry}
            voiceCount={voiceCounts?.get(entry.descriptor.id) ?? null}
            providers={providers}
            onOpen={onOpen}
          />
        ))}
      </ul>
      {entries && entries.length === 0 && (
        <p className="text-xs text-text/70">{t("tts.providers.empty")}</p>
      )}
    </section>
  );
};
