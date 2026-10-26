import React from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Switch } from "@fluentui/react-components";
import type { Device, LicenseInfo, TtsProviderEntry } from "@/bindings";
import Badge from "../../../ui/Badge";
import { Button } from "../../../ui/Button";
import {
  hasEnableSwitch,
  nextApprovalStep,
  safeSourceUrl,
} from "./providerModel";

interface Props {
  entry: TtsProviderEntry;
  expanded: boolean;
  busy: boolean;
  onToggleExpanded: () => void;
  onEnable: (enabled: boolean) => void;
  onRecheck: () => void;
  onApprove: (step: "license" | "remote") => void;
  onRevokeRemote: () => void;
  children?: React.ReactNode;
}

/** Quell-Link einer Lizenzangabe (nur Web-Adressen). */
export const SourceLink: React.FC<{ license: LicenseInfo }> = ({ license }) => {
  const { t } = useTranslation();
  const url = safeSourceUrl(license.source_url);
  if (!url) return null;
  return (
    <button
      type="button"
      onClick={() => void openUrl(url)}
      className="underline hover:text-text"
      data-testid="tts-provider-license-source"
    >
      {t("tts.providers.license.source")}
    </button>
  );
};

/** Pruefstand der Lizenzangabe: "verifiziert am ..." oder deutlich "nicht verifiziert". */
export const VerifiedLine: React.FC<{ license: LicenseInfo }> = ({
  license,
}) => {
  const { t } = useTranslation();
  if (license.verified && license.verified_on) {
    return (
      <span>
        {t("tts.providers.license.verifiedOn", { date: license.verified_on })}{" "}
        <SourceLink license={license} />
      </span>
    );
  }
  return (
    <span className="text-amber-700 dark:text-amber-400">
      <span className="font-medium">
        {t("tts.providers.license.notVerified")}
      </span>{" "}
      <SourceLink license={license} />
    </span>
  );
};

const FACT_ROW = "contents";

/**
 * Eine Zeile der Anbieterliste: Name, Stufe, Zustand und Fakten, Schalter
 * "Aktivieren", Sperrgrund mit Weg zur Freigabe. Reine Darstellung der
 * Faehigkeiten und Zustaende -- keine Sonderfaelle je Anbieter.
 */
export const ProviderRow: React.FC<Props> = ({
  entry,
  expanded,
  busy,
  onToggleExpanded,
  onEnable,
  onRecheck,
  onApprove,
  onRevokeRemote,
  children,
}) => {
  const { t, i18n } = useTranslation();
  const { descriptor, health } = entry;
  const { caps, license } = descriptor;
  const id = descriptor.id;
  const step = nextApprovalStep(entry);

  const languageName = (code: string) => {
    if (code === "und") return t("tts.providers.languageAny");
    try {
      return (
        new Intl.DisplayNames([i18n.language], { type: "language" }).of(code) ??
        code
      );
    } catch {
      return code;
    }
  };
  const deviceName = (d: Device) => t(`tts.providers.device.${d}`);

  const tierVariant =
    descriptor.tier === "supported"
      ? "success"
      : descriptor.tier === "experimental"
        ? "warning"
        : "secondary";

  // Was fehlt, damit der Anbieter benutzt werden darf (leer = nichts).
  const missing = (() => {
    if (entry.blocked === null) return null;
    if (descriptor.opt_in && !entry.enabled)
      return t("tts.providers.blocked.notEnabled");
    if (entry.license_needs_ack && !entry.license_acked)
      return t("tts.providers.blocked.needsLicense");
    if (entry.remote && !entry.allow_remote)
      return t("tts.providers.blocked.needsRemote");
    return entry.blocked.message;
  })();

  return (
    <div
      className="px-4 py-3"
      data-testid={`tts-provider-${id}`}
      data-selected={entry.selected ? "true" : "false"}
    >
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={onToggleExpanded}
          aria-expanded={expanded}
          aria-controls={`tts-provider-details-${id}`}
          aria-label={t(
            expanded
              ? "tts.providers.details.hide"
              : "tts.providers.details.show",
            { name: descriptor.label },
          )}
          data-testid={`tts-provider-toggle-${id}`}
          className="flex items-center gap-1 text-sm font-medium text-text hover:underline"
        >
          {expanded ? (
            <ChevronDown className="h-4 w-4" aria-hidden="true" />
          ) : (
            <ChevronRight className="h-4 w-4" aria-hidden="true" />
          )}
          {descriptor.label}
        </button>
        <span data-testid={`tts-provider-tier-${id}`}>
          <Badge variant={tierVariant}>
            {t(`tts.providers.tier.${descriptor.tier}`)}
          </Badge>
        </span>
        {entry.selected && (
          <span data-testid={`tts-provider-selected-${id}`}>
            <Badge variant="primary">{t("tts.providers.selected")}</Badge>
          </span>
        )}
        <span data-testid={`tts-provider-health-${id}`}>
          <Badge variant={health.available ? "success" : "secondary"}>
            {health.available
              ? t("tts.providers.availability.available")
              : t("tts.providers.availability.unavailable")}
          </Badge>
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            data-testid={`tts-provider-recheck-${id}`}
            onClick={onRecheck}
          >
            {t("tts.providers.recheck")}
          </Button>
          {hasEnableSwitch(entry) && (
            <Switch
              checked={entry.enabled}
              disabled={busy}
              label={t("tts.providers.enable")}
              labelPosition="before"
              data-testid={`tts-provider-enable-${id}`}
              onChange={(_e, data) => onEnable(data.checked)}
            />
          )}
        </div>
      </div>

      {!health.available && (health.reason || health.hint) && (
        <p
          className="mt-1 text-xs text-text/80"
          data-testid={`tts-provider-reason-${id}`}
        >
          {health.reason}
          {health.reason && health.hint ? " " : ""}
          {health.hint && <span className="text-text/60">{health.hint}</span>}
        </p>
      )}
      {health.available && health.hint && (
        <p className="mt-1 text-xs text-text/60">{health.hint}</p>
      )}

      <dl className="mt-2 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1 text-xs">
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">{t("tts.providers.facts.device")}</dt>
          <dd data-testid={`tts-provider-device-${id}`}>
            {health.device
              ? deviceName(health.device)
              : caps.devices.length > 0
                ? caps.devices.map(deviceName).join(", ")
                : t("tts.providers.unknown")}
          </dd>
        </div>
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">{t("tts.providers.facts.version")}</dt>
          <dd>{health.version ?? t("tts.providers.unknown")}</dd>
        </div>
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">
            {t("tts.providers.facts.streaming")}
          </dt>
          <dd>{t(`tts.providers.streaming.${caps.streaming}`)}</dd>
        </div>
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">
            {t("tts.providers.facts.languages")}
          </dt>
          <dd>
            {caps.languages.length > 0
              ? caps.languages.map(languageName).join(", ")
              : t("tts.providers.unknown")}
          </dd>
        </div>
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">{t("tts.providers.facts.cloning")}</dt>
          <dd data-testid={`tts-provider-cloning-${id}`}>
            {caps.cloning ? t("tts.providers.yes") : t("tts.providers.no")}
          </dd>
        </div>
        <div className={FACT_ROW}>
          <dt className="text-mid-gray">{t("tts.providers.facts.license")}</dt>
          <dd
            className="space-y-0.5"
            data-testid={`tts-provider-license-${id}`}
          >
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span>
                {t("tts.providers.license.code")}: {license.code || "–"} ·{" "}
                {t("tts.providers.license.weights")}: {license.weights || "–"}
              </span>
              {license.non_commercial && (
                <span data-testid={`tts-provider-noncommercial-${id}`}>
                  <Badge variant="warning" className="!px-2 !py-0">
                    {t("tts.providers.license.nonCommercial")}
                  </Badge>
                </span>
              )}
            </div>
            <div data-testid={`tts-provider-verified-${id}`}>
              <VerifiedLine license={license} />
            </div>
            {license.note && <div className="text-text/60">{license.note}</div>}
          </dd>
        </div>
      </dl>

      {missing && (
        <div
          role="status"
          className="mt-2 flex flex-wrap items-center gap-2 rounded bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-300"
          data-testid={`tts-provider-blocked-${id}`}
          data-kind={entry.blocked?.kind}
        >
          <span>
            <span className="font-medium">
              {t("tts.providers.blocked.title")}
            </span>{" "}
            {missing}
          </span>
          {step === "license" && (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={busy}
              data-testid={`tts-provider-open-license-${id}`}
              onClick={() => onApprove("license")}
            >
              {t("tts.providers.blocked.confirmLicense")}
            </Button>
          )}
          {step === "remote" && (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={busy}
              data-testid={`tts-provider-open-remote-${id}`}
              onClick={() => onApprove("remote")}
            >
              {t("tts.providers.blocked.allowRemote")}
            </Button>
          )}
        </div>
      )}

      {entry.remote && entry.allow_remote && (
        <div
          className="mt-2 flex flex-wrap items-center gap-2 text-xs text-text/80"
          data-testid={`tts-provider-remote-allowed-${id}`}
        >
          <span>{t("tts.providers.remote.allowed")}</span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            data-testid={`tts-provider-revoke-remote-${id}`}
            onClick={onRevokeRemote}
          >
            {t("tts.providers.remote.revoke")}
          </Button>
        </div>
      )}

      {expanded && <div id={`tts-provider-details-${id}`}>{children}</div>}
    </div>
  );
};
